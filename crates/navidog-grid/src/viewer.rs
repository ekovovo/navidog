//! Table viewer: paging, row count, WHERE / ORDER BY text, editing, commits and transactions
//! (21-table-viewer.md §3–6).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

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
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, Image, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, WeakEntity, Window, actions, div, img,
};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent, SharedSession, run_on};
use navidog_db::{CancelHandle, ColumnInfo, DbResult, ResultSet};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{FormRow, confirm_with, truncated};
use navidog_ui::{palette, tabs, tokens};
use navidog_util::format::{decode_hex, encode_hex, pretty_json};
use navidog_util::sql_type::{Choices, choices};
use navidog_util::starts_with_ignore_ascii_case;
use rust_i18n::t;

use crate::editor::{self, Kind};
use crate::filter::{self, Filter, Rule, Sort};
use crate::grid::{GridEvent, GridTarget, HideColumn, ResultGrid, ToggleRawData, ToggleRowId};
use crate::model::{Cell, GridColumn, GridModel};

const PANEL: &str = "TableViewer";
const CONTEXT: &str = "TableViewer";
actions!(
    viewer,
    [ApplyChanges, ShowFind, FindNext, ShowReplace, ToggleFilter]
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
    /// The loaded page before hidden columns are removed, so toggling them needs no refetch
    raw_columns: Vec<GridColumn>,
    raw_rows: Vec<Vec<Cell>>,
    hidden: HashSet<String>,
    /// Restored when a commit started by hiding a column fails, so the grid and the set still match
    hidden_revert: Option<HashSet<String>>,
    /// Insert a row once the commit that made room for it succeeds
    add_after_commit: bool,
    /// Load the page once the commit started by paging, sorting or filtering succeeds
    reload_after_commit: bool,
    form: bool,
    /// Cell text is submitted as SQL (F-DATA-010)
    raw_data: bool,
    /// SQLite only: SELECT includes rowid as a read-only column
    show_rowid: bool,
    form_inputs: Vec<Entity<InputState>>,
    form_for: Option<usize>,
    in_transaction: bool,
    busy: bool,
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

    /// If the tab is closed with uncommitted changes or an open transaction, ask whether to commit or
    /// discard; the session stays open meanwhile.
    fn on_added_to(&mut self, group: WeakEntity<TabGroup>, _: &mut Window, cx: &mut Context<Self>) {
        tabs::added(PanelId::from(cx.entity_id()), group, cx);
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
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
        let dirty = self.grid.read(cx).model(cx).has_changes();
        if !dirty && !self.in_transaction {
            close_session(session);
            return;
        }
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
                                .and_then(|r| r.map_err(|e| e.to_string()));
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
        let color = store
            .read(cx)
            .get(&self.object.scope.connection)
            .and_then(|c| c.color.clone());
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("table")),
            self.object.name.clone(),
            color.and_then(|c| tokens::connection_color(&c, cx)),
        )
    }
}

/// Change statements and how to commit them, run in the background when the tab is closed.
#[derive(Clone)]
struct Commit {
    statements: Vec<String>,
    begin: &'static str,
    dialect: Dialect,
}

/// Applies the changes; either all take effect or none. Without an open transaction they run in
/// one that is rolled back on failure; inside the user's transaction a savepoint does that, so a
/// failed batch does not leave half its statements in the transaction.
/// When `end` is true, the already open transaction is ended (committed).
async fn apply(
    rel: &mut dyn navidog_db::Relational,
    c: Commit,
    in_tx: bool,
    end: bool,
) -> DbResult<()> {
    const SAVEPOINT: &str = "SAVEPOINT navidog_apply";
    let (start, undo) = if in_tx {
        (SAVEPOINT, "ROLLBACK TO SAVEPOINT navidog_apply")
    } else {
        (c.begin, "ROLLBACK")
    };
    rel.execute(start).await?;
    for s in &c.statements {
        let result = rel.execute(s).await;
        if let Err(e) = result.and_then(|sets| matched_a_row(s, &sets, c.dialect)) {
            let _ = rel.execute(undo).await;
            return Err(e);
        }
    }
    if in_tx {
        rel.execute("RELEASE SAVEPOINT navidog_apply").await?;
    }
    if !in_tx || end {
        rel.execute("COMMIT").await?;
    }
    Ok(())
}

/// An UPDATE or DELETE that matched no row means the grid's old values no longer find it (changed
/// elsewhere, or a value only the server knew). MySQL counts changed rather than matched rows for
/// UPDATE, so an UPDATE to the same value is 0 there too: only its DELETE is checked.
fn matched_a_row(sql: &str, sets: &[ResultSet], dialect: Dialect) -> DbResult<()> {
    let delete = starts_with_ignore_ascii_case(sql, "DELETE");
    let update = starts_with_ignore_ascii_case(sql, "UPDATE") && dialect != Dialect::MySql;
    if (delete || update) && sets.last().and_then(|s| s.affected_rows) == Some(0) {
        return Err(navidog_db::DbError::msg(
            t!("viewer.row_not_found", sql = sql).to_string(),
        ));
    }
    Ok(())
}

fn close_session(session: SharedSession) {
    navidog_db::runtime::spawn(async move {
        if let Some(s) = session.lock().await.take() {
            let _ = s.close().await;
        }
    });
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
                cx.subscribe_in(&find_input, window, |this, _, e: &InputEvent, _, cx| {
                    if matches!(e, InputEvent::Change) && this.find_incremental {
                        this.find_step(false, cx);
                    } else if matches!(e, InputEvent::Change) {
                        this.sync_highlight(cx);
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
                            this.in_transaction = false;
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
                raw_columns: Vec::new(),
                raw_rows: Vec::new(),
                hidden: HashSet::new(),
                hidden_revert: None,
                add_after_commit: false,
                reload_after_commit: false,
                form: false,
                raw_data: false,
                show_rowid: false,
                form_inputs: Vec::new(),
                form_for: None,
                in_transaction: false,
                busy: false,
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
                        .update(cx, |t, cx| t.fail(e.to_string(), cx))
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
            let _ = this.update(cx, |this, cx| {
                match columns {
                    Ok(Ok((dialect, columns))) => {
                        this.dialect = Some(dialect);
                        this.columns = columns;
                        this.grid.update(cx, |g, cx| {
                            g.set_target(
                                Some(GridTarget {
                                    dialect,
                                    scope: this.object.scope.clone(),
                                    table: this.object.name.clone(),
                                }),
                                cx,
                            )
                        });
                    }
                    Ok(Err(e)) => this.status = e.to_string().into(),
                    Err(e) => this.status = e.to_string().into(),
                }
                this.session = Some(shared);
                this.cancel = cancel;
                this.load_foreign_keys(cx);
                // Back after a reopen with edits pending: keep the rows they belong to; they
                // commit on the new session. A reload would discard them.
                if this.grid.read(cx).model(cx).has_changes() {
                    this.set_busy(false, cx);
                    this.status = SharedString::default();
                } else {
                    this.reload(true, cx);
                }
                if Settings::global(cx).auto_begin_transaction {
                    this.begin_transaction(cx);
                }
            });
        }));
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

    fn open_fk_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, col, _, _)) = self.focused_cell(cx) else {
            return;
        };
        let name = self
            .grid
            .read(cx)
            .model(cx)
            .columns
            .get(col)
            .map(|column| column.name.to_string());
        let Some(name) = name else {
            return;
        };
        let Some(key) = self
            .foreign_keys
            .iter()
            .find(|key| key.column == name)
            .cloned()
        else {
            return;
        };
        let (Some(session), Some(dialect)) = (self.session.clone(), self.dialect) else {
            return;
        };
        let viewer = cx.entity().downgrade();
        let picker =
            cx.new(|cx| FkPicker::new(key, dialect, session, viewer, (row, col), window, cx));
        picker.update(cx, |picker, cx| picker.reload(cx));
        window.open_dialog(cx, move |dialog, window, _| {
            // Enter in the search field searches again and leaves the picker open
            confirm_with(dialog, &picker, |p, _, cx| p.reload(cx))
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .title(t!("grid.fk_pick").to_string())
                .child(picker.clone())
        });
    }

    /// `busy` also locks the grid: an edit made while a commit or a load runs would be lost
    /// when it finishes (see `GridModel::locked`).
    fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        self.grid.update(cx, |g, cx| g.set_locked(busy, cx));
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
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = this.update(cx, |this, cx| {
                this.set_busy(false, cx);
                match result {
                    Ok(mut sets) if !sets.is_empty() => {
                        let set = sets.swap_remove(0);
                        let pk = |name: &str| {
                            this.columns.iter().any(|c| c.name == name && c.primary_key)
                        };
                        let columns: Vec<GridColumn> = set
                            .columns
                            .iter()
                            .map(|c| {
                                let info = this.columns.iter().find(|i| i.name == c.name);
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
                        this.raw_columns = columns;
                        this.raw_rows = rows;
                        this.reproject(cx);
                        if std::mem::take(&mut this.add_after_commit) {
                            this.insert_row(cx);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        this.add_after_commit = false;
                        this.status = e.into();
                    }
                }
                cx.notify();
            });
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
        Commit {
            statements,
            begin: d.begin_transaction(),
            dialect: d,
        }
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
        // The statements of a running commit already went to the server; a second commit of the
        // same changes would insert twice
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
            } else if self.hidden_revert.take().is_some() {
                self.reproject(cx);
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
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_busy(false, cx);
                match result {
                    Ok(()) => {
                        if end_transaction {
                            this.in_transaction = false;
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
                .and_then(|r| r.map_err(|e| e.to_string()));
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

    fn begin_transaction(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.dialect else { return };
        self.run_simple(
            d.begin_transaction(),
            |this, _| this.in_transaction = true,
            cx,
        );
    }

    fn rollback(&mut self, cx: &mut Context<Self>) {
        self.run_simple(
            "ROLLBACK",
            |this, cx| {
                this.in_transaction = false;
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

    fn reproject(&mut self, cx: &mut Context<Self>) {
        let (raw_columns, raw_rows, hidden) = (&self.raw_columns, &self.raw_rows, &self.hidden);
        let (columns, rows) = GridModel::project(raw_columns, raw_rows, hidden);
        self.grid.update(cx, |g, cx| {
            g.set_data(columns, rows, cx);
            g.update_model(cx, |m| m.set_hidden_columns(raw_columns, raw_rows, hidden));
        });
        self.form_for = None;
        // The focused cell may keep its coordinates on new rows; the next `render` reloads the
        // cell editor through `sync_editor`
        self.editor_cell = None;
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
            .raw_columns
            .iter()
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
            self.hidden = next;
            self.reproject(cx);
        }
    }

    fn columns_menu(&self) -> NativeMenu {
        let names: Vec<String> = if self.raw_columns.is_empty() {
            self.columns.iter().map(|c| c.name.clone()).collect()
        } else {
            self.raw_columns
                .iter()
                .map(|c| c.name.to_string())
                .collect()
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

    fn flush_form(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.form_for else {
            return;
        };
        let values: Vec<String> = self
            .form_inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        for (c, text) in values.into_iter().enumerate() {
            self.write_form_cell(row, c, text, cx);
        }
    }

    fn write_form_cell(&mut self, row: usize, c: usize, text: String, cx: &mut Context<Self>) {
        let raw = self.raw_data;
        let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                let old = m.raw_text(row, c, raw, dialect);
                if text != old {
                    let cell = Some(text.into());
                    if raw {
                        m.set_raw(row, c, cell);
                    } else {
                        m.set_value(row, c, cell);
                    }
                }
            });
        });
    }

    fn toggle_raw(&mut self, cx: &mut Context<Self>) {
        self.raw_data = !self.raw_data;
        self.form_for = None;
        let raw = self.raw_data;
        self.grid.update(cx, |g, cx| g.set_raw_mode(raw, cx));
        cx.notify();
    }

    fn sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.form {
            return;
        }
        let (row, n) = {
            let model = self.grid.read(cx).model(cx);
            (model.head().0, model.columns.len())
        };
        if self.form_for == Some(row) && self.form_inputs.len() == n {
            return;
        }
        self.form_inputs.clear();
        for c in 0..n {
            let raw = self.raw_data;
            let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
            let value = self.grid.read(cx).model(cx).raw_text(row, c, raw, dialect);
            let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
            let row_at = row;
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let text = input.read(cx).value().to_string();
                        this.write_form_cell(row_at, c, text, cx);
                    }
                },
            )
            .detach();
            self.form_inputs.push(input);
        }
        self.form_for = Some(row);
    }

    fn form_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.flush_form(cx);
        let (row, n) = {
            let model = self.grid.read(cx).model(cx);
            (model.head().0, model.row_count())
        };
        if n == 0 {
            return;
        }
        let next = (row as isize + delta).clamp(0, n as isize - 1) as usize;
        if next == row {
            return;
        }
        self.grid.update(cx, |g, cx| g.move_to_row(next, cx));
        self.form_for = None;
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

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<_> = self
            .grid
            .read(cx)
            .model(cx)
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect();
        v_flex()
            .id("form-view")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap(tokens::form::ROW_GAP.rems())
            .children(
                self.form_inputs
                    .iter()
                    .zip(names)
                    .map(|(input, name)| FormRow::new(name, Input::new(input).flex_1())),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("form-prev")
                            .ghost()
                            .small()
                            .label(t!("viewer.prev_record").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.form_step(-1, cx))),
                    )
                    .child(
                        Button::new("form-next")
                            .ghost()
                            .small()
                            .label(t!("viewer.next_record").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.form_step(1, cx))),
                    ),
            )
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

    fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = true;
        self.replace_mode = replace;
        self.find_input.update(cx, |s, cx| s.focus(window, cx));
        self.sync_highlight(cx);
        cx.notify();
    }

    /// Menu items dispatch along the focused element. Stay on the grid when it
    /// already has focus; otherwise focus this viewer so the item is handled.
    fn focus_for_menu(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = false;
        self.sync_highlight(cx);
        let handle = self.grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn sync_highlight(&mut self, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        let on = self.show_find && self.find_highlight && !self.find_columns && !query.is_empty();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.set_find(on.then(|| (query.into(), case)), cx);
        });
    }

    fn find_step(&mut self, after: bool, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        if query.is_empty() {
            self.sync_highlight(cx);
            return;
        }
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        if self.find_columns {
            let hit = self
                .grid
                .read(cx)
                .model(cx)
                .find_column_name(&query, head.1, after, case);
            if let Some(c) = hit {
                self.grid.update(cx, |g, cx| {
                    g.update_model(cx, |m| m.select_column(c, false))
                });
            }
        } else if let Some(pos) = self
            .grid
            .read(cx)
            .model(cx)
            .find_cell(&query, head, after, case)
        {
            self.grid.update(cx, |g, cx| g.go_to(pos, cx));
        }
        self.sync_highlight(cx);
    }

    fn replace_current(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        let replaced = self.grid.update(cx, |g, cx| {
            let mut ok = false;
            g.update_model(cx, |m| {
                ok = m.replace_cell(head, &query, &replacement, case)
            });
            ok
        });
        self.find_step(replaced, cx);
    }

    fn replace_every(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                m.replace_all(&query, &replacement, case);
            });
        });
        self.sync_highlight(cx);
    }

    fn render_find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let toggle = |id: &'static str, on: bool, label: String| {
            Button::new(id).xsmall().ghost().selected(on).label(label)
        };
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(p.separator)
            .child(
                Button::new("find-close")
                    .xsmall()
                    .ghost()
                    .icon(lucide("x"))
                    .tooltip(t!("viewer.close_find").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
            .child({
                let label = if self.find_columns {
                    t!("viewer.find_columns").to_string()
                } else {
                    t!("viewer.find_data").to_string()
                };
                toggle("find-columns", self.find_columns, label).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.find_columns = !this.find_columns;
                        if this.find_columns {
                            this.replace_mode = false;
                        }
                        this.find_step(false, cx);
                    },
                ))
            })
            .child(Input::new(&self.find_input).xsmall().flex_1())
            .child(
                Button::new("find-next")
                    .xsmall()
                    .ghost()
                    .label(t!("viewer.find_next").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))),
            )
            .child(
                toggle(
                    "find-replace",
                    self.replace_mode,
                    t!("viewer.replace").to_string(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.replace_mode = !this.replace_mode;
                    if this.replace_mode {
                        this.find_columns = false;
                        this.replace_input.update(cx, |s, cx| s.focus(window, cx));
                    }
                    cx.notify();
                })),
            )
            .when(self.replace_mode && !self.find_columns, |bar| {
                bar.child(Input::new(&self.replace_input).xsmall().flex_1())
                    .child(
                        Button::new("replace-one")
                            .xsmall()
                            .ghost()
                            .label(t!("viewer.replace").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.replace_current(cx))),
                    )
                    .child(
                        Button::new("replace-all")
                            .xsmall()
                            .ghost()
                            .label(t!("viewer.replace_all").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.replace_every(cx))),
                    )
            })
            .child(
                toggle(
                    "find-case",
                    self.find_case,
                    t!("viewer.find_case").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_case = !this.find_case;
                    this.sync_highlight(cx);
                })),
            )
            .child(
                toggle(
                    "find-highlight",
                    self.find_highlight,
                    t!("viewer.find_highlight").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_highlight = !this.find_highlight;
                    this.sync_highlight(cx);
                })),
            )
            .child(
                toggle(
                    "find-incremental",
                    self.find_incremental,
                    t!("viewer.find_incremental").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_incremental = !this.find_incremental;
                    cx.notify();
                })),
            )
    }

    fn focused_cell(&self, cx: &App) -> Option<(usize, usize, String, String)> {
        let model = self.grid.read(cx).model(cx);
        if model.columns.is_empty() || model.row_count() == 0 {
            return None;
        }
        let (r, c) = model.head();
        let type_name = model.columns.get(c)?.type_name.to_string();
        let text = match model.value(r, c) {
            Some(Some(s)) => s.to_string(),
            _ => String::new(),
        };
        Some((r, c, text, type_name))
    }

    fn write_at(&mut self, row: usize, col: usize, text: String, cx: &mut Context<Self>) {
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                let current = match m.value(row, col) {
                    Some(Some(s)) => Some(s.to_string()),
                    _ => None,
                };
                if (text.is_empty() && current.is_none())
                    || current.as_deref() == Some(text.as_str())
                {
                    return;
                }
                m.set_value(row, col, Some(text.into()));
            });
        });
    }

    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.editor else {
            return;
        };
        let Some((r, c, text, _)) = self.focused_cell(cx) else {
            return;
        };
        if self.editor_cell == Some((r, c)) {
            return;
        }
        self.editor_cell = Some((r, c));
        self.editor_note = SharedString::default();
        let shown = match kind {
            Kind::Hex => encode_hex(&text),
            Kind::Json => pretty_json(&text).unwrap_or(text.clone()),
            _ => text.clone(),
        };
        if kind == Kind::Json {
            self.json_input
                .update(cx, |s, cx| s.set_value(shown, window, cx));
        } else {
            self.editor_input
                .update(cx, |s, cx| s.set_value(shown, window, cx));
        }
        self.image = editor::sniff_image(text.as_bytes())
            .map(|format| Arc::new(Image::from_bytes(format, text.into_bytes())));
    }

    fn open_editor(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor == Some(kind) {
            return;
        }
        self.editor = Some(kind);
        self.editor_cell = None;
        self.sync_editor(window, cx);
        cx.notify();
    }

    fn apply_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((r, c, _, _)) = self.focused_cell(cx) else {
            return;
        };
        let raw = if self.editor == Some(Kind::Json) {
            self.json_input.read(cx).value().to_string()
        } else {
            self.editor_input.read(cx).value().to_string()
        };
        let text = match self.editor {
            Some(Kind::Hex) => match decode_hex(&raw) {
                Some(text) => text,
                None => {
                    self.editor_note = t!("viewer.editor_hex_invalid").into();
                    cx.notify();
                    return;
                }
            },
            Some(Kind::Text | Kind::Web | Kind::Json) => raw,
            _ => return,
        };
        self.editor_note = SharedString::default();
        self.write_at(r, c, text, cx);
        self.editor_cell = Some((r, c));
    }

    fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.json_input.read(cx).value().to_string();
        match pretty_json(&raw) {
            Some(pretty) => {
                self.editor_note = SharedString::default();
                self.json_input
                    .update(cx, |s, cx| s.set_value(pretty, window, cx));
            }
            None => self.editor_note = t!("viewer.editor_json_invalid").into(),
        }
        cx.notify();
    }

    fn load_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let bytes = std::fs::read(&path).ok();
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(bytes) = bytes else {
                    this.editor_note = t!("viewer.editor_image_read").into();
                    cx.notify();
                    return;
                };
                if let Some(format) = editor::sniff_image(&bytes) {
                    this.image = Some(Arc::new(Image::from_bytes(format, bytes.clone())));
                }
                match String::from_utf8(bytes) {
                    Ok(text) => {
                        if let Some((r, c, _, _)) = this.focused_cell(cx) {
                            this.write_at(r, c, text.clone(), cx);
                            this.editor_cell = Some((r, c));
                            this.editor_input
                                .update(cx, |s, cx| s.set_value(text, window, cx));
                        }
                        this.editor_note = SharedString::default();
                    }
                    Err(_) => this.editor_note = t!("viewer.editor_image_binary").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bytes = if let Some(image) = &self.image {
            image.bytes.clone()
        } else {
            self.focused_cell(cx)
                .map(|(_, _, text, _)| text.into_bytes())
                .unwrap_or_default()
        };
        let rx = cx.prompt_for_new_path(Path::new("."), Some("cell.bin"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            if std::fs::write(&path, bytes).is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.editor_note = t!("viewer.editor_image_write").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn clear_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.image = None;
        if let Some((r, c, _, _)) = self.focused_cell(cx) {
            self.write_at(r, c, String::new(), cx);
            self.editor_cell = Some((r, c));
            self.editor_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.editor_note = SharedString::default();
        cx.notify();
    }

    fn pick_choice(&mut self, value: String, cx: &mut Context<Self>) {
        let Some((r, c, current, type_name)) = self.focused_cell(cx) else {
            return;
        };
        let text = match choices(&type_name) {
            Some(Choices::Many(values)) => {
                let mut picked: Vec<String> = current
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && values.iter().any(|v| v == s))
                    .map(str::to_string)
                    .collect();
                if let Some(ix) = picked.iter().position(|v| v == &value) {
                    picked.remove(ix);
                } else {
                    picked.push(value);
                }
                picked.join(",")
            }
            _ => value,
        };
        self.write_at(r, c, text, cx);
    }

    fn dialect(&self) -> Dialect {
        self.dialect.unwrap_or(Dialect::Sqlite)
    }

    fn field_names(&self) -> Vec<String> {
        if self.raw_columns.is_empty() {
            self.columns.iter().map(|c| c.name.clone()).collect()
        } else {
            self.raw_columns
                .iter()
                .map(|c| c.name.to_string())
                .collect()
        }
    }

    fn make_draft(rule: Rule, window: &mut Window, cx: &mut Context<Self>) -> Draft {
        let value = cx.new(|cx| InputState::new(window, cx).default_value(rule.value));
        let value2 = cx.new(|cx| InputState::new(window, cx).default_value(rule.value2));
        let on_enter = |this: &mut Self,
                        _: &Entity<InputState>,
                        e: &InputEvent,
                        window: &mut Window,
                        cx: &mut Context<Self>| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                this.apply_filter(window, cx);
            }
        };
        let _subscriptions = vec![
            cx.subscribe_in(&value, window, on_enter),
            cx.subscribe_in(&value2, window, on_enter),
        ];
        Draft {
            field: rule.field,
            op: rule.op,
            value,
            value2,
            or_join: rule.or,
            negate: rule.negate,
            _subscriptions,
        }
    }

    fn current_filter(&self, cx: &App) -> Filter {
        Filter {
            rules: self
                .rules
                .iter()
                .map(|d| Rule {
                    field: d.field.clone(),
                    op: d.op,
                    value: d.value.read(cx).value().to_string(),
                    value2: d.value2.read(cx).value().to_string(),
                    or: d.or_join,
                    negate: d.negate,
                })
                .collect(),
            grouped: self.grouped,
            group_negate: self.group_negate,
        }
    }

    fn write_filter_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialect = self.dialect();
        let where_text = filter::where_sql(&self.current_filter(cx), dialect);
        let order_text = filter::order_sql(&self.sorts, dialect);
        self.filter
            .update(cx, |s, cx| s.set_value(where_text, window, cx));
        self.order
            .update(cx, |s, cx| s.set_value(order_text, window, cx));
    }

    fn apply_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.builder {
            self.write_filter_sql(window, cx);
        }
        self.page = 0;
        self.reload_committed(true, window, cx);
    }

    fn set_builder(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on == self.builder {
            return;
        }
        let dialect = self.dialect();
        if on {
            let where_text = self.filter.read(cx).value().to_string();
            let order_text = self.order.read(cx).value().to_string();
            let (Some(parsed), Some(sorts)) = (
                filter::parse_where(&where_text, dialect),
                filter::parse_order(&order_text, dialect),
            ) else {
                return;
            };
            let mut rules = Vec::new();
            for rule in parsed.rules {
                rules.push(Self::make_draft(rule, window, cx));
            }
            self.rules = rules;
            self.sorts = sorts;
            self.grouped = parsed.grouped;
            self.group_negate = parsed.group_negate;
        } else {
            self.write_filter_sql(window, cx);
        }
        self.builder = on;
        cx.notify();
    }

    fn quick_filter(
        &mut self,
        column: &str,
        op: filter::Op,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_filter = true;
        let dialect = self.dialect();
        let rule = Rule {
            field: column.to_string(),
            op,
            value: value.to_string(),
            value2: String::new(),
            or: false,
            negate: false,
        };
        if self.builder {
            self.rules.push(Self::make_draft(rule, window, cx));
            self.write_filter_sql(window, cx);
        } else {
            let extra = filter::where_sql(
                &Filter {
                    rules: vec![rule],
                    ..Filter::default()
                },
                dialect,
            );
            let current = self.filter.read(cx).value().to_string();
            let text = if current.trim().is_empty() {
                extra
            } else {
                format!("({current}) AND {extra}")
            };
            self.filter
                .update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.page = 0;
        self.reload_committed(true, window, cx);
    }

    fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.field_names().into_iter().next().unwrap_or_default();
        self.rules.push(Self::make_draft(
            Rule {
                field,
                op: filter::Op::Eq,
                value: String::new(),
                value2: String::new(),
                or: false,
                negate: false,
            },
            window,
            cx,
        ));
        cx.notify();
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_filter = !this.show_filter;
                        cx.notify();
                    })),
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
                        this.columns_menu().show(event.position(), window, cx);
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

    fn render_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let mut pane = v_flex()
            .id("filter-pane")
            .w_full()
            .min_w_0()
            .max_h(tokens::filter::BUILDER_MAX_HEIGHT.rems())
            .overflow_y_scroll()
            .border_b_1()
            .border_color(p.separator)
            .child(self.render_filter_bar(cx));
        if self.builder {
            for i in 0..self.rules.len() {
                pane = pane.child(self.render_rule(i, cx));
            }
            pane = pane.child(self.render_rule_actions(cx));
            for i in 0..self.sorts.len() {
                pane = pane.child(self.render_sort(i, cx));
            }
            pane = pane.child(self.render_sort_actions(cx));
        } else {
            pane = pane.child(self.render_filter_text(cx));
        }
        pane
    }

    fn render_filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = |id: &'static str, on: bool, label: String| {
            Button::new(id).xsmall().flex_1().selected(on).label(label)
        };
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_2()
            .child(
                h_flex()
                    .w(tokens::filter::SEGMENTED_WIDTH.rems())
                    .gap_1()
                    .child(
                        mode(
                            "filter-builder",
                            self.builder,
                            t!("viewer.builder").to_string(),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.set_builder(true, window, cx)),
                        ),
                    )
                    .child(
                        mode("filter-text", !self.builder, t!("viewer.text").to_string()).on_click(
                            cx.listener(|this, _, window, cx| this.set_builder(false, window, cx)),
                        ),
                    ),
            )
            .when(self.builder, |bar| {
                bar.child(
                    Button::new("filter-group")
                        .xsmall()
                        .selected(self.grouped)
                        .disabled(self.rules.len() < 2)
                        .label(t!("viewer.group").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.grouped = !this.grouped;
                            if !this.grouped {
                                this.group_negate = false;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("filter-group-not")
                        .xsmall()
                        .selected(self.group_negate)
                        .disabled(!self.grouped)
                        .label(t!("viewer.negate").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.group_negate = !this.group_negate;
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("apply-filter")
                    .xsmall()
                    .label(t!("viewer.apply_filter").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.apply_filter(window, cx))),
            )
    }

    fn render_filter_text(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let label = |s: &'static str| div().text_color(p.label_secondary).child(s);
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_2()
            .child(label("WHERE"))
            .child(Input::new(&self.filter).xsmall().flex_1().min_w_0())
            .child(label("ORDER BY"))
            .child(Input::new(&self.order).xsmall().flex_1().min_w_0())
    }

    fn render_rule(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let rule = &self.rules[index];
        let field = if rule.field.is_empty() {
            "…".to_string()
        } else {
            rule.field.clone()
        };
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_1()
            .min_w_0()
            .when(index > 0, |row| {
                let or_join = rule.or_join;
                row.child(
                    Button::new(ElementId::Name(format!("filter-join-{index}").into()))
                        .xsmall()
                        .label(if or_join {
                            t!("viewer.or").to_string()
                        } else {
                            t!("viewer.and").to_string()
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(rule) = this.rules.get_mut(index) {
                                rule.or_join = !rule.or_join;
                                cx.notify();
                            }
                        })),
                )
            })
            .child(
                Button::new(ElementId::Name(format!("filter-not-{index}").into()))
                    .xsmall()
                    .selected(rule.negate)
                    .label(t!("viewer.negate").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(rule) = this.rules.get_mut(index) {
                            rule.negate = !rule.negate;
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("filter-field-{index}").into()))
                    .xsmall()
                    .label(field)
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.focus_for_menu(window, cx);
                        this.field_menu(index).show(event.position(), window, cx);
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("filter-op-{index}").into()))
                    .xsmall()
                    .label(rule.op.label())
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.focus_for_menu(window, cx);
                        this.op_menu(index).show(event.position(), window, cx);
                    })),
            )
            .when(rule.op.needs_value(), |row| {
                row.child(Input::new(&rule.value).xsmall().flex_1().min_w_0())
            })
            .when(rule.op.needs_second(), |row| {
                row.child(Input::new(&rule.value2).xsmall().flex_1().min_w_0())
            })
            .child(
                Button::new(ElementId::Name(format!("filter-remove-{index}").into()))
                    .ghost()
                    .xsmall()
                    .icon(lucide("x"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index < this.rules.len() {
                            this.rules.remove(index);
                            cx.notify();
                        }
                    })),
            )
    }

    fn render_rule_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().px_2().gap_1().child(
            Button::new("filter-add-rule")
                .ghost()
                .xsmall()
                .icon(lucide("plus"))
                .label(t!("viewer.add_rule").to_string())
                .on_click(cx.listener(|this, _, window, cx| this.add_rule(window, cx))),
        )
    }

    fn render_sort(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let sort = &self.sorts[index];
        let field = if sort.field.is_empty() {
            "…".to_string()
        } else {
            sort.field.clone()
        };
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_1()
            .child(
                Button::new(ElementId::Name(format!("sort-field-{index}").into()))
                    .xsmall()
                    .label(field)
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.focus_for_menu(window, cx);
                        this.sort_menu(index).show(event.position(), window, cx);
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("sort-dir-{index}").into()))
                    .xsmall()
                    .label(if sort.asc {
                        t!("viewer.asc").to_string()
                    } else {
                        t!("viewer.desc").to_string()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sort) = this.sorts.get_mut(index) {
                            sort.asc = !sort.asc;
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("sort-up-{index}").into()))
                    .ghost()
                    .xsmall()
                    .disabled(index == 0)
                    .label(t!("viewer.sort_up").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index > 0 && index < this.sorts.len() {
                            this.sorts.swap(index, index - 1);
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("sort-down-{index}").into()))
                    .ghost()
                    .xsmall()
                    .disabled(index + 1 >= self.sorts.len())
                    .label(t!("viewer.sort_down").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index + 1 < this.sorts.len() {
                            this.sorts.swap(index, index + 1);
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new(ElementId::Name(format!("sort-remove-{index}").into()))
                    .ghost()
                    .xsmall()
                    .icon(lucide("x"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index < this.sorts.len() {
                            this.sorts.remove(index);
                            cx.notify();
                        }
                    })),
            )
    }

    fn render_sort_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().px_2().pb_1().gap_1().child(
            Button::new("filter-add-sort")
                .ghost()
                .xsmall()
                .icon(lucide("plus"))
                .label(t!("viewer.add_sort").to_string())
                .on_click(cx.listener(|this, _, _, cx| {
                    let field = this.field_names().into_iter().next().unwrap_or_default();
                    this.sorts.push(Sort { field, asc: true });
                    cx.notify();
                })),
        )
    }

    fn field_menu(&self, index: usize) -> NativeMenu {
        self.field_names()
            .into_iter()
            .fold(NativeMenu::new(), |menu, name| {
                menu.menu(
                    name.clone(),
                    Box::new(PickField {
                        index,
                        name: name.into(),
                    }),
                )
            })
    }

    fn op_menu(&self, index: usize) -> NativeMenu {
        filter::Op::choices(self.dialect())
            .into_iter()
            .fold(NativeMenu::new(), |menu, op| {
                menu.menu(
                    op.label(),
                    Box::new(PickOp {
                        index,
                        op: op.key().into(),
                    }),
                )
            })
    }

    fn sort_menu(&self, index: usize) -> NativeMenu {
        self.field_names()
            .into_iter()
            .fold(NativeMenu::new(), |menu, name| {
                menu.menu(
                    name.clone(),
                    Box::new(PickSort {
                        index,
                        name: name.into(),
                    }),
                )
            })
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let kind = self.editor.unwrap_or(Kind::Text);
        let kind_button = |id: &'static str, on: bool, label: String, next: Kind| {
            Button::new(id)
                .xsmall()
                .selected(on)
                .label(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor(next, window, cx);
                }))
        };
        let (_, _, text, type_name) = self
            .focused_cell(cx)
            .unwrap_or_else(|| (0, 0, String::new(), String::new()));
        // Tabs and the button row sit in this chrome; the text area takes the rest and
        // scrolls inside itself. The pane can shrink so a short window keeps the bottom bar.
        let chrome = tokens::viewer::FILTER_HEIGHT.rems() * 2.;
        v_flex()
            .id("cell-editor")
            .w_full()
            .min_w_0()
            .h(tokens::viewer::CELL_EDITOR_HEIGHT.rems() + chrome)
            .min_h(chrome)
            .flex_shrink(1.)
            .overflow_hidden()
            .border_t_1()
            .border_color(p.separator)
            .px_2()
            .py_1()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(kind_button(
                        "editor-text",
                        kind == Kind::Text,
                        t!("viewer.editor_text").to_string(),
                        Kind::Text,
                    ))
                    .child(kind_button(
                        "editor-hex",
                        kind == Kind::Hex,
                        t!("viewer.editor_hex").to_string(),
                        Kind::Hex,
                    ))
                    .child(kind_button(
                        "editor-image",
                        kind == Kind::Image,
                        t!("viewer.editor_image").to_string(),
                        Kind::Image,
                    ))
                    .child(kind_button(
                        "editor-web",
                        kind == Kind::Web,
                        t!("viewer.editor_web").to_string(),
                        Kind::Web,
                    ))
                    .child(kind_button(
                        "editor-json",
                        kind == Kind::Json,
                        t!("viewer.editor_json").to_string(),
                        Kind::Json,
                    ))
                    .child(kind_button(
                        "editor-enum",
                        kind == Kind::Enum,
                        t!("viewer.editor_enum").to_string(),
                        Kind::Enum,
                    )),
            )
            .when(kind == Kind::Web, |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(t!("viewer.editor_web_note").to_string()),
                )
            })
            .when(
                matches!(kind, Kind::Text | Kind::Hex | Kind::Web | Kind::Json),
                |pane| {
                    pane.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            // raw-px: gpui-kit's focus ring is 3px outside the border; this slot clips it square
                            .p(gpui_kit::px(3.))
                            .when(kind == Kind::Json, |slot| {
                                slot.child(Editor::new(&self.json_input).size_full())
                            })
                            .when(kind != Kind::Json, |slot| {
                                slot.child(Textarea::new(&self.editor_input).size_full())
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("editor-update")
                                    .xsmall()
                                    .label(t!("viewer.editor_update").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply_editor(window, cx)
                                    })),
                            )
                            .when(kind == Kind::Json, |row| {
                                row.child(
                                    Button::new("editor-format")
                                        .xsmall()
                                        .label(t!("viewer.editor_format").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.format_json(window, cx)
                                        })),
                                )
                            }),
                    )
                },
            )
            .when(kind == Kind::Image, |pane| {
                pane.when_some(self.image.clone(), |pane, image| {
                    pane.child(
                        img(image)
                            .max_h(tokens::viewer::CELL_EDITOR_HEIGHT.rems())
                            .max_w(tokens::form::FIELD_WIDTH.rems()),
                    )
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("editor-load")
                                .xsmall()
                                .label(t!("viewer.editor_load").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.load_image(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("editor-save")
                                .xsmall()
                                .label(t!("viewer.editor_save").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_image(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("editor-clear")
                                .xsmall()
                                .label(t!("viewer.editor_clear").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.clear_image(window, cx)),
                                ),
                        ),
                )
            })
            .when(kind == Kind::Enum, |pane| match choices(&type_name) {
                Some(Choices::One(values)) => {
                    pane.child(self.choice_row(&values, &text, false, cx))
                }
                Some(Choices::Many(values)) => {
                    pane.child(self.choice_row(&values, &text, true, cx))
                }
                None => pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(t!("viewer.editor_enum_missing").to_string()),
                ),
            })
            .when(!self.editor_note.is_empty(), |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(self.editor_note.clone()),
                )
            })
    }

    fn choice_row(
        &self,
        values: &[String],
        current: &str,
        many: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let picked: Vec<&str> = if many {
            current
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![current]
        };
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(values.iter().enumerate().map(|(ix, value)| {
                let on = picked.iter().any(|p| *p == value);
                let value = value.clone();
                Checkbox::new(ElementId::Name(format!("editor-choice-{ix}").into()))
                    .xsmall()
                    .label(value.clone())
                    .checked(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_choice(value.clone(), cx);
                    }))
            }))
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
            .child(
                icon_button("reload", "refresh-cw", t!("viewer.refresh").to_string())
                    .disabled(!ready)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.reload_committed(true, window, cx)),
                    ),
            )
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
            .on_action(cx.listener(|this, _: &ToggleFilter, _, cx| {
                this.show_filter = !this.show_filter;
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
                    .when(!form, |d| d.child(self.grid.clone()))
                    .when(form, |d| d.child(self.render_form(cx))),
            )
            .when(self.editor.is_some(), |v| v.child(self.render_editor(cx)))
            .child(self.render_bottom_bar(cx))
    }
}

impl Drop for TableViewer {
    fn drop(&mut self) {
        if let Some(s) = self.session.take() {
            close_session(s);
        }
    }
}

/// Referenced-column picker (F-DATA-012). One click writes the value into the cell that was focused
/// when the dialog opened.
struct FkPicker {
    key: navidog_db::ForeignKey,
    dialect: Dialect,
    session: SharedSession,
    viewer: WeakEntity<TableViewer>,
    cell: (usize, usize),
    search: Entity<InputState>,
    rows: Vec<Option<String>>,
    note: SharedString,
    all: bool,
}

impl FkPicker {
    fn new(
        key: navidog_db::ForeignKey,
        dialect: Dialect,
        session: SharedSession,
        viewer: WeakEntity<TableViewer>,
        cell: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("grid.fk_search").to_string()));
        Self {
            key,
            dialect,
            session,
            viewer,
            cell,
            search,
            rows: Vec::new(),
            note: SharedString::default(),
            all: false,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let search = self.search.read(cx).value().to_string();
        let sql = navidog_db::referenced_values_sql(
            self.dialect,
            &self.key,
            &search,
            (!self.all).then_some(1000),
        );
        let session = self.session.clone();
        let job = run_on(session, move |rel| {
            Box::pin(async move { rel.execute(&sql).await })
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(sets)) => {
                        this.rows = sets
                            .into_iter()
                            .next()
                            .map(|set| {
                                set.rows
                                    .into_iter()
                                    .map(|row| row.into_iter().next().unwrap_or(None))
                                    .collect()
                            })
                            .unwrap_or_default();
                        this.note = if this.rows.is_empty() {
                            t!("grid.fk_empty").into()
                        } else {
                            SharedString::default()
                        };
                    }
                    Ok(Err(e)) => this.note = e.to_string().into(),
                    Err(e) => this.note = e.to_string().into(),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for FkPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .gap_2()
            .h(tokens::form::LIST_HEIGHT.rems())
            .child(
                h_flex()
                    .gap_1()
                    .child(Input::new(&self.search).flex_1())
                    .child(
                        Button::new("fk-all")
                            .xsmall()
                            .selected(self.all)
                            .label(if self.all {
                                t!("grid.fk_limit").to_string()
                            } else {
                                t!("grid.fk_all").to_string()
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.all = !this.all;
                                this.reload(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("fk-rows")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.rows.iter().enumerate().map(|(ix, value)| {
                        let stored = value.clone();
                        let label = stored.clone().unwrap_or_else(|| "NULL".into());
                        div()
                            .id(("fk-row", ix))
                            .h(tokens::form::ROW_HEIGHT.rems())
                            .px_1()
                            .min_w_0()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let value = stored.clone();
                                let (row, col) = this.cell;
                                let _ = this.viewer.update(cx, |viewer, cx| {
                                    viewer.write_cell(row, col, value.map(Into::into), cx);
                                });
                                window.close_dialog(cx);
                            }))
                            .child(truncated(("fk-text", ix), label))
                    })),
            )
            .when(!self.note.is_empty(), |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(self.note.clone()),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_that_matched_no_row_fails() {
        let affected = |n| {
            vec![ResultSet {
                columns: Vec::new(),
                rows: Vec::new(),
                affected_rows: Some(n),
            }]
        };
        let update = "UPDATE \"t\" SET \"name\" = 'b' WHERE \"id\" IS NULL";
        let delete = "DELETE FROM `t` WHERE `id` = 1 LIMIT 1";
        assert!(matched_a_row(update, &affected(0), Dialect::Sqlite).is_err());
        assert!(matched_a_row(update, &affected(1), Dialect::Postgres).is_ok());
        assert!(matched_a_row(delete, &affected(0), Dialect::MySql).is_err());
        // MySQL counts changed rows: an UPDATE to the same value is 0 and still fine
        assert!(matched_a_row(update, &affected(0), Dialect::MySql).is_ok());
        assert!(matched_a_row("INSERT INTO t VALUES (1)", &affected(0), Dialect::Sqlite).is_ok());
    }
}
