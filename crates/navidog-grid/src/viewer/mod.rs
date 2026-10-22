//! Table viewer: paging, row count, WHERE / ORDER BY text, editing, commits and transactions
//! (21-table-viewer.md §3–6).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{
    Editor, EditorState, Escape, Input, InputEvent, InputState, Textarea, TextareaState,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Image, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, WeakEntity, Window, actions, div, img, px,
};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent, SharedSession, run_on};
use navidog_db::{CancelHandle, ColumnInfo, ResultSet};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{FormRow, confirm_with, truncated};
use navidog_ui::{palette, tabs, tokens};
use navidog_util::format::{decode_hex, encode_hex, pretty_json};
use navidog_util::sql_type::{Choices, choices};
use rust_i18n::t;

use crate::commit::{Commit, apply, close_session};
use crate::editor::{self, Kind};
use crate::filter::{self, Filter, Rule, Sort};
use crate::grid::{GridEvent, GridTarget, HideColumn, ResultGrid, ToggleRawData, ToggleRowId};
use crate::model::{Cell, GridColumn};

mod cell_editor;
mod filter_pane;
mod find;
mod fk_picker;
mod form;

const PANEL: &str = "TableViewer";
const CONTEXT: &str = "TableViewer";
/// Half a turn of the refresh icon. `Spinner`'s cycle is 0.8s and not configurable.
const REFRESH_SPIN: Duration = Duration::from_millis(400);
actions!(
    viewer,
    [
        ApplyChanges,
        ShowFind,
        FindNext,
        ShowReplace,
        ToggleFilter,
        BeginTransaction,
        CommitTransaction,
        RollbackTransaction,
        DesignTable,
        QueryTable,
        ApplyFilterSort,
        ClearFilterSort,
        RefreshTable,
        StopLoad,
        SortAscending,
        SortDescending,
        SortNone,
        FindData,
        FindReplaceMode,
        FindColumnsMode,
        ToggleMatchCase,
        ToggleHighlight,
        ToggleIncremental,
    ]
);

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = viewer, no_json)]
struct PickField {
    index: usize,
    name: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = viewer, no_json)]
struct PickOp {
    index: usize,
    op: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = viewer, no_json)]
struct PickSort {
    index: usize,
    name: SharedString,
}

struct Draft {
    field: String,
    op: filter::Op,
    value: Entity<InputState>,
    value2: Entity<InputState>,
    or_join: bool,
    negate: bool,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", ApplyChanges, Some(CONTEXT)),
        KeyBinding::new("cmd-f", ShowFind, Some(CONTEXT)),
        KeyBinding::new("cmd-g", FindNext, Some(CONTEXT)),
        KeyBinding::new("alt-cmd-f", ShowReplace, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-f", ToggleFilter, Some(CONTEXT)),
        KeyBinding::new("cmd-r", RefreshTable, Some(CONTEXT)),
        KeyBinding::new("cmd-.", StopLoad, Some(CONTEXT)),
        KeyBinding::new("ctrl-r", ApplyFilterSort, Some(CONTEXT)),
        KeyBinding::new("shift-cmd-y", QueryTable, Some(CONTEXT)),
        KeyBinding::new("cmd-d", DesignTable, Some(CONTEXT)),
    ]);
    register_panel(cx, PANEL, |ctx, window, cx| {
        let object = match ctx.info() {
            PanelInfo::Panel(v) => serde_json::from_value::<ObjectRef>(v.clone()).ok(),
            _ => None,
        };
        let object = object.unwrap_or_else(|| ObjectRef {
            scope: Scope::new("", "", None),
            kind: ObjectKind::Table,
            name: String::new(),
        });
        panel_handle(TableViewer::new(object, window, cx))
    });
}

pub struct TableViewer {
    focus_handle: FocusHandle,
    object: ObjectRef,
    grid: Entity<ResultGrid>,
    filter: Entity<InputState>,
    order: Entity<InputState>,
    limit_input: Entity<InputState>,
    show_filter: bool,
    /// Visual builder. Text mode keeps editing `filter` and `order` directly.
    builder: bool,
    rules: Vec<Draft>,
    sorts: Vec<Sort>,
    grouped: bool,
    group_negate: bool,
    /// Cell editor under the grid. `None` hides it (F-DATA-011).
    editor: Option<Kind>,
    /// Dragged cell-editor height in pixels. `None` uses the default until the user drags.
    cell_editor_px: Option<f32>,
    editor_input: Entity<TextareaState>,
    /// JSON tab. Separate from `editor_input` so it can use the JSON highlighter.
    json_input: Entity<EditorState>,
    /// Row and column the editor text was loaded from.
    editor_cell: Option<(usize, usize)>,
    editor_note: SharedString,
    /// Preview of a loaded image. Binary files are not written into the cell.
    image: Option<Arc<Image>>,
    find_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    show_find: bool,
    replace_mode: bool,
    find_columns: bool,
    find_case: bool,
    find_highlight: bool,
    find_incremental: bool,
    limit: Option<u64>,
    page: u64,
    total: Option<u64>,
    session: Option<SharedSession>,
    cancel: Option<CancelHandle>,
    dialect: Option<Dialect>,
    columns: Vec<ColumnInfo>,
    foreign_keys: Vec<navidog_db::ForeignKey>,
    hidden: HashSet<String>,
    /// Restored when a commit started by hiding a column fails, so the grid and the set still match
    hidden_revert: Option<HashSet<String>>,
    /// Insert a row once the commit that made room for it succeeds
    add_after_commit: bool,
    /// Load the page once the commit started by paging, sorting or filtering succeeds
    reload_after_commit: bool,
    /// The user answered the close prompt; the tab may close now (pending edits are committed or dropped)
    close_approved: bool,
    /// Close the tab once the commit chosen in the close prompt succeeds
    close_after_commit: bool,
    /// Window this tab was last added to. A popped-out tab updates it in `on_added_to`.
    home: Option<gpui_kit::AnyWindowHandle>,
    form: bool,
    /// Cell text is submitted as SQL (F-DATA-010)
    raw_data: bool,
    /// SQLite only: SELECT includes rowid as a read-only column
    show_rowid: bool,
    form_inputs: Vec<Entity<InputState>>,
    form_for: Option<usize>,
    in_transaction: bool,
    busy: bool,
    /// Refresh icon keeps turning until this instant, even if the load finished sooner.
    spin_until: Option<Instant>,
    /// The most recently executed SQL or error
    status: SharedString,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for TableViewer {}

impl Focusable for TableViewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for TableViewer {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(serde_json::to_value(&self.object).unwrap_or_default()),
        }
    }

    /// With uncommitted changes or an open transaction the tab refuses to close; the tab close
    /// commands run `ask_to_close` instead (see `tabs::CloseGuard`).
    fn closable(&self, cx: &App) -> bool {
        !self.needs_close_prompt(cx)
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = PanelId::from(cx.entity_id());
        tabs::added(panel, group, cx);
        // Runs again when the tab moves to another window, so the prompt follows it
        self.home = Some(window.window_handle());
        let (viewer, home) = (cx.entity().downgrade(), window.window_handle());
        let guard: tabs::CloseGuard = std::rc::Rc::new(move |cx| {
            let viewer = viewer.clone();
            let _ = home.update(cx, |_, window, cx| {
                let _ = viewer.update(cx, |v, cx| v.ask_to_close(window, cx));
            });
        });
        tabs::set_close_guard(panel, guard, cx);
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        let panel = PanelId::from(cx.entity_id());
        tabs::set_active(panel, active, cx);
        tabs::set_surface(
            panel,
            active,
            tabs::MenuSurface::Table,
            tabs::TableMenu {
                in_transaction: self.in_transaction,
                filter_open: self.show_filter,
            },
            cx,
        );
        let workspace = Workspace::global(cx);
        if active {
            // The info pane and the navigator follow the tab, not the last tree click.
            let object = self.object.clone();
            workspace.update(cx, |w, cx| w.select(Selection::Object(object), cx));
        } else {
            workspace.update(cx, |w, cx| w.set_focused_column(None, cx));
        }
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if tabs::removed(PanelId::from(cx.entity_id()), cx) {
            return;
        }
        let Some(session) = self.session.take() else {
            return;
        };
        // Closing the session rolls back an open transaction, which is what "discard" asked for
        let dirty = self.grid.read(cx).model(cx).has_changes();
        if self.close_approved || (!dirty && !self.in_transaction) {
            close_session(session);
            return;
        }
        // Only reached when something closed the tab without asking first (the dock refusing
        // via `closable` is the normal path): ask now, with the tab already gone.
        let commit = self.commit_statements(cx);
        let in_tx = self.in_transaction;
        let (ok_session, cancel_session) = (session.clone(), session);
        let name = self.object.name.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (ok_session, cancel_session, commit) =
                (ok_session.clone(), cancel_session.clone(), commit.clone());
            let name = name.clone();
            alert
                .icon(lucide("info"))
                .title(t!("viewer.close_pending", name = name).to_string())
                .show_cancel(true)
                .ok_text(t!("viewer.commit").to_string())
                .cancel_text(t!("viewer.discard").to_string())
                // The tab is already gone: Esc must not quietly pick "discard"
                .keyboard(false)
                .on_ok(move |_, window, cx| {
                    let (session, commit) = (ok_session.clone(), commit.clone());
                    let job = run_on(session.clone(), move |rel| {
                        Box::pin(apply(rel, commit, in_tx, true))
                    });
                    let name = name.clone();
                    window
                        .spawn(cx, async move |cx| {
                            let result = job
                                .await
                                .map_err(|e| e.to_string())
                                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
                            close_session(session);
                            let Err(e) = result else { return };
                            tracing::warn!("commit on tab close failed: {e}");
                            let _ = cx.update(|window, cx| {
                                window.open_alert_dialog(cx, move |alert, _, _| {
                                    alert
                                        .icon(lucide("circle-x"))
                                        .title(
                                            t!("viewer.close_commit_failed", name = name)
                                                .to_string(),
                                        )
                                        .description(e.clone())
                                });
                            });
                        })
                        .detach();
                    true
                })
                .on_cancel(move |_, _, _| {
                    let session = cancel_session.clone();
                    navidog_db::runtime::spawn(async move {
                        if in_tx {
                            let _ = run_on(session.clone(), |rel| {
                                Box::pin(async move { rel.execute("ROLLBACK").await })
                            })
                            .await;
                        }
                        close_session(session);
                    });
                    true
                })
        });
    }
}

impl Panel for TableViewer {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = navidog_core::store::ConnectionStore::global(cx);
        let (color, connection) = match store.read(cx).get(&self.object.scope.connection) {
            Some(c) => (c.color.clone(), Some(c.name.clone())),
            None => (None, None),
        };
        let pending = self.needs_close_prompt(cx);
        let label = tabs::object_label(
            &self.object.name,
            Some(&self.object.scope.database),
            connection.as_deref(),
        );
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("table")),
            label,
            color.and_then(|c| tokens::connection_color(&c, cx)),
            pending,
        )
    }
}

impl TableViewer {
    fn needs_close_prompt(&self, cx: &App) -> bool {
        !self.close_approved
            && self.session.is_some()
            && (self.in_transaction || self.grid.read(cx).model(cx).has_changes())
    }

    /// The tab close commands land here while there are uncommitted changes: commit them and
    /// close, drop them and close, or keep the tab. A failed commit keeps the tab and the changes.
    fn ask_to_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.grid.update(cx, |g, cx| g.commit_edit(window, cx));
        self.flush_form(cx);
        if !self.needs_close_prompt(cx) {
            self.close_now(window, cx);
            return;
        }
        let viewer = cx.entity();
        let name = self.object.name.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (commit, discard) = (viewer.clone(), viewer.clone());
            let enter = viewer.clone();
            alert
                .icon(lucide("info"))
                .title(t!("viewer.close_pending", name = name).to_string())
                // Enter commits, like the primary button; Esc keeps the tab
                .on_ok(move |_, window, cx| {
                    enter.update(cx, |v, cx| v.commit_and_close(window, cx));
                    true
                })
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-keep")
                                .label(t!("grid.cancel").to_string())
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("close-discard")
                                .label(t!("viewer.discard").to_string())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    discard.update(cx, |v, cx| {
                                        v.grid.update(cx, |g, cx| {
                                            g.update_model(cx, |m| m.discard())
                                        });
                                        v.close_now(window, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("close-commit")
                                .primary()
                                .label(t!("viewer.commit").to_string())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    commit.update(cx, |v, cx| v.commit_and_close(window, cx));
                                }),
                        ),
                )
        });
    }

    fn commit_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_after_commit = true;
        self.apply_changes(true, window, cx);
    }

    /// Sends a close-prompt commit that was skipped because a load still held `busy`.
    /// The commit completion path closes the tab itself and must not call this.
    fn resume_close(viewer: WeakEntity<Self>, cx: &mut App) {
        let Ok(Some(home)) = viewer.update(cx, |this, _| this.home) else {
            return;
        };
        let _ = home.update(cx, |_, window, cx| {
            let _ = viewer.update(cx, |this, cx| {
                if this.close_after_commit && !this.busy {
                    this.apply_changes(true, window, cx);
                }
            });
        });
    }

    /// Closes this tab past its own prompt.
    fn close_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_approved = true;
        let close = tabs::CloseTabs {
            panel: Some(PanelId::from(cx.entity_id())),
            which: tabs::Close::This,
        };
        window.dispatch_action(Box::new(close), cx);
    }
}

fn first_cell(sets: &[ResultSet]) -> Option<u64> {
    sets.first()?
        .rows
        .first()?
        .first()?
        .as_deref()?
        .parse()
        .ok()
}

impl TableViewer {
    pub fn new(object: ObjectRef, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let grid = cx.new(|cx| ResultGrid::new(window, cx));
            let filter = cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("viewer.where_hint").to_string())
            });
            let order = cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("viewer.order_hint").to_string())
            });
            let limit_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(Settings::global(cx).records_per_page.to_string())
            });
            let find_input = cx.new(|cx| InputState::new(window, cx));
            let replace_input = cx.new(|cx| InputState::new(window, cx));
            let editor_input = cx.new(|cx| TextareaState::new(window, cx));
            let json_input = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    .line_number(false)
            });
            let apply_on_enter = |this: &mut Self,
                                  _: &Entity<InputState>,
                                  e: &InputEvent,
                                  window: &mut Window,
                                  cx: &mut Context<Self>| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.apply_filter(window, cx);
                }
            };
            let subs = vec![
                cx.subscribe_in(
                    &find_input,
                    window,
                    |this, _, e: &InputEvent, _, cx| match e {
                        InputEvent::PressEnter { .. } => this.find_step(true, cx),
                        InputEvent::Change if this.find_incremental => this.find_step(false, cx),
                        InputEvent::Change => this.sync_highlight(cx),
                        _ => {}
                    },
                ),
                cx.subscribe_in(&replace_input, window, |this, _, e: &InputEvent, _, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.replace_current(cx);
                    }
                }),
                cx.subscribe_in(&filter, window, apply_on_enter),
                cx.subscribe_in(&order, window, apply_on_enter),
                cx.subscribe_in(
                    &limit_input,
                    window,
                    |this, input, e: &InputEvent, window, cx| {
                        if matches!(e, InputEvent::PressEnter { .. }) {
                            this.limit = input
                                .read(cx)
                                .value()
                                .trim()
                                .parse()
                                .ok()
                                .filter(|n| *n > 0)
                                .or(this.limit);
                            this.page = 0;
                            this.reload_committed(false, window, cx);
                        }
                    },
                ),
                cx.subscribe_in(
                    &grid,
                    window,
                    |this, _, e: &GridEvent, window, cx| match e {
                        GridEvent::SortChanged(sort) => {
                            if this.builder {
                                this.sorts = sort
                                    .as_ref()
                                    .map(|(col, asc)| Sort {
                                        field: col.to_string(),
                                        asc: *asc,
                                    })
                                    .into_iter()
                                    .collect();
                                this.write_filter_sql(window, cx);
                            } else {
                                let text = sort
                                    .as_ref()
                                    .zip(this.dialect)
                                    .map(|((col, asc), d)| {
                                        format!(
                                            "{} {}",
                                            d.quote_ident(col),
                                            if *asc { "ASC" } else { "DESC" }
                                        )
                                    })
                                    .unwrap_or_default();
                                this.order.update(cx, |s, cx| s.set_value(text, window, cx));
                            }
                            this.page = 0;
                            this.reload_committed(false, window, cx);
                        }
                        GridEvent::PickForeignKey => this.open_fk_picker(window, cx),
                        GridEvent::QuickFilter { column, op, value } => {
                            if let Some(op) = filter::Op::parse_key(op) {
                                this.quick_filter(column, op, value, window, cx);
                            }
                        }
                        GridEvent::Changed => cx.notify(),
                        // Leaving a modified row commits it (F-DATA-003)
                        GridEvent::CommitRequested => {
                            if !this.busy {
                                this.apply_changes(false, window, cx);
                            }
                        }
                        GridEvent::ColumnFocused(name) => {
                            let name = name.to_string();
                            Workspace::global(cx)
                                .update(cx, |w, cx| w.set_focused_column(Some(name), cx));
                            this.sync_editor(window, cx);
                        }
                    },
                ),
                cx.subscribe(
                    &Sessions::global(cx),
                    |this, _, e: &SessionsEvent, cx| match e {
                        SessionsEvent::Opened(id)
                            if *id == this.object.scope.connection && this.session.is_none() =>
                        {
                            this.connect(cx)
                        }
                        SessionsEvent::Closed(id) | SessionsEvent::Reopening(id)
                            if *id == this.object.scope.connection =>
                        {
                            if let Some(s) = this.session.take() {
                                close_session(s);
                            }
                            this.set_in_transaction(false, cx);
                            this.status = t!("viewer.closed").to_string().into();
                            cx.notify();
                        }
                        _ => {}
                    },
                ),
            ];
            let mut this = Self {
                focus_handle: cx.focus_handle(),
                object,
                grid,
                filter,
                order,
                limit_input,
                find_input,
                replace_input,
                editor_input,
                json_input,
                show_filter: false,
                builder: false,
                rules: Vec::new(),
                sorts: Vec::new(),
                grouped: false,
                group_negate: false,
                editor: None,
                cell_editor_px: None,
                editor_cell: None,
                editor_note: SharedString::default(),
                image: None,
                show_find: false,
                replace_mode: false,
                find_columns: false,
                find_case: false,
                find_highlight: true,
                find_incremental: false,
                limit: {
                    let s = Settings::global(cx);
                    s.limit_records.then_some(s.records_per_page.max(1))
                },
                page: 0,
                total: None,
                session: None,
                cancel: None,
                dialect: None,
                columns: Vec::new(),
                foreign_keys: Vec::new(),
                hidden: HashSet::new(),
                hidden_revert: None,
                add_after_commit: false,
                reload_after_commit: false,
                close_approved: false,
                close_after_commit: false,
                home: None,
                form: false,
                raw_data: false,
                show_rowid: false,
                form_inputs: Vec::new(),
                form_for: None,
                in_transaction: false,
                busy: false,
                spin_until: None,
                status: SharedString::default(),
                task: None,
                _subscriptions: subs,
            };
            this.connect(cx);
            this
        })
    }

    /// Opens a separate session for the viewer, fetches the field info, then loads the first page.
    fn connect(&mut self, cx: &mut Context<Self>) {
        let sessions = Sessions::global(cx).read(cx);
        if !matches!(
            sessions.state(&self.object.scope.connection),
            ConnState::Open { .. }
        ) {
            return;
        }
        let Some(job) = sessions.new_session(
            &self.object.scope.connection,
            Some(&self.object.scope.database),
        ) else {
            return;
        };
        let (scope, table) = (self.object.scope.clone(), self.object.name.clone());
        self.set_busy(true, cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            let session = match job.await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    return this
                        .update(cx, |t, cx| t.fail(crate::db_text(&e), cx))
                        .unwrap_or(());
                }
                Err(e) => {
                    return this
                        .update(cx, |t, cx| t.fail(e.to_string(), cx))
                        .unwrap_or(());
                }
            };
            let cancel = session.cancel_handle();
            let shared: SharedSession = std::sync::Arc::new(tokio::sync::Mutex::new(Some(session)));
            let columns = run_on(shared.clone(), move |rel| {
                Box::pin(async move {
                    let d = rel.dialect();
                    Ok((d, rel.table_columns(&scope, &table).await?))
                })
            })
            .await;
            let pending_close = this
                .update(cx, |this, cx| {
                    this.apply_connected(columns, shared, cancel, cx)
                })
                .unwrap_or(false);
            if pending_close {
                cx.update(|cx| TableViewer::resume_close(this.clone(), cx));
            }
        }));
    }

    /// Catalog columns and the viewer's own session, then the first page. Returns whether a
    /// close that waited on this load should continue.
    fn apply_connected(
        &mut self,
        columns: Result<navidog_db::DbResult<(Dialect, Vec<ColumnInfo>)>, tokio::task::JoinError>,
        shared: SharedSession,
        cancel: Option<CancelHandle>,
        cx: &mut Context<Self>,
    ) -> bool {
        match columns {
            Ok(Ok((dialect, columns))) => {
                self.dialect = Some(dialect);
                self.columns = columns;
                self.grid.update(cx, |g, cx| {
                    g.set_target(
                        Some(GridTarget {
                            dialect,
                            scope: self.object.scope.clone(),
                            table: self.object.name.clone(),
                        }),
                        cx,
                    )
                });
            }
            Ok(Err(e)) => self.status = crate::db_text(&e).into(),
            Err(e) => self.status = e.to_string().into(),
        }
        self.session = Some(shared);
        self.cancel = cancel;
        self.load_foreign_keys(cx);
        // Back after a reopen with edits pending: keep the rows they belong to; they
        // commit on the new session. A reload would discard them.
        if self.grid.read(cx).model(cx).has_changes() {
            self.set_busy(false, cx);
            self.status = SharedString::default();
        } else {
            self.reload(true, cx);
        }
        if Settings::global(cx).auto_begin_transaction {
            self.begin_transaction(cx);
        }
        self.close_after_commit && !self.busy
    }

    fn load_foreign_keys(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let (scope, table) = (self.object.scope.clone(), self.object.name.clone());
        let job = run_on(session, move |rel| {
            Box::pin(async move { rel.foreign_keys(&scope, &table).await })
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let Ok(Ok(keys)) = result else {
                    return;
                };
                let names = keys.iter().map(|key| key.column.clone()).collect();
                this.foreign_keys = keys;
                this.grid
                    .update(cx, |grid, cx| grid.set_foreign_keys(names, cx));
            });
        })
        .detach();
    }

    fn write_cell(&mut self, row: usize, col: usize, value: Cell, cx: &mut Context<Self>) {
        self.grid.update(cx, |grid, cx| {
            grid.update_model(cx, |model| model.set_value(row, col, value));
        });
    }

    /// `busy` also locks the grid: an edit made while a commit or a load runs would be lost
    /// when it finishes (see `GridModel::locked`).
    fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        self.grid.update(cx, |g, cx| g.set_locked(busy, cx));
        if busy {
            self.spin_until = Some(Instant::now() + REFRESH_SPIN);
            return;
        }
        let Some(until) = self.spin_until else {
            return;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.spin_until = None;
            return;
        }
        // The load can finish before half a turn; wake the view when that half ends.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(left).await;
            let _ = this.update(cx, |this, cx| {
                if this.busy || this.spin_until.is_some_and(|t| Instant::now() < t) {
                    return;
                }
                this.spin_until = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn spinning(&self) -> bool {
        refresh_spinning(self.busy, self.spin_until, Instant::now())
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.set_busy(false, cx);
        self.status = message.into();
        cx.notify();
    }

    /// Loads the current page; with `count`, also queries the total row count (when the filter changes).
    fn reload(&mut self, count: bool, cx: &mut Context<Self>) {
        let (Some(session), Some(d)) = (self.session.clone(), self.dialect) else {
            return;
        };
        let (scope, table) = (&self.object.scope, &self.object.name);
        let filter = self.filter.read(cx).value().to_string();
        let order = self.order.read(cx).value().to_string();
        let offset = self.page * self.limit.unwrap_or(0);
        let sql = d.select_page(
            scope,
            table,
            &filter,
            &order,
            self.limit,
            offset,
            self.show_rowid,
        );
        let count_sql = count.then(|| d.count(scope, table, &filter));
        self.status = sql.clone().into();
        self.set_busy(true, cx);
        cx.notify();
        let page = run_on(session.clone(), move |rel| {
            Box::pin(async move { rel.execute(&sql).await })
        });
        let total = count_sql.map(|sql| {
            run_on(session, move |rel| {
                Box::pin(async move { rel.execute(&sql).await })
            })
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = page
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let pending_close = this
                .update(cx, |this, cx| this.apply_page(result, cx))
                .unwrap_or(false);
            if pending_close {
                cx.update(|cx| TableViewer::resume_close(this.clone(), cx));
                return;
            }
            if let Some(total) = total {
                let n = total
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .and_then(|sets| first_cell(&sets));
                let _ = this.update(cx, |this, cx| {
                    this.total = n;
                    cx.notify();
                });
            }
        }));
    }

    /// Projects one loaded page onto the grid. Returns whether a close that waited on this load
    /// should continue.
    fn apply_page(
        &mut self,
        result: Result<Vec<ResultSet>, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.set_busy(false, cx);
        match result {
            Ok(mut sets) if !sets.is_empty() => {
                let set = sets.swap_remove(0);
                let pk = |name: &str| self.columns.iter().any(|c| c.name == name && c.primary_key);
                let columns: Vec<GridColumn> = set
                    .columns
                    .iter()
                    .map(|c| {
                        let info = self.columns.iter().find(|i| i.name == c.name);
                        let type_name = info
                            .map(|i| i.type_name.clone())
                            .filter(|t| !t.is_empty())
                            .unwrap_or_else(|| c.type_name.clone());
                        let read_only = c.name == "_rowid";
                        GridColumn {
                            name: if read_only {
                                "ROWID".into()
                            } else {
                                c.name.clone().into()
                            },
                            type_name: type_name.into(),
                            primary_key: pk(&c.name),
                            comment: info.and_then(|i| i.comment.clone()).map(Into::into),
                            read_only,
                        }
                    })
                    .collect();
                let rows: Vec<Vec<Cell>> = set
                    .rows
                    .into_iter()
                    .map(|r| r.into_iter().map(|v| v.map(Into::into)).collect())
                    .collect();
                self.grid.update(cx, |g, cx| g.set_data(columns, rows, cx));
                self.reproject(cx);
                if std::mem::take(&mut self.add_after_commit) {
                    self.insert_row(cx);
                }
            }
            Ok(_) => {}
            Err(e) => {
                self.add_after_commit = false;
                self.status = e.into();
            }
        }
        cx.notify();
        self.close_after_commit && !self.busy
    }

    /// Reloads after the page, sort, filter or limit changed. Like leaving a modified row
    /// (F-DATA-003), pending edits are committed first and the page loads once that succeeds; a
    /// failed commit keeps the edits and the rows they belong to.
    fn reload_committed(&mut self, count: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.grid.update(cx, |g, cx| g.commit_edit(window, cx));
        if !self.grid.read(cx).model(cx).has_changes() {
            self.reload(count, cx);
            return;
        }
        // A commit is already running; its success reloads with the new state
        if self.busy {
            return;
        }
        self.reload_after_commit = true;
        self.apply_changes(false, window, cx);
    }

    fn commit_statements(&self, cx: &App) -> Commit {
        let d = self.dialect.unwrap_or(Dialect::MySql);
        let m = self.grid.read(cx).model(cx);
        let statements = d.render_changes(
            &self.object.scope,
            &self.object.name,
            &m.change_set(),
            m.has_primary_key(),
        );
        Commit::new(d, statements)
    }

    /// Commits the grid changes; with `end_transaction`, also commits the open transaction. On failure
    /// the database error is shown and the changes are kept.
    fn apply_changes(
        &mut self,
        end_transaction: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        // Statements already in flight must not be sent again. A close-prompt commit keeps
        // `close_after_commit`; the load that holds `busy` resumes it when it finishes.
        if self.busy {
            return;
        }
        // The cell being typed in belongs to this commit; once the grid locks it could not be written
        self.grid.update(cx, |g, cx| g.commit_edit(window, cx));
        self.flush_form(cx);
        let commit = self.commit_statements(cx);
        if commit.statements.is_empty() && !end_transaction {
            // Nothing to send. A hide still has to reproject; an empty new row stays as it is,
            // unless the page is being left.
            self.add_after_commit = false;
            if std::mem::take(&mut self.reload_after_commit) {
                self.hidden_revert = None;
                self.reload(true, cx);
            } else if let Some(old) = self.hidden_revert.take()
                && !self.reproject(cx)
            {
                self.hidden = old;
            }
            return;
        }
        let in_tx = self.in_transaction;
        self.status = commit.statements.join(";\n").into();
        self.set_busy(true, cx);
        let job = run_on(session, move |rel| {
            Box::pin(apply(rel, commit, in_tx, end_transaction))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_busy(false, cx);
                match result {
                    Ok(()) => {
                        if end_transaction {
                            this.set_in_transaction(false, cx);
                        }
                        // Committed from the close prompt: nothing to reload, the tab goes
                        if std::mem::take(&mut this.close_after_commit) {
                            this.grid.update(cx, |g, cx| {
                                g.update_model(cx, |m| m.accept_committed());
                            });
                            this.close_now(window, cx);
                            return;
                        }
                        // Hiding a column while dirty still reloads so the projection matches.
                        let reproject = this.hidden_revert.take().is_some();
                        let leaving = std::mem::take(&mut this.reload_after_commit);
                        let sync = !cx.has_global::<Settings>() || Settings::global(cx).sync_record;
                        // Without sync, rows whose stored values only the server knows still reload
                        let unknown = this.grid.read(cx).model(cx).commit_needs_reload();
                        if sync || reproject || leaving || unknown {
                            this.reload(true, cx);
                        } else {
                            this.grid.update(cx, |g, cx| {
                                g.update_model(cx, |m| m.accept_committed());
                            });
                            this.form_for = None;
                            if std::mem::take(&mut this.add_after_commit) {
                                this.insert_row(cx);
                            }
                        }
                    }
                    Err(e) => {
                        this.add_after_commit = false;
                        this.reload_after_commit = false;
                        this.close_after_commit = false;
                        if let Some(old) = this.hidden_revert.take() {
                            this.hidden = old;
                        }
                        this.status = e.clone().into();
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            alert
                                .icon(lucide("circle-x"))
                                .title(t!("viewer.commit_failed").to_string())
                                .description(e.clone())
                        });
                    }
                }
                cx.notify();
            });
        }));
    }

    fn run_simple(
        &mut self,
        sql: &'static str,
        after: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let job = run_on(session, move |rel| {
            Box::pin(async move { rel.execute(sql).await })
        });
        // Not in `self.task`: replacing that drops the page load in flight (opening a table with
        // "auto begin transaction" sends BEGIN right after the first SELECT)
        cx.spawn(async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => after(this, cx),
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn note_menu(&self, cx: &mut Context<Self>) {
        tabs::set_table_menu(
            PanelId::from(cx.entity_id()),
            tabs::TableMenu {
                in_transaction: self.in_transaction,
                filter_open: self.show_filter,
            },
            cx,
        );
    }

    fn set_in_transaction(&mut self, on: bool, cx: &mut Context<Self>) {
        self.in_transaction = on;
        self.note_menu(cx);
    }

    fn toggle_filter_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_filter = !self.show_filter;
        if self.show_filter {
            self.focus_filter_input(window, cx);
        } else if self.focus_handle.contains_focused(window, cx) {
            // The focused field is in this pane and unmounts with it. Leave focus on the
            // grid, or the shortcut has nowhere to land and cannot open the pane again.
            self.focus_grid(window, cx);
        }
        self.note_menu(cx);
        cx.notify();
    }

    /// Where keyboard input goes once a pane that held it has closed.
    fn focus_grid(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form {
            window.focus(&self.focus_handle, cx);
        } else {
            let handle = self.grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// Text mode focuses the WHERE field. The builder has no WHERE field, so the first
    /// condition's value takes its place; an empty builder leaves focus where it was.
    fn focus_filter_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.builder {
            if let Some(rule) = self.rules.first() {
                rule.value.update(cx, |s, cx| s.focus(window, cx));
            }
        } else {
            self.filter.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    /// "Query Table": `SELECT * FROM` this table, in a new query, then run.
    fn query_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
        let sql = format!(
            "SELECT * FROM {}",
            dialect.qualified(&self.object.scope, &self.object.name)
        );
        window.dispatch_action(
            Box::new(navidog_core::actions::NewQueryText {
                connection: self.object.scope.connection.clone().into(),
                database: self.object.scope.database.clone().into(),
                schema: self.object.scope.schema.clone().map(Into::into),
                sql: sql.into(),
            }),
            cx,
        );
    }

    fn sort_focused(&mut self, dir: Option<bool>, cx: &mut Context<Self>) {
        let col = self.grid.read(cx).model(cx).head().1;
        if self.grid.read(cx).model(cx).columns.get(col).is_none() {
            return;
        }
        self.grid.update(cx, |g, cx| {
            g.request_sort(dir.map(|_| col), dir.unwrap_or(true), cx)
        });
    }

    fn clear_filter_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rules.clear();
        self.sorts.clear();
        self.grouped = false;
        self.group_negate = false;
        self.filter
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.order
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.page = 0;
        self.reload_committed(true, window, cx);
    }

    fn begin_transaction(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.dialect else { return };
        self.run_simple(
            d.begin_transaction(),
            |this, cx| this.set_in_transaction(true, cx),
            cx,
        );
    }

    fn rollback(&mut self, cx: &mut Context<Self>) {
        self.run_simple(
            "ROLLBACK",
            |this, cx| {
                this.set_in_transaction(false, cx);
                this.reload(true, cx);
            },
            cx,
        );
    }

    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.clone() {
            navidog_db::runtime::spawn(async move {
                if let Err(e) = cancel().await {
                    tracing::warn!("cancel query failed: {e}");
                }
            });
        }
    }

    fn last_page(&self) -> Option<u64> {
        let (total, limit) = (self.total?, self.limit?);
        Some(total.saturating_sub(1) / limit.max(1))
    }

    fn go_page(&mut self, page: u64, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.last_page().map_or(page, |last| page.min(last));
        if page != self.page {
            self.page = page;
            self.reload_committed(false, window, cx);
        }
    }

    fn reproject(&mut self, cx: &mut Context<Self>) -> bool {
        let hidden = self.hidden.clone();
        let ok = self
            .grid
            .update(cx, |g, cx| g.set_hidden_columns(&hidden, cx));
        if !ok {
            return false;
        }
        self.form_for = None;
        // The focused cell may keep its coordinates on new rows; the next `render` reloads the
        // cell editor through `sync_editor`
        self.editor_cell = None;
        true
    }

    fn insert_row(&mut self, cx: &mut Context<Self>) {
        self.grid.update(cx, |g, cx| {
            let mut row = 0;
            g.update_model(cx, |m| row = m.insert_row());
            g.scroll_to_row(row, cx);
            g.settle(cx);
        });
        self.form_for = None;
    }

    fn toggle_column(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self
            .grid
            .read(cx)
            .model(cx)
            .all_columns()
            .filter(|c| !self.hidden.contains(c.name.as_ref()))
            .count();
        let mut next = self.hidden.clone();
        if next.contains(&name) {
            next.remove(&name);
        } else if visible <= 1 {
            return;
        } else {
            next.insert(name);
        }
        if self.grid.read(cx).model(cx).has_changes() {
            // A commit in flight would leave the new set without its reprojection
            if self.busy {
                return;
            }
            self.hidden_revert = Some(std::mem::replace(&mut self.hidden, next));
            self.apply_changes(false, window, cx);
        } else {
            let previous = std::mem::replace(&mut self.hidden, next);
            if !self.reproject(cx) {
                self.hidden = previous;
            }
        }
    }

    fn columns_menu(&self, cx: &App) -> NativeMenu {
        let model = self.grid.read(cx).model(cx);
        let names: Vec<String> = if model.all_columns().next().is_none() {
            self.columns.iter().map(|c| c.name.clone()).collect()
        } else {
            model.all_columns().map(|c| c.name.to_string()).collect()
        };
        names.into_iter().fold(NativeMenu::new(), |menu, name| {
            let shown = !self.hidden.contains(&name);
            menu.menu_with_check(
                name.clone(),
                shown,
                Box::new(HideColumn { name: name.into() }),
            )
        })
    }

    fn toggle_raw(&mut self, cx: &mut Context<Self>) {
        self.raw_data = !self.raw_data;
        self.form_for = None;
        let raw = self.raw_data;
        self.grid.update(cx, |g, cx| g.set_raw_mode(raw, cx));
        cx.notify();
    }

    fn show_sql(&self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.commit_statements(cx).statements.join(";\n");
        let text = if pending.is_empty() {
            self.status.to_string()
        } else {
            format!("{}\n\n{pending}", self.status)
        };
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(false)
        });
        editor.update(cx, |s, cx| {
            s.set_readonly(true, cx);
            s.set_value(text, window, cx);
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .w(tokens::conn_form::WIDTH.to_pixels(window.rem_size()))
                .title(t!("viewer.show_sql").to_string())
                .child(
                    div()
                        .id("sql-preview")
                        .h(tokens::viewer::CELL_EDITOR_HEIGHT.rems())
                        .w_full()
                        .child(
                            Editor::new(&editor)
                                .readonly(true)
                                .bordered(false)
                                .size_full(),
                        ),
                )
        });
    }

    fn toggle_rowid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialect != Some(Dialect::Sqlite) {
            return;
        }
        self.show_rowid = !self.show_rowid;
        let on = self.show_rowid;
        self.grid.update(cx, |g, _| g.show_rowid = on);
        self.reload_committed(false, window, cx);
    }

    /// Menu items dispatch along the focused element. Stay on the grid when it
    /// already has focus; otherwise focus this viewer so the item is handled.
    fn focus_for_menu(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let ready = self.session.is_some() && !self.busy;
        let button = |id: &'static str, icon: &'static str, label: String| {
            let mut glyph = lucide(icon);
            if let Some(color) = tokens::icon_color(icon, cx) {
                glyph = glyph.text_color(color);
            }
            Button::new(id)
                .small()
                .custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(p.label)
                        .hover(p.fill)
                        .active(p.chrome_active),
                )
                .icon(glyph)
                .label(label)
        };
        h_flex()
            .h(tokens::object_toolbar::HEIGHT.rems())
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(p.separator)
            .map(|bar| {
                if self.in_transaction {
                    bar.child(
                        button("tx-commit", "check", t!("viewer.commit").to_string())
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_changes(true, window, cx)
                            })),
                    )
                    .child(
                        button(
                            "tx-rollback",
                            "rotate-ccw",
                            t!("viewer.rollback").to_string(),
                        )
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| this.rollback(cx))),
                    )
                } else {
                    bar.child(
                        button(
                            "tx-begin",
                            "git-commit-horizontal",
                            t!("viewer.begin").to_string(),
                        )
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, _, cx| this.begin_transaction(cx))),
                    )
                }
            })
            .child(
                button("toggle-filter", "funnel", t!("viewer.filter").to_string())
                    .selected(self.show_filter)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_filter_pane(window, cx)),
                    ),
            )
            .child(
                button("viewer-find", "search", t!("viewer.find").to_string())
                    .selected(self.show_find)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.show_find {
                            this.close_find(window, cx);
                        } else {
                            this.open_find(false, window, cx);
                        }
                    })),
            )
            .child(button("import", "file-text", t!("viewer.import").to_string()).disabled(true))
            .child(button("export", "file-text", t!("viewer.export").to_string()).disabled(true))
            .child(div().flex_1())
            .child(
                Button::new("viewer-columns")
                    .ghost()
                    .small()
                    .icon(lucide("columns-3"))
                    .tooltip(t!("viewer.columns").to_string())
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        this.focus_for_menu(window, cx);
                        this.columns_menu(cx).show(event.position(), window, cx);
                    })),
            )
            .child(
                Button::new("viewer-raw")
                    .ghost()
                    .small()
                    .icon(lucide("square-function"))
                    .tooltip(t!("viewer.raw_data").to_string())
                    .selected(self.raw_data)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_raw(cx))),
            )
            .child(
                Button::new("viewer-form")
                    .ghost()
                    .small()
                    .icon(lucide("list"))
                    .tooltip(t!("viewer.form").to_string())
                    .selected(self.form)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.form {
                            this.flush_form(cx);
                        }
                        this.form = !this.form;
                        this.form_for = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("viewer-editor")
                    .ghost()
                    .small()
                    .icon(lucide("code"))
                    .tooltip(t!("viewer.editor").to_string())
                    .selected(self.editor.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.editor.take().is_some() {
                            cx.notify();
                        } else {
                            this.open_editor(Kind::Text, window, cx);
                        }
                    })),
            )
            .child(
                Button::new("focus-mode")
                    .ghost()
                    .small()
                    .icon(lucide("scan"))
                    .tooltip(t!("viewer.focus_mode").to_string())
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(navidog_core::actions::ToggleFocusMode), cx)
                    }),
            )
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let ready = self.session.is_some() && !self.busy;
        let dirty = self.grid.read(cx).model(cx).has_changes();
        let icon_button = |id: &'static str, icon: &'static str, tip: String| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(lucide(icon))
                .tooltip(tip)
                .w(tokens::viewer::BOTTOM_BUTTON_WIDTH.rems())
                .h(tokens::viewer::BOTTOM_BUTTON_HEIGHT.rems())
        };
        let page = self.page;
        let last = self.last_page();
        let model = self.grid.read(cx).model(cx);
        let (rows, selected) = (model.row_count(), model.head().0 + 1);
        let mut records = t!(
            "viewer.records",
            selected = selected.min(rows),
            count = rows,
            page = page + 1
        )
        .to_string();
        if let Some(total) = self.total {
            records = format!("{records}  {}", t!("viewer.total", total = total));
        }
        h_flex()
            .h(tokens::viewer::BOTTOM_BAR_HEIGHT.rems())
            .min_w_0()
            .flex_none()
            .px_2()
            .gap_1()
            .border_t_1()
            .border_color(p.separator)
            .text_xs()
            .child(
                icon_button("row-add", "plus", t!("viewer.add_row").to_string())
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.flush_form(cx);
                        let dirty = {
                            let model = this.grid.read(cx).model(cx);
                            model.row_dirty(model.head().0)
                        };
                        if dirty {
                            this.add_after_commit = true;
                            this.apply_changes(false, window, cx);
                        } else {
                            this.insert_row(cx);
                        }
                    })),
            )
            .child(
                icon_button("row-delete", "minus", t!("viewer.delete_row").to_string())
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid
                            .update(cx, |g, cx| g.update_model(cx, |m| m.delete_selected_rows()));
                    })),
            )
            .child(
                icon_button("row-apply", "check", t!("viewer.apply_changes").to_string())
                    .disabled(!ready || !dirty)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.apply_changes(false, window, cx)),
                    ),
            )
            .child(
                icon_button("row-discard", "x", t!("viewer.discard_changes").to_string())
                    .disabled(!dirty || self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid
                            .update(cx, |g, cx| g.update_model(cx, |m| m.discard()));
                        this.form_for = None;
                    })),
            )
            .child({
                let mut reload = Button::new("reload")
                    .ghost()
                    .xsmall()
                    .tooltip(t!("viewer.refresh").to_string())
                    .w(tokens::viewer::BOTTOM_BUTTON_WIDTH.rems())
                    .h(tokens::viewer::BOTTOM_BUTTON_HEIGHT.rems())
                    .disabled(self.session.is_none());
                // Disabled would fade the icon, so a spin looks like it never started.
                if self.spinning() {
                    reload = reload.child(
                        Spinner::new()
                            .icon(lucide("refresh-cw"))
                            .xsmall()
                            .ease(|t| t),
                    );
                } else {
                    reload = reload.icon(lucide("refresh-cw"));
                }
                reload.on_click(cx.listener(|this, _, window, cx| {
                    if this.busy {
                        return;
                    }
                    this.reload_committed(true, window, cx);
                }))
            })
            .child(
                icon_button("stop", "circle-stop", t!("viewer.stop").to_string())
                    .disabled(!self.busy || self.cancel.is_none())
                    .on_click(cx.listener(|this, _, _, _| this.stop())),
            )
            .child(
                icon_button("show-sql", "code", t!("viewer.show_sql").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.show_sql(window, cx))),
            )
            .child(
                // The last SQL or error is often longer than the bar; hover shows all of it
                truncated("viewer-status", self.status.clone())
                    .flex_1()
                    .px_2()
                    .text_color(p.label_secondary),
            )
            .when(
                !self.columns.is_empty() && !self.columns.iter().any(|c| c.primary_key),
                |bar| {
                    bar.child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_color(p.label_secondary)
                            .child(t!("viewer.no_primary_key").to_string()),
                    )
                },
            )
            .child(div().text_color(p.label_secondary).child(records))
            .child(
                h_flex()
                    .w(tokens::viewer::PAGER_WIDTH.rems())
                    .justify_center()
                    .child(
                        icon_button(
                            "page-first",
                            "chevron-first",
                            t!("viewer.first_page").to_string(),
                        )
                        .disabled(!ready || page == 0)
                        .on_click(cx.listener(|this, _, window, cx| this.go_page(0, window, cx))),
                    )
                    .child(
                        icon_button(
                            "page-prev",
                            "chevron-left",
                            t!("viewer.prev_page").to_string(),
                        )
                        .disabled(!ready || page == 0)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.go_page(page.saturating_sub(1), window, cx)
                            },
                        )),
                    )
                    .child(
                        icon_button(
                            "page-next",
                            "chevron-right",
                            t!("viewer.next_page").to_string(),
                        )
                        .disabled(!ready || self.limit.is_none() || last.is_some_and(|l| page >= l))
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.go_page(page + 1, window, cx),
                        )),
                    )
                    .child(
                        icon_button(
                            "page-last",
                            "chevron-last",
                            t!("viewer.last_page").to_string(),
                        )
                        .disabled(!ready || last.is_none_or(|l| page >= l))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if let Some(l) = last {
                                    this.go_page(l, window, cx)
                                }
                            },
                        )),
                    ),
            )
            .child(
                h_flex()
                    .w(tokens::viewer::LIMIT_WIDTH.rems())
                    .gap_1()
                    .child(
                        Checkbox::new("limit")
                            .xsmall()
                            .label(t!("viewer.limit").to_string())
                            .checked(self.limit.is_some())
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                this.limit = checked
                                    .then(|| this.limit_input.read(cx).value().trim().parse().ok())
                                    .flatten()
                                    .or(checked
                                        .then_some(Settings::global(cx).records_per_page.max(1)));
                                this.page = 0;
                                this.reload_committed(false, window, cx);
                            })),
                    )
                    .child(
                        Input::new(&self.limit_input)
                            .xsmall()
                            .flex_1()
                            .disabled(self.limit.is_none()),
                    )
                    .child(t!("viewer.per_page").to_string()),
            )
    }

    /// Grid or form, with the cell editor as a draggable split when it is open.
    fn render_main(&self, form: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .when(!form, |d| d.child(self.grid.clone()))
            .when(form, |d| d.child(self.render_form(cx)));
        if self.editor.is_none() {
            return body.into_any_element();
        }
        let rem = window.rem_size();
        // Two button rows. The text area is the rest of the default height.
        let chrome = tokens::viewer::FILTER_HEIGHT.to_pixels(rem) * 2.;
        // raw-px: the divider position the user dragged to, kept in pixels
        let height = self
            .cell_editor_px
            .map(px)
            .unwrap_or_else(|| chrome + tokens::viewer::CELL_EDITOR_HEIGHT.to_pixels(rem));
        let viewer = cx.entity();
        v_resizable("viewer-cell-split")
            .on_resize(move |state, _, cx| {
                let Some(size) = state.read(cx).sizes().get(1).copied() else {
                    return;
                };
                viewer.update(cx, |this, _| this.cell_editor_px = Some(f32::from(size)));
            })
            .child(resizable_panel().child(body))
            .child(
                resizable_panel()
                    .size(height)
                    .size_range(chrome..gpui_kit::Pixels::MAX)
                    .child(self.render_editor(cx)),
            )
            .into_any_element()
    }
}

impl Render for TableViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_form(window, cx);
        self.sync_editor(window, cx);
        let form = self.form;
        v_flex()
            .id("table-viewer")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(palette(cx).control_bg)
            .on_action(cx.listener(|this, _: &ApplyChanges, window, cx| {
                this.apply_changes(false, window, cx)
            }))
            .on_action(cx.listener(|this, action: &HideColumn, window, cx| {
                this.toggle_column(action.name.to_string(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRawData, _, cx| this.toggle_raw(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleRowId, window, cx| this.toggle_rowid(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.show_find {
                    this.close_find(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|this, _: &ShowFind, window, cx| this.open_find(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowReplace, window, cx| this.open_find(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, _, cx| {
                this.show_find = true;
                this.find_step(true, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ToggleFilter, window, cx| {
                    this.toggle_filter_pane(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &BeginTransaction, _, cx| this.begin_transaction(cx)))
            .on_action(cx.listener(|this, _: &CommitTransaction, window, cx| {
                this.apply_changes(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RollbackTransaction, _, cx| this.rollback(cx)))
            .on_action(cx.listener(|this, _: &QueryTable, window, cx| this.query_table(window, cx)))
            .on_action(|_: &DesignTable, _, _| {})
            .on_action(
                cx.listener(|this, _: &ApplyFilterSort, window, cx| this.apply_filter(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ClearFilterSort, window, cx| {
                this.clear_filter_sort(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RefreshTable, window, cx| {
                this.reload_committed(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StopLoad, _, _| this.stop()))
            .on_action(
                cx.listener(|this, _: &SortAscending, _, cx| this.sort_focused(Some(true), cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortDescending, _, cx| this.sort_focused(Some(false), cx)),
            )
            .on_action(cx.listener(|this, _: &SortNone, _, cx| this.sort_focused(None, cx)))
            .on_action(cx.listener(|this, _: &FindData, window, cx| {
                this.find_columns = false;
                this.replace_mode = false;
                this.open_find(false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindReplaceMode, window, cx| {
                this.find_columns = false;
                this.open_find(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindColumnsMode, window, cx| {
                this.find_columns = true;
                this.replace_mode = false;
                this.open_find(false, window, cx);
                this.find_step(false, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMatchCase, _, cx| {
                this.find_case = !this.find_case;
                this.sync_highlight(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleHighlight, _, cx| {
                this.find_highlight = !this.find_highlight;
                this.sync_highlight(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleIncremental, _, cx| {
                this.find_incremental = !this.find_incremental;
                cx.notify();
            }))
            .on_action(cx.listener(|this, a: &PickField, _, cx| {
                if let Some(rule) = this.rules.get_mut(a.index) {
                    rule.field = a.name.to_string();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, a: &PickOp, _, cx| {
                let Some(op) = filter::Op::parse_key(&a.op) else {
                    return;
                };
                if let Some(rule) = this.rules.get_mut(a.index) {
                    rule.op = op;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, a: &PickSort, _, cx| {
                if let Some(sort) = this.sorts.get_mut(a.index) {
                    sort.field = a.name.to_string();
                    cx.notify();
                }
            }))
            .child(self.render_toolbar(cx))
            .when(self.show_find, |v| v.child(self.render_find(cx)))
            .when(self.show_filter, |v| v.child(self.render_filter(cx)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(self.render_main(form, window, cx)),
            )
            .child(self.render_bottom_bar(cx))
    }
}

fn refresh_spinning(busy: bool, until: Option<Instant>, now: Instant) -> bool {
    busy || until.is_some_and(|t| now < t)
}

impl Drop for TableViewer {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            close_session(s);
        }
    }
}

#[cfg(test)]
mod spin_tests {
    use super::refresh_spinning;
    use std::time::{Duration, Instant};

    #[test]
    fn refresh_keeps_spinning_for_half_a_turn_after_the_load() {
        let start = Instant::now();
        let until = start + Duration::from_millis(400);
        assert!(refresh_spinning(true, Some(until), start));
        assert!(refresh_spinning(
            false,
            Some(until),
            start + Duration::from_millis(100)
        ));
        assert!(!refresh_spinning(
            false,
            Some(until),
            start + Duration::from_millis(400)
        ));
        assert!(!refresh_spinning(false, None, start));
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use gpui_kit::{TestAppContext, WindowOptions};
    use navidog_core::{connection::Connection, crypto::Key, store::ConnectionStore};
    use navidog_db::{ConnectParams, Secrets, connect, sqlite::SqliteDriver};
    use std::sync::Arc;
    use std::time::Duration;

    struct Host(Entity<TableViewer>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    fn sql(file: &std::path::Path, sql: &str) -> Vec<Vec<Option<String>>> {
        navidog_db::runtime::runtime().block_on(async {
            let mut session = connect(
                Arc::new(SqliteDriver::default()),
                ConnectParams {
                    file: Some(file.into()),
                    create_new: !file.exists(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let rows = session
                .relational()
                .unwrap()
                .execute(sql)
                .await
                .unwrap()
                .pop()
                .unwrap()
                .rows;
            session.close().await.unwrap();
            rows
        })
    }

    fn wait(cx: &mut TestAppContext, done: impl Fn(&mut TestAppContext) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(cx) {
            assert!(Instant::now() < deadline, "the viewer operation timed out");
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[gpui_kit::test]
    fn visibility_uses_accepted_updates_inserts_and_deletes(cx: &mut TestAppContext) {
        let home =
            std::env::temp_dir().join(format!("navidog-viewer-ownership-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let file = home.join("rows.db");
        sql(
            &file,
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES(1, 'a'), (2, 'b')",
        );
        // SAFETY: this is the only test in this process that changes the configuration directory.
        unsafe {
            std::env::set_var("NAVIDOG_HOME", &home);
        }
        cx.executor().allow_parking();
        let id = cx.update(|cx| {
            gpui_kit::init(cx);
            navidog_core::init(cx);
            navidog_ui::theme::init(cx);
            navidog_db::init(cx);
            crate::init(cx);
            Settings::update(cx, |s| {
                s.sync_record = false;
                s.auto_begin_transaction = false;
            });
            ConnectionStore::global(cx).update(cx, |s, cx| {
                s.set_key(Key::random());
                let mut connection = Connection::new("sqlite");
                connection.name = "ownership".into();
                connection.general.file = Some(file.to_string_lossy().into_owned());
                let id = connection.id.clone();
                s.upsert(connection, &[], cx).unwrap();
                id
            })
        });
        cx.update(|cx| {
            Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
        });
        wait(cx, |cx| {
            cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
        });
        let (handle, viewer) = cx.update(|cx| {
            let mut viewer = None;
            let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let entity = TableViewer::new(
                    ObjectRef {
                        scope: Scope::new(id.clone(), "main", None),
                        kind: ObjectKind::Table,
                        name: "t".into(),
                    },
                    window,
                    cx,
                );
                viewer = Some(entity.clone());
                cx.new(|_| Host(entity))
            })
            .unwrap();
            (handle.0, viewer.unwrap())
        });
        wait(cx, |cx| {
            cx.update(|cx| {
                !viewer.read(cx).busy && viewer.read(cx).grid.read(cx).model(cx).row_count() == 2
            })
        });
        cx.update_window(handle, |_, window, cx| {
            viewer.update(cx, |v, cx| {
                v.grid.update(cx, |g, cx| {
                    g.update_model(cx, |m| {
                        m.set_value(0, 1, Some("z".into()));
                        m.select((1, 0), false);
                        m.delete_selected_rows();
                        let row = m.insert_row();
                        m.set_value(row, 0, Some("3".into()));
                        m.set_value(row, 1, Some("c".into()));
                    })
                });
                v.apply_changes(false, window, cx);
            })
        })
        .unwrap();
        wait(cx, |cx| cx.update(|cx| !viewer.read(cx).busy));
        cx.update_window(handle, |_, window, cx| {
            viewer.update(cx, |v, cx| {
                v.toggle_column("id".into(), window, cx);
                v.toggle_column("id".into(), window, cx);
                let model = v.grid.read(cx).model(cx);
                assert_eq!(model.row_count(), 2);
                assert_eq!(model.value(0, 0), Some(&Some("1".into())));
                assert_eq!(model.value(0, 1), Some(&Some("z".into())));
                assert_eq!(model.value(1, 0), Some(&Some("3".into())));
                assert_eq!(model.value(1, 1), Some(&Some("c".into())));

                v.insert_row(cx);
                v.toggle_column("id".into(), window, cx);
                assert!(v.hidden.contains("id"));
                let model = v.grid.read(cx).model(cx);
                assert_eq!(model.columns.len(), 1);
                assert_eq!(model.columns[0].name, "name");
                assert_eq!(model.row_count(), 3);
                assert_eq!(model.row_state(2), crate::model::RowState::Inserted);
                assert!(model.change_set().inserts.is_empty());
                v.toggle_column("id".into(), window, cx);
                let model = v.grid.read(cx).model(cx);
                assert_eq!(model.columns.len(), 2);
                assert_eq!(model.value(2, 0), None);
                assert_eq!(model.value(2, 1), None);
            })
        })
        .unwrap();
        assert_eq!(
            sql(&file, "SELECT id,name FROM t ORDER BY id"),
            [
                vec![Some("1".into()), Some("z".into())],
                vec![Some("3".into()), Some("c".into())]
            ]
        );
    }
}
