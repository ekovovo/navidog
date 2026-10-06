//! Table viewer: paging, row count, WHERE / ORDER BY text, editing, commits and transactions
//! (21-table-viewer.md §3–6).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, WeakEntity, Window, actions, div,
};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent, SharedSession, run_on};
use navidog_db::{CancelHandle, ColumnInfo, DbResult, ResultSet};
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;

use crate::grid::{GridEvent, GridTarget, ResultGrid};
use crate::model::GridColumn;

const PANEL: &str = "TableViewer";
const CONTEXT: &str = "TableViewer";
const DEFAULT_LIMIT: u64 = 1000;

actions!(viewer, [ApplyChanges]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-s", ApplyChanges, Some(CONTEXT))]);
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
    limit: Option<u64>,
    page: u64,
    total: Option<u64>,
    session: Option<SharedSession>,
    cancel: Option<CancelHandle>,
    dialect: Option<Dialect>,
    columns: Vec<ColumnInfo>,
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
            alert
                .icon(lucide("info"))
                .title(t!("viewer.close_pending", name = name).to_string())
                .show_cancel(true)
                .ok_text(t!("viewer.commit").to_string())
                .cancel_text(t!("viewer.discard").to_string())
                .on_ok(move |_, _, _| {
                    let (session, commit) = (ok_session.clone(), commit.clone());
                    navidog_db::runtime::spawn(async move {
                        let r = run_on(session.clone(), move |rel| {
                            Box::pin(apply(rel, commit, in_tx, true))
                        })
                        .await;
                        if let Ok(Err(e)) = r {
                            tracing::warn!("关闭选项卡时提交失败: {e}");
                        }
                        close_session(session);
                    });
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
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("table")),
            self.object.name.clone(),
        )
    }
}

/// Change statements and how to commit them, run in the background when the tab is closed.
#[derive(Clone)]
struct Commit {
    statements: Vec<String>,
    begin: &'static str,
}

/// Applies the changes. Without an open transaction they are wrapped in one and rolled back on
/// failure, so either all succeed or none take effect.
/// When `end` is true, the already open transaction is ended (committed).
async fn apply(
    rel: &mut dyn navidog_db::Relational,
    c: Commit,
    in_tx: bool,
    end: bool,
) -> DbResult<()> {
    if !in_tx {
        rel.execute(c.begin).await?;
    }
    for s in &c.statements {
        if let Err(e) = rel.execute(s).await {
            if !in_tx {
                let _ = rel.execute("ROLLBACK").await;
            }
            return Err(e);
        }
    }
    if !in_tx || end {
        rel.execute("COMMIT").await?;
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
            let limit_input =
                cx.new(|cx| InputState::new(window, cx).default_value(DEFAULT_LIMIT.to_string()));
            let apply_on_enter = |this: &mut Self,
                                  _: &Entity<InputState>,
                                  e: &InputEvent,
                                  _: &mut Window,
                                  cx: &mut Context<Self>| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.page = 0;
                    this.reload(true, cx);
                }
            };
            let subs = vec![
                cx.subscribe_in(&filter, window, apply_on_enter),
                cx.subscribe_in(&order, window, apply_on_enter),
                cx.subscribe_in(
                    &limit_input,
                    window,
                    |this, input, e: &InputEvent, _, cx| {
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
                            this.reload(false, cx);
                        }
                    },
                ),
                cx.subscribe_in(
                    &grid,
                    window,
                    |this, _, e: &GridEvent, window, cx| match e {
                        GridEvent::SortChanged(sort) => {
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
                            this.page = 0;
                            this.reload(false, cx);
                        }
                        GridEvent::Changed => cx.notify(),
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
                        SessionsEvent::Closed(id) if *id == this.object.scope.connection => {
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
                show_filter: false,
                limit: Some(DEFAULT_LIMIT),
                page: 0,
                total: None,
                session: None,
                cancel: None,
                dialect: None,
                columns: Vec::new(),
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
        self.busy = true;
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
                        this.grid.update(cx, |g, _| {
                            g.set_target(Some(GridTarget {
                                dialect,
                                scope: this.object.scope.clone(),
                                table: this.object.name.clone(),
                            }))
                        });
                    }
                    Ok(Err(e)) => this.status = e.to_string().into(),
                    Err(e) => this.status = e.to_string().into(),
                }
                this.session = Some(shared);
                this.cancel = cancel;
                this.reload(true, cx);
            });
        }));
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.busy = false;
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
        let sql = d.select_page(scope, table, &filter, &order, self.limit, offset);
        let count_sql = count.then(|| d.count(scope, table, &filter));
        self.status = sql.clone().into();
        self.busy = true;
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
                this.busy = false;
                match result {
                    Ok(mut sets) if !sets.is_empty() => {
                        let set = sets.swap_remove(0);
                        let pk = |name: &str| {
                            this.columns.iter().any(|c| c.name == name && c.primary_key)
                        };
                        let columns = set
                            .columns
                            .iter()
                            .map(|c| GridColumn {
                                name: c.name.clone().into(),
                                type_name: c.type_name.clone().into(),
                                primary_key: pk(&c.name),
                            })
                            .collect();
                        let rows = set
                            .rows
                            .into_iter()
                            .map(|r| r.into_iter().map(|v| v.map(Into::into)).collect())
                            .collect();
                        this.grid.update(cx, |g, cx| g.set_data(columns, rows, cx));
                    }
                    Ok(_) => {}
                    Err(e) => this.status = e.into(),
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
        let commit = self.commit_statements(cx);
        if commit.statements.is_empty() && !end_transaction {
            return;
        }
        let in_tx = self.in_transaction;
        self.status = commit.statements.join(";\n").into();
        self.busy = true;
        let job = run_on(session, move |rel| {
            Box::pin(apply(rel, commit, in_tx, end_transaction))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if end_transaction {
                            this.in_transaction = false;
                        }
                        this.reload(true, cx);
                    }
                    Err(e) => {
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
        self.task = Some(cx.spawn(async move |this, cx| {
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
        }));
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
                    tracing::warn!("取消查询失败: {e}");
                }
            });
        }
    }

    fn last_page(&self) -> Option<u64> {
        let (total, limit) = (self.total?, self.limit?);
        Some(total.saturating_sub(1) / limit.max(1))
    }

    fn go_page(&mut self, page: u64, cx: &mut Context<Self>) {
        let page = self.last_page().map_or(page, |last| page.min(last));
        if page != self.page {
            self.page = page;
            self.reload(false, cx);
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let ready = self.session.is_some() && !self.busy;
        let button = |id: &'static str, icon: &'static str, label: String| {
            Button::new(id)
                .ghost()
                .small()
                .icon(lucide(icon))
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
                button(
                    "toggle-sort",
                    "arrow-up-down",
                    t!("viewer.sort").to_string(),
                )
                .selected(self.show_filter)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_filter = !this.show_filter;
                    cx.notify();
                })),
            )
            .child(button("import", "file-text", t!("viewer.import").to_string()).disabled(true))
            .child(button("export", "file-text", t!("viewer.export").to_string()).disabled(true))
    }

    fn render_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let label = |s: &'static str| div().text_color(p.label_secondary).child(s);
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(p.separator)
            .child(label("WHERE"))
            .child(Input::new(&self.filter).xsmall().flex_1())
            .child(label("ORDER BY"))
            .child(Input::new(&self.order).xsmall().flex_1())
            .child(
                Button::new("apply-filter")
                    .small()
                    .label(t!("viewer.apply").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = 0;
                        this.reload(true, cx);
                    })),
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
            .px_2()
            .gap_1()
            .border_t_1()
            .border_color(p.separator)
            .text_xs()
            .child(
                icon_button("row-add", "plus", t!("viewer.add_row").to_string())
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid.update(cx, |g, cx| {
                            let mut row = 0;
                            g.update_model(cx, |m| row = m.insert_row());
                            g.scroll_to_row(row, cx);
                        });
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
                    .disabled(!dirty)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid
                            .update(cx, |g, cx| g.update_model(cx, |m| m.discard()));
                    })),
            )
            .child(
                icon_button("reload", "refresh-cw", t!("viewer.refresh").to_string())
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(true, cx))),
            )
            .child(
                icon_button("stop", "circle-stop", t!("viewer.stop").to_string())
                    .disabled(!self.busy || self.cancel.is_none())
                    .on_click(cx.listener(|this, _, _, _| this.stop())),
            )
            .child(
                div()
                    .id("viewer-status")
                    .flex_1()
                    .px_2()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(p.label_secondary)
                    .child(self.status.clone()),
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
                        .on_click(cx.listener(|this, _, _, cx| this.go_page(0, cx))),
                    )
                    .child(
                        icon_button(
                            "page-prev",
                            "chevron-left",
                            t!("viewer.prev_page").to_string(),
                        )
                        .disabled(!ready || page == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.go_page(page.saturating_sub(1), cx)
                        })),
                    )
                    .child(
                        icon_button(
                            "page-next",
                            "chevron-right",
                            t!("viewer.next_page").to_string(),
                        )
                        .disabled(!ready || self.limit.is_none() || last.is_some_and(|l| page >= l))
                        .on_click(cx.listener(move |this, _, _, cx| this.go_page(page + 1, cx))),
                    )
                    .child(
                        icon_button(
                            "page-last",
                            "chevron-last",
                            t!("viewer.last_page").to_string(),
                        )
                        .disabled(!ready || last.is_none_or(|l| page >= l))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(l) = last {
                                this.go_page(l, cx)
                            }
                        })),
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
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.limit = checked
                                    .then(|| this.limit_input.read(cx).value().trim().parse().ok())
                                    .flatten()
                                    .or(checked.then_some(DEFAULT_LIMIT));
                                this.page = 0;
                                this.reload(false, cx);
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("table-viewer")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(palette(cx).control_bg)
            .on_action(cx.listener(|this, _: &ApplyChanges, window, cx| {
                this.apply_changes(false, window, cx)
            }))
            .child(self.render_toolbar(cx))
            .when(self.show_filter, |v| v.child(self.render_filter(cx)))
            .child(div().flex_1().min_h_0().child(self.grid.clone()))
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
