//! Results area of a query editor (31-query-results.md §4): result and explain tabs, messages,
//! summary, pinning, and editing single-table results.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, EditorState, InputEvent, Textarea, TextareaState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
};
use navidog_core::object::Scope;
use navidog_core::settings::{ResultsPosition, Settings, ShowAfterRun};
use navidog_db::dialect::{ChangeSet, Dialect, RowKey, Values, Writes};
use navidog_db::sessions::{Sessions, run_on};
use navidog_db::{Column, ResultSet};
use navidog_grid::commit::{Commit, apply};
use navidog_grid::{Cell, GridColumn, GridEvent, GridTarget, ResultGrid};
use navidog_ui::assets::lucide;
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use navidog_util::sql_text;
use rust_i18n::t;

use crate::editor::QueryEditor;
use crate::explain::ExplainView;
use crate::run::Outcome;

/// At most this many result sets get a tab per run; the rest only reach the messages (31 §4.1).
pub(crate) const MAX_RESULTS: usize = 20;

/// When and where a result came from, shown on hover over a pinned tab and on explain tabs.
#[derive(Clone, Debug)]
pub(crate) struct RunInfo {
    pub at: chrono::DateTime<chrono::Local>,
    pub query_time: Duration,
    pub connection: String,
    pub connection_id: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub dialect: Option<Dialect>,
}

impl RunInfo {
    /// Where a result column's table lives: MySQL and SQLite name the database in the origin,
    /// PostgreSQL the schema of the database this editor runs in.
    pub fn scope_for(&self, origin_schema: &str) -> Option<Scope> {
        let conn = self.connection_id.clone();
        Some(match self.dialect? {
            Dialect::Postgres => Scope::new(
                conn,
                self.database.clone()?,
                Some(origin_schema.to_string()),
            ),
            Dialect::MySql | Dialect::Sqlite => Scope::new(conn, origin_schema, None),
        })
    }

    pub fn lines(&self, sql: &str) -> Vec<(String, String)> {
        let mut out = vec![
            (
                t!("query.info_time").to_string(),
                self.at.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
            (
                t!("query.info_query_time").to_string(),
                secs(self.query_time),
            ),
            (
                t!("query.info_connection").to_string(),
                self.connection.clone(),
            ),
        ];
        if let Some(db) = &self.database {
            out.push((t!("query.database").to_string(), db.clone()));
        }
        if let Some(s) = &self.schema {
            out.push((t!("query.schema").to_string(), s.clone()));
        }
        out.push(("SQL".into(), sql.to_string()));
        out
    }
}

pub(crate) fn secs(d: Duration) -> String {
    format!("{:.3}s", d.as_secs_f64())
}

pub(crate) enum Body {
    Grid(Entity<ResultView>),
    Explain(Entity<ExplainView>),
}

pub(crate) struct Tab {
    pub id: u64,
    pub name: String,
    pub pinned: bool,
    pub body: Body,
    pub info: RunInfo,
    pub sql: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Messages,
    Summary,
    Tab(u64),
    Profile,
    Status,
}

pub(crate) struct SummaryRow {
    pub sql: String,
    pub message: String,
    pub elapsed: Duration,
    pub error: bool,
    pub range: Range<usize>,
}

pub(crate) struct Results {
    pub tabs: Vec<Tab>,
    pub page: Page,
    messages: Entity<EditorState>,
    messages_text: String,
    pub rows: Vec<SummaryRow>,
    started: Option<chrono::DateTime<chrono::Local>>,
    ended: Option<chrono::DateTime<chrono::Local>>,
    pub only_errors: bool,
    /// Result and explain tabs made by the current run, for naming and the cap
    results_this_run: usize,
    /// `next_id` at the start of the current run. Tabs above this were created by it.
    run_floor: u64,
    explains_this_run: usize,
    /// MySQL "Show Profile and Status" tabs of the last run
    profile: Option<Entity<ResultGrid>>,
    status: Option<Entity<ResultGrid>>,
    next_id: u64,
    subscriptions: BTreeMap<u64, Subscription>,
}

/// Commands from a result tab's context menu.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct TogglePinTab {
    pub id: u64,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = query, no_json)]
pub(crate) struct CloseResultTab {
    pub id: u64,
}

gpui_kit::actions!(query, [UnpinAllTabs, CloseAllResults, BackToEditor]);

pub(crate) enum ResultEvent {
    TogglePin,
}

impl Results {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let messages = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .line_number(false)
                .soft_wrap(true)
        });
        messages.update(cx, |e, cx| e.set_readonly(true, cx));
        Self {
            tabs: Vec::new(),
            page: Page::Messages,
            messages,
            messages_text: String::new(),
            rows: Vec::new(),
            started: None,
            ended: None,
            only_errors: false,
            results_this_run: 0,
            run_floor: 0,
            explains_this_run: 0,
            profile: None,
            status: None,
            next_id: 0,
            subscriptions: BTreeMap::new(),
        }
    }

    pub fn has_run(&self) -> bool {
        self.started.is_some()
    }

    /// Each run drops the tabs that are not pinned, unless `keep` is set (31 §2).
    pub fn begin_run(&mut self, keep: bool, cx: &mut Context<QueryEditor>) {
        if !keep {
            let dropped: Vec<u64> = self
                .tabs
                .iter()
                .filter(|t| !t.pinned)
                .map(|t| t.id)
                .collect();
            for id in dropped {
                self.subscriptions.remove(&id);
            }
            self.tabs.retain(|t| t.pinned);
        }
        self.run_floor = self.next_id;
        self.rows.clear();
        self.messages_text.clear();
        self.results_this_run = 0;
        self.explains_this_run = 0;
        self.profile = None;
        self.status = None;
        self.started = Some(chrono::Local::now());
        self.ended = None;
        if !matches!(self.page, Page::Tab(id) if self.tabs.iter().any(|t| t.id == id)) {
            self.page = Page::Messages;
        }
        cx.notify();
    }

    pub fn failed_to_start(
        &mut self,
        error: &str,
        window: &mut Window,
        cx: &mut Context<QueryEditor>,
    ) {
        self.messages_text.push_str(&format!("> {error}\n"));
        self.sync_messages(window, cx);
        self.page = Page::Messages;
    }

    fn sync_messages(&mut self, window: &mut Window, cx: &mut Context<QueryEditor>) {
        let text = self.messages_text.clone();
        self.messages
            .update(cx, |e, cx| e.set_value(text, window, cx));
    }

    /// Writes a statement to the messages and summary pages.
    pub fn note(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<QueryEditor>) {
        let Outcome {
            planned,
            result,
            elapsed,
            ..
        } = outcome;
        self.messages_text
            .push_str(&message_block(&planned.shown, &result, elapsed));
        self.sync_messages(window, cx);
        self.rows.push(SummaryRow {
            sql: planned.shown.clone(),
            message: summary_message(&result),
            elapsed,
            error: result.is_err(),
            range: planned.range.clone(),
        });
    }

    pub fn record(
        &mut self,
        outcome: Outcome,
        info: RunInfo,
        editor: WeakEntity<QueryEditor>,
        window: &mut Window,
        cx: &mut Context<QueryEditor>,
    ) {
        let sets = outcome.result.as_ref().ok().cloned();
        let planned = outcome.planned.clone();
        self.note(outcome, window, cx);
        let Some(sets) = sets else { return };
        for set in sets.into_iter().filter(|s| !s.columns.is_empty()) {
            if self.results_this_run >= MAX_RESULTS {
                break;
            }
            self.results_this_run += 1;
            let name = sql_text::name_hint(&planned.shown)
                .unwrap_or_else(|| t!("query.result_n", n = self.results_this_run).to_string());
            let view = ResultView::new(
                set,
                planned.sql.clone(),
                info.clone(),
                editor.clone(),
                window,
                cx,
            );
            let id = self.add_tab(
                name,
                Body::Grid(view.clone()),
                info.clone(),
                planned.sql.clone(),
            );
            let sub = cx.subscribe(
                &view,
                move |this: &mut QueryEditor, _, e: &ResultEvent, cx| match e {
                    ResultEvent::TogglePin => this.results.toggle_pin(id, cx),
                },
            );
            self.subscriptions.insert(id, sub);
        }
    }

    pub fn add_tab(&mut self, name: String, body: Body, info: RunInfo, sql: String) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.tabs.push(Tab {
            id,
            name,
            pinned: false,
            body,
            info,
            sql,
        });
        id
    }

    pub fn next_explain_name(&mut self) -> String {
        self.explains_this_run += 1;
        t!("query.explain_n", n = self.explains_this_run).to_string()
    }

    /// Picks the page "View > Show After Execution" asks for.
    pub fn end_run(&mut self, explain: bool, cx: &mut Context<QueryEditor>) {
        self.ended = Some(chrono::Local::now());
        let first_new = self
            .tabs
            .iter()
            .filter(|t| t.id > self.run_floor)
            .map(|t| t.id)
            .min();
        self.page = match Settings::global(cx).show_after_run {
            ShowAfterRun::Result => first_new.map(Page::Tab).unwrap_or(Page::Messages),
            ShowAfterRun::Messages if !explain => Page::Messages,
            ShowAfterRun::Summary if !explain => Page::Summary,
            _ => first_new.map(Page::Tab).unwrap_or(Page::Messages),
        };
        cx.notify();
    }

    pub fn show_profile(
        &mut self,
        profiled: Option<(ResultSet, Vec<(String, String)>)>,
        window: &mut Window,
        cx: &mut Context<QueryEditor>,
    ) {
        let Some((profile, changes)) = profiled else {
            return;
        };
        let status = ResultSet {
            columns: ["Variable_name", "Value"]
                .iter()
                .map(|n| Column {
                    name: n.to_string(),
                    ..Column::default()
                })
                .collect(),
            rows: changes
                .into_iter()
                .map(|(k, v)| vec![Some(k), Some(v)])
                .collect(),
            ..ResultSet::default()
        };
        self.profile = Some(read_only_grid(&profile, window, cx));
        self.status = Some(read_only_grid(&status, window, cx));
    }

    /// Pinned tabs sit at the far left in the order they were pinned (31 §4.2).
    pub fn toggle_pin(&mut self, id: u64, cx: &mut Context<QueryEditor>) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let mut tab = self.tabs.remove(ix);
        tab.pinned = !tab.pinned;
        let pinned = tab.pinned;
        if let Body::Grid(view) = &tab.body {
            view.update(cx, |v, cx| {
                v.pinned = pinned;
                cx.notify();
            });
        }
        let at = self.tabs.iter().take_while(|t| t.pinned).count();
        self.tabs.insert(at, tab);
        cx.notify();
    }

    pub fn close_tab(&mut self, id: u64, cx: &mut Context<QueryEditor>) {
        self.tabs.retain(|t| t.id != id);
        self.subscriptions.remove(&id);
        if self.page == Page::Tab(id) {
            self.page = Page::Messages;
        }
        cx.notify();
    }

    fn page_exists(&self) -> bool {
        match self.page {
            Page::Tab(id) => self.tabs.iter().any(|t| t.id == id),
            Page::Profile => self.profile.is_some(),
            Page::Status => self.status.is_some(),
            _ => true,
        }
    }
}

fn read_only_grid(set: &ResultSet, window: &mut Window, cx: &mut App) -> Entity<ResultGrid> {
    let grid = cx.new(|cx| ResultGrid::new(window, cx));
    let (columns, rows) = grid_data(set);
    grid.update(cx, |g, cx| {
        g.set_quick_filter(false);
        g.set_data(columns, rows, cx);
        g.set_locked(true, cx);
    });
    grid
}

fn grid_data(set: &ResultSet) -> (Vec<GridColumn>, Vec<Vec<Cell>>) {
    let columns = set
        .columns
        .iter()
        .map(|c| GridColumn {
            name: c.name.clone().into(),
            type_name: c.type_name.clone().into(),
            ..GridColumn::default()
        })
        .collect();
    let rows = set
        .rows
        .iter()
        .map(|r| r.iter().map(|v| v.clone().map(Into::into)).collect())
        .collect();
    (columns, rows)
}

/// One statement on the messages page (31 §4.3).
pub(crate) fn message_block(
    sql: &str,
    result: &navidog_db::DbResult<Vec<ResultSet>>,
    elapsed: Duration,
) -> String {
    let mut out = format!("{}\n", sql.trim());
    match result {
        Ok(sets) => {
            for notice in sets.iter().flat_map(|s| &s.notices) {
                out.push_str(&format!("> {notice}\n"));
            }
            let affected: Option<u64> = sets
                .iter()
                .filter(|s| s.columns.is_empty())
                .filter_map(|s| s.affected_rows)
                .reduce(|a, b| a + b);
            match affected {
                Some(n) if sets.iter().all(|s| s.columns.is_empty()) => {
                    out.push_str(&format!("> {}: {n}\n", t!("query.affected_rows")))
                }
                _ => out.push_str("> OK\n"),
            }
        }
        Err(e) => out.push_str(&format!("> {e}\n")),
    }
    out.push_str(&format!("> {}: {}\n\n", t!("query.time"), secs(elapsed)));
    out
}

fn summary_message(result: &navidog_db::DbResult<Vec<ResultSet>>) -> String {
    match result {
        Ok(sets) if sets.iter().all(|s| s.columns.is_empty()) => {
            let n: u64 = sets.iter().filter_map(|s| s.affected_rows).sum();
            format!("{}: {n}", t!("query.affected_rows"))
        }
        Ok(_) => "OK".into(),
        Err(e) => e.to_string(),
    }
}

/// The table a result can be edited in: a SELECT whose columns all come from one table, each
/// column at most once (31 §4.2). The primary key check needs the server and comes after.
pub(crate) fn single_table(sql: &str, columns: &[Column]) -> Option<(String, String)> {
    if !sql_text::first_keyword(sql).eq_ignore_ascii_case("SELECT") {
        return None;
    }
    let first = columns.first()?.origin.as_ref()?;
    let mut seen = HashSet::new();
    for c in columns {
        let o = c.origin.as_ref()?;
        if o.schema != first.schema || o.table != first.table || !seen.insert(o.column.as_str()) {
            return None;
        }
    }
    Some((first.schema.clone(), first.table.clone()))
}

/// The grid writes changes under the result's column names, which may be aliases; the table
/// knows the source names.
pub(crate) fn rename_changes(changes: ChangeSet, names: &BTreeMap<String, String>) -> ChangeSet {
    let values = |v: Values| -> Values {
        v.into_iter()
            .map(|(c, x)| (names.get(&c).cloned().unwrap_or(c), x))
            .collect()
    };
    let writes = |w: Writes| -> Writes {
        w.into_iter()
            .map(|(c, x)| (names.get(&c).cloned().unwrap_or(c), x))
            .collect()
    };
    ChangeSet {
        updates: changes
            .updates
            .into_iter()
            .map(|u| navidog_db::dialect::RowUpdate {
                key: RowKey(values(u.key.0)),
                values: writes(u.values),
            })
            .collect(),
        inserts: changes.inserts.into_iter().map(writes).collect(),
        deletes: changes
            .deletes
            .into_iter()
            .map(|k| RowKey(values(k.0)))
            .collect(),
    }
}

/// Sorts rows by one column: NULL first, numbers by value, the rest as text.
pub(crate) fn sort_rows(rows: &mut [Vec<Option<String>>], col: usize, asc: bool) {
    rows.sort_by(|a, b| {
        let (x, y) = (a.get(col).cloned().flatten(), b.get(col).cloned().flatten());
        let ord = match (x, y) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
                _ => x.cmp(&y),
            },
        };
        if asc { ord } else { ord.reverse() }
    });
}

/// Where an editable result writes.
struct Editing {
    dialect: Dialect,
    scope: Scope,
    table: String,
    /// Result column name → table column name
    names: BTreeMap<String, String>,
    /// Primary key columns of the table
    pk: HashSet<String>,
}

impl Editing {
    fn is_pk(&self, result_column: &str) -> bool {
        self.names
            .get(result_column)
            .is_some_and(|c| self.pk.contains(c))
    }
}

/// One result tab: the grid, the bottom bar and the edit and commit flow.
pub(crate) struct ResultView {
    pub grid: Entity<ResultGrid>,
    pub sql: String,
    columns: Vec<Column>,
    pub(crate) rows: Vec<Vec<Option<String>>>,
    edit: Option<Editing>,
    info: RunInfo,
    editor: WeakEntity<QueryEditor>,
    pub pinned: bool,
    busy: bool,
    status: SharedString,
    /// The cell editor under the grid (F-DATA-011 style), and the cell it shows
    cell_editor: bool,
    cell_input: Entity<TextareaState>,
    cell_at: Option<(usize, usize)>,
    /// Form view of the current row instead of the grid
    form: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ResultEvent> for ResultView {}

impl ResultView {
    pub fn new(
        set: ResultSet,
        sql: String,
        info: RunInfo,
        editor: WeakEntity<QueryEditor>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let grid = cx.new(|cx| {
            let mut grid = ResultGrid::new(window, cx);
            grid.set_quick_filter(false);
            grid
        });
        let cell_input = cx.new(|cx| TextareaState::new(window, cx));
        cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.subscribe_in(&grid, window, Self::on_grid_event),
                cx.subscribe(&cell_input, |this, input, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change) {
                        let text = input.read(cx).value().to_string();
                        this.write_cell(text, cx);
                    }
                }),
            ];
            let mut this = Self {
                grid,
                sql,
                columns: set.columns,
                rows: set.rows,
                edit: None,
                info,
                editor,
                pinned: false,
                busy: false,
                status: SharedString::default(),
                cell_editor: false,
                cell_input,
                cell_at: None,
                form: false,
                _subscriptions: subs,
            };
            this.show_rows(cx);
            this.grid.update(cx, |g, cx| g.set_locked(true, cx));
            this.find_table(cx);
            this
        })
    }

    /// Keeps the cell editor on the focused cell (needs `window`, so called from render).
    fn sync_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cell_editor {
            return;
        }
        let model = self.grid.read(cx).model(cx);
        if model.row_count() == 0 {
            return;
        }
        let at = model.head();
        if self.cell_at == Some(at) {
            return;
        }
        let text = model
            .value(at.0, at.1)
            .cloned()
            .flatten()
            .map(|v| v.to_string())
            .unwrap_or_default();
        self.cell_at = Some(at);
        self.cell_input
            .update(cx, |s, cx| s.set_value(text, window, cx));
    }

    /// Typing in the cell editor edits the cell, when the result is editable.
    fn write_cell(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some((r, c)), true) = (self.cell_at, self.edit.is_some() && !self.busy) else {
            return;
        };
        let current = self.grid.read(cx).model(cx).value(r, c).cloned().flatten();
        if current.as_deref() == Some(text.as_str()) {
            return;
        }
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| m.set_value(r, c, Some(text.into())))
        });
    }

    /// The current row as field / value pairs, with previous and next.
    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let model = self.grid.read(cx).model(cx);
        let rows = model.row_count();
        let row = model.head().0.min(rows.saturating_sub(1));
        let fields: Vec<(SharedString, String)> = model
            .columns
            .iter()
            .enumerate()
            .map(|(c, col)| {
                let v = model
                    .value(row, c)
                    .cloned()
                    .flatten()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "(NULL)".into());
                (col.name.clone(), v)
            })
            .collect();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(tokens::result::BOTTOM_BAR_HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        Button::new("form-prev")
                            .ghost()
                            .xsmall()
                            .icon(lucide("chevron-left"))
                            .disabled(row == 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.grid
                                    .update(cx, |g, cx| g.move_to_row(row.saturating_sub(1), cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("form-next")
                            .ghost()
                            .xsmall()
                            .icon(lucide("chevron-right"))
                            .disabled(row + 1 >= rows)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.grid.update(cx, |g, cx| g.move_to_row(row + 1, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.label_secondary)
                            .child(format!("{} / {rows}", (row + 1).min(rows))),
                    ),
            )
            .child(
                v_flex()
                    .id("result-form")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_1()
                    .children(fields.into_iter().enumerate().map(|(ix, (name, value))| {
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                truncated(("form-field", ix), name)
                                    .w(tokens::list::DETAIL_COLUMN_WIDTH.rems())
                                    .flex_none()
                                    .text_color(p.label_secondary),
                            )
                            .child(truncated(("form-value", ix), value).flex_1())
                    })),
            )
            .into_any_element()
    }

    #[cfg(test)]
    pub(crate) fn editable(&self) -> bool {
        self.edit.is_some()
    }

    #[cfg(test)]
    pub(crate) fn commit_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(window, cx);
    }

    fn show_rows(&mut self, cx: &mut Context<Self>) {
        let set = ResultSet {
            columns: self.columns.clone(),
            rows: self.rows.clone(),
            ..ResultSet::default()
        };
        let (mut columns, rows) = grid_data(&set);
        if let Some(e) = &self.edit {
            for c in &mut columns {
                c.primary_key = e.is_pk(&c.name);
            }
        }
        self.grid.update(cx, |g, cx| g.set_data(columns, rows, cx));
    }

    /// Looks up the primary key of the result's table; with all of it in the result, the result
    /// becomes editable.
    fn find_table(&mut self, cx: &mut Context<Self>) {
        let Some(dialect) = self.info.dialect else {
            return;
        };
        let Some((schema, table)) = single_table(&self.sql, &self.columns) else {
            return;
        };
        let Some(scope) = self.info.scope_for(&schema) else {
            return;
        };
        let (s, t) = (scope.clone(), table.clone());
        let Some(job) = Sessions::global(cx).read(cx).run_rel(
            &scope.connection,
            Some(&scope.database),
            move |rel| Box::pin(async move { rel.table_columns(&s, &t).await }),
        ) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(Ok(columns)) = job.await else { return };
            let _ = this.update(cx, |this, cx| {
                let pk: Vec<&str> = columns
                    .iter()
                    .filter(|c| c.primary_key)
                    .map(|c| c.name.as_str())
                    .collect();
                let names: BTreeMap<String, String> = this
                    .columns
                    .iter()
                    .filter_map(|c| Some((c.name.clone(), c.origin.as_ref()?.column.clone())))
                    .collect();
                let all_pk_present =
                    !pk.is_empty() && pk.iter().all(|k| names.values().any(|v| v == k));
                if !all_pk_present {
                    return;
                }
                this.edit = Some(Editing {
                    dialect,
                    scope: scope.clone(),
                    table: table.clone(),
                    names,
                    pk: pk.iter().map(|k| k.to_string()).collect(),
                });
                this.show_rows(cx);
                this.grid.update(cx, |g, cx| {
                    g.set_target(
                        Some(GridTarget {
                            dialect,
                            scope,
                            table,
                        }),
                        cx,
                    );
                    g.set_locked(false, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn on_grid_event(
        &mut self,
        _: &Entity<ResultGrid>,
        e: &GridEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            GridEvent::SortChanged(sort) => {
                if self.grid.read(cx).model(cx).has_changes() {
                    return;
                }
                if let Some((name, asc)) = sort
                    && let Some(col) = self.columns.iter().position(|c| c.name == name.as_ref())
                {
                    sort_rows(&mut self.rows, col, *asc);
                    self.show_rows(cx);
                }
            }
            GridEvent::CommitRequested => self.commit(window, cx),
            GridEvent::Changed => cx.notify(),
            _ => {}
        }
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(edit) = &self.edit else { return };
        self.grid.update(cx, |g, cx| g.commit_edit(window, cx));
        let model = self.grid.read(cx).model(cx);
        if !model.has_changes() {
            return;
        }
        let changes = rename_changes(model.change_set(), &edit.names);
        let statements = edit
            .dialect
            .render_changes(&edit.scope, &edit.table, &changes, true);
        if statements.is_empty() {
            return;
        }
        self.status = statements.join(";\n").into();
        let commit = Commit::new(edit.dialect, statements);
        // On the connection's metadata session, outside any transaction opened in the editor
        // (31 §4.2, a recorded deviation)
        let Some(job) = Sessions::global(cx).read(cx).run_rel(
            &edit.scope.connection,
            Some(&edit.scope.database),
            move |rel| Box::pin(apply(rel, commit, false, false)),
        ) else {
            return;
        };
        self.set_busy(true, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_busy(false, cx);
                match result {
                    Ok(()) => {
                        let sync = Settings::global(cx).sync_record;
                        let unknown = this.grid.read(cx).model(cx).commit_needs_reload();
                        if sync || unknown {
                            this.refresh(window, cx);
                        } else {
                            this.grid
                                .update(cx, |g, cx| g.update_model(cx, |m| m.accept_committed()));
                        }
                    }
                    Err(e) => {
                        this.status = e.clone().into();
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            alert
                                .icon(lucide("circle-x"))
                                .title(t!("query.commit_failed").to_string())
                                .description(e.clone())
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs the SELECT again on the editor's session and shows the fresh rows.
    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self
            .editor
            .upgrade()
            .and_then(|e| e.read(cx).live.as_ref().map(|l| l.session.clone()))
        else {
            return;
        };
        let sql = self.sql.clone();
        let job = run_on(session, move |rel| {
            Box::pin(async move { rel.execute_statement(&sql).await })
        });
        self.set_busy(true, cx);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.set_busy(false, cx);
                if let Ok(Ok(sets)) = result
                    && let Some(set) = sets.into_iter().find(|s| !s.columns.is_empty())
                {
                    this.rows = set.rows;
                    this.show_rows(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        let lock = busy || self.edit.is_none();
        self.grid.update(cx, |g, cx| g.set_locked(lock, cx));
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let editable = self.edit.is_some() && !self.busy;
        let dirty = self.grid.read(cx).model(cx).has_changes();
        let icon_button = |id: &'static str, icon: &'static str, tip: String| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(lucide(icon))
                .tooltip(tip)
                .w(tokens::result::BOTTOM_BUTTON.rems())
                .h(tokens::result::BOTTOM_BUTTON.rems())
        };
        let sql: SharedString = if self.status.is_empty() {
            self.sql.clone().into()
        } else {
            self.status.clone()
        };
        h_flex()
            .h(tokens::result::BOTTOM_BAR_HEIGHT.rems())
            .min_w_0()
            .flex_none()
            .px_2()
            .gap_1()
            .border_t_1()
            .border_color(p.separator)
            .text_xs()
            .child(
                icon_button(
                    "result-cell-editor",
                    "code",
                    t!("query.cell_editor").to_string(),
                )
                .disabled(true),
            )
            .child(
                icon_button(
                    "result-analysis",
                    "chart-pie",
                    t!("query.data_analysis").to_string(),
                )
                .disabled(true),
            )
            .child(
                icon_button("result-export", "file-text", t!("query.export").to_string())
                    .disabled(true),
            )
            .child(
                icon_button("result-pin", "pin", t!("query.pin").to_string())
                    .selected(self.pinned)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ResultEvent::TogglePin))),
            )
            .child(
                truncated("result-sql", sql)
                    .flex_1()
                    .px_2()
                    .text_color(p.label_secondary),
            )
            .when(self.edit.is_none(), |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .text_color(p.label_secondary)
                        .child(t!("query.read_only").to_string()),
                )
            })
            .child(
                icon_button("result-add", "plus", t!("query.add_row").to_string())
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid.update(cx, |g, cx| {
                            let mut row = 0;
                            g.update_model(cx, |m| row = m.insert_row());
                            g.scroll_to_row(row, cx);
                            g.settle(cx);
                        });
                    })),
            )
            .child(
                icon_button("result-delete", "minus", t!("query.delete_row").to_string())
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid
                            .update(cx, |g, cx| g.update_model(cx, |m| m.delete_selected_rows()));
                    })),
            )
            .child(
                icon_button("result-apply", "check", t!("query.apply").to_string())
                    .disabled(!editable || !dirty)
                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
            )
            .child(
                icon_button("result-discard", "x", t!("query.discard").to_string())
                    .disabled(!dirty || self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid
                            .update(cx, |g, cx| g.update_model(cx, |m| m.discard()));
                    })),
            )
    }
}

impl Render for ResultView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_cell(window, cx);
        let p = palette(cx);
        let body: AnyElement = if self.form {
            self.render_form(cx)
        } else {
            self.grid.clone().into_any_element()
        };
        let read_only = self.edit.is_none();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
            .when(self.cell_editor, |v| {
                v.child(
                    div()
                        .h(tokens::viewer::CELL_EDITOR_HEIGHT.rems())
                        .border_t_1()
                        .border_color(p.separator)
                        .child(
                            Textarea::new(&self.cell_input)
                                .disabled(read_only)
                                .size_full(),
                        ),
                )
            })
            .child(self.render_bottom_bar(cx))
    }
}

impl QueryEditor {
    fn tab_button(
        &self,
        id: SharedString,
        label: String,
        page: Page,
        pinned: bool,
        close: Option<u64>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = palette(cx);
        let active = self.results.page == page;
        div()
            .id(id.clone())
            .flex_none()
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .border_b_2()
            .border_color(if active {
                p.accent
            } else {
                gpui_kit::transparent_black()
            })
            .text_color(if active { p.label } else { p.label_secondary })
            .when(pinned, |d| {
                d.child(lucide("pin").size(tokens::navigator::ROW_ICON.rems()))
            })
            .child(
                truncated(SharedString::from(format!("{id}-label")), label)
                    .max_w(tokens::editor_tab::TITLE_MAX_WIDTH.rems()),
            )
            .when_some(close, |d, tab_id| {
                d.child(
                    Button::new(SharedString::from(format!("result-close-{tab_id}")))
                        .ghost()
                        .xsmall()
                        .icon(lucide("x"))
                        .tooltip(t!("query.close").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.results.close_tab(tab_id, cx);
                        })),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.results.page = page;
                cx.notify();
            }))
    }

    pub(crate) fn render_results(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = palette(cx);
        if !self.results.page_exists() {
            self.results.page = Page::Messages;
        }
        let mut strip: Vec<AnyElement> = Vec::new();
        let tab_el = |this: &Self, t: &Tab, cx: &mut Context<Self>| {
            let id = t.id;
            let lines = t.info.lines(&t.sql);
            let pinned = t.pinned;
            let close = Settings::global(cx).keep_results.then_some(id);
            this.tab_button(
                SharedString::from(format!("result-tab-{id}")),
                t.name.clone(),
                Page::Tab(id),
                pinned,
                close,
                cx,
            )
            .when(pinned, |d| {
                d.tooltip(move |window, cx| {
                    let text = lines
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    gpui_kit::component::tooltip::Tooltip::new(text).build(window, cx)
                })
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui_kit::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.focus_for_menu(window, cx);
                    let pinned = this.results.tabs.iter().any(|t| t.id == id && t.pinned);
                    NativeMenu::new()
                        .menu(
                            if pinned {
                                t!("query.unpin").to_string()
                            } else {
                                t!("query.pin").to_string()
                            },
                            Box::new(TogglePinTab { id }),
                        )
                        .menu(
                            t!("query.close").to_string(),
                            Box::new(CloseResultTab { id }),
                        )
                        .separator()
                        .menu(t!("query.unpin_all").to_string(), Box::new(UnpinAllTabs))
                        .menu(
                            t!("query.close_all_results").to_string(),
                            Box::new(CloseAllResults),
                        )
                        .show(e.position, window, cx);
                }),
            )
            .into_any_element()
        };
        let tabs: Vec<(bool, usize)> = self
            .results
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| (t.pinned, i))
            .collect();
        for (pinned, i) in &tabs {
            if *pinned {
                let t = &self.results.tabs[*i];
                strip.push(tab_el(self, t, cx));
            }
        }
        strip.push(
            self.tab_button(
                "result-messages".into(),
                t!("query.messages").to_string(),
                Page::Messages,
                false,
                None,
                cx,
            )
            .into_any_element(),
        );
        strip.push(
            self.tab_button(
                "result-summary".into(),
                t!("query.summary").to_string(),
                Page::Summary,
                false,
                None,
                cx,
            )
            .into_any_element(),
        );
        for (pinned, i) in &tabs {
            if !*pinned {
                let t = &self.results.tabs[*i];
                strip.push(tab_el(self, t, cx));
            }
        }
        if self.results.profile.is_some() {
            strip.push(
                self.tab_button(
                    "result-profile".into(),
                    t!("query.profile").to_string(),
                    Page::Profile,
                    false,
                    None,
                    cx,
                )
                .into_any_element(),
            );
            strip.push(
                self.tab_button(
                    "result-status".into(),
                    t!("query.status").to_string(),
                    Page::Status,
                    false,
                    None,
                    cx,
                )
                .into_any_element(),
            );
        }
        let position = Settings::global(cx).results_position;
        let pos_button = |id: &'static str, icon: &'static str, tip: String, on: bool| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(lucide(icon))
                .tooltip(tip)
                .selected(on)
        };
        let body: AnyElement = match self.results.page {
            Page::Messages => div()
                .size_full()
                .child(
                    Editor::new(&self.results.messages)
                        .bordered(false)
                        .size_full(),
                )
                .into_any_element(),
            Page::Summary => self.render_summary(cx).into_any_element(),
            Page::Tab(id) => match self
                .results
                .tabs
                .iter()
                .find(|t| t.id == id)
                .map(|t| &t.body)
            {
                Some(Body::Grid(view)) => view.clone().into_any_element(),
                Some(Body::Explain(view)) => view.clone().into_any_element(),
                None => div().into_any_element(),
            },
            Page::Profile => self
                .results
                .profile
                .clone()
                .map(|g| g.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            Page::Status => self
                .results
                .status
                .clone()
                .map(|g| g.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        v_flex()
            .id("query-results")
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(tokens::result::TAB_HEIGHT.rems())
                    .border_b_1()
                    .border_t_1()
                    .border_color(p.separator)
                    .child(
                        div()
                            .id("result-strip")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .overflow_x_scroll()
                            .children(strip),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .px_1()
                            .gap_0p5()
                            .when(position == ResultsPosition::NewPage, |r| {
                                r.child(
                                    pos_button(
                                        "result-back",
                                        "pencil",
                                        t!("query.back_to_editor").to_string(),
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.back_to_editor(window, cx),
                                    )),
                                )
                            })
                            .child(
                                pos_button(
                                    "result-below",
                                    "panel-bottom",
                                    t!("query.results_below").to_string(),
                                    position == ResultsPosition::Below,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.place_results(ResultsPosition::Below, window, cx)
                                    },
                                )),
                            )
                            .child(
                                pos_button(
                                    "result-beside",
                                    "panel-right",
                                    t!("query.results_beside").to_string(),
                                    position == ResultsPosition::Beside,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.place_results(ResultsPosition::Beside, window, cx)
                                    },
                                )),
                            )
                            .child(
                                pos_button(
                                    "result-new-page",
                                    "square",
                                    t!("query.results_new_page").to_string(),
                                    position == ResultsPosition::NewPage,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.place_results(ResultsPosition::NewPage, window, cx)
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
    }

    fn render_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let r = &self.results;
        let none = || t!("query.none").to_string();
        let errors = r.rows.iter().filter(|x| x.error).count();
        let fmt = |t: &Option<chrono::DateTime<chrono::Local>>| {
            t.map(|t| t.format("%H:%M:%S%.3f").to_string())
                .unwrap_or_else(none)
        };
        let ran = r.started.is_some();
        let elapsed = match (r.started, r.ended) {
            (Some(a), Some(b)) => secs((b - a).to_std().unwrap_or_default()),
            _ => none(),
        };
        let stat = |label: String, value: String| {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .text_color(p.label_secondary)
                        .child(format!("{label}:")),
                )
                .child(div().child(value))
        };
        let count = |n: usize| if ran { n.to_string() } else { none() };
        let rows: Vec<(usize, &SummaryRow)> = r
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !r.only_errors || row.error)
            .collect();
        let header_cell = |text: String| {
            div()
                .px_2()
                .truncate()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(text)
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(tokens::result::SUMMARY_HEIGHT.rems())
                    .px_4()
                    .gap_8()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(stat(t!("query.processed").to_string(), count(r.rows.len())))
                            .child(stat(
                                t!("query.succeeded").to_string(),
                                count(r.rows.len() - errors),
                            ))
                            .child(stat(t!("query.errors").to_string(), count(errors))),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(stat(t!("query.start_time").to_string(), fmt(&r.started)))
                            .child(stat(t!("query.end_time").to_string(), fmt(&r.ended)))
                            .child(stat(t!("query.elapsed").to_string(), elapsed)),
                    )
                    .child(div().flex_1())
                    .child(
                        Checkbox::new("summary-only-errors")
                            .label(t!("query.only_errors").to_string())
                            .checked(r.only_errors)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.results.only_errors = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .border_b_1()
                    .border_color(p.separator)
                    .child(header_cell(t!("query.query").to_string()).flex_1())
                    .child(header_cell(t!("query.message").to_string()).flex_1())
                    .child(
                        header_cell(t!("query.elapsed").to_string())
                            .w(tokens::list::DETAIL_COLUMN_WIDTH.rems()),
                    ),
            )
            .child(
                v_flex()
                    .id("summary-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|(ix, row)| {
                        let range = row.range.clone();
                        h_flex()
                            .id(("summary-row", ix))
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .when(ix % 2 == 1, |d| d.bg(p.row_alt))
                            .when(row.error, |d| d.text_color(p.danger))
                            .child(
                                truncated(("summary-sql", ix), row.sql.replace('\n', " "))
                                    .flex_1()
                                    .px_2(),
                            )
                            .child(
                                truncated(("summary-msg", ix), row.message.clone())
                                    .flex_1()
                                    .px_2(),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .w(tokens::list::DETAIL_COLUMN_WIDTH.rems())
                                    .child(secs(row.elapsed)),
                            )
                            .on_click(cx.listener(
                                move |this, e: &gpui_kit::ClickEvent, window, cx| {
                                    if e.click_count() == 2 {
                                        this.select_range(range.clone(), window, cx);
                                    }
                                },
                            ))
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_db::ColumnOrigin;
    use navidog_db::dialect::{RowUpdate, Written};

    fn col(name: &str, table: Option<(&str, &str)>) -> Column {
        Column {
            name: name.into(),
            type_name: String::new(),
            origin: table.map(|(t, c)| ColumnOrigin {
                schema: "s".into(),
                table: t.into(),
                column: c.into(),
            }),
        }
    }

    #[test]
    fn single_table_needs_a_select_from_one_table() {
        let cols = [
            col("id", Some(("actor", "actor_id"))),
            col("n", Some(("actor", "first_name"))),
        ];
        assert_eq!(
            single_table("-- x\nselect * from actor", &cols),
            Some(("s".into(), "actor".into()))
        );
        assert_eq!(single_table("CALL p()", &cols), None);
        let joined = [
            col("id", Some(("actor", "actor_id"))),
            col("t", Some(("film", "title"))),
        ];
        assert_eq!(single_table("SELECT 1", &joined), None);
        let expr = [col("id", Some(("actor", "actor_id"))), col("x", None)];
        assert_eq!(single_table("SELECT 1", &expr), None);
        let twice = [
            col("a", Some(("actor", "actor_id"))),
            col("b", Some(("actor", "actor_id"))),
        ];
        assert_eq!(single_table("SELECT 1", &twice), None);
    }

    #[test]
    fn aliases_become_table_columns() {
        let names = BTreeMap::from([("n".to_string(), "first_name".to_string())]);
        let changes = ChangeSet {
            updates: vec![RowUpdate {
                key: RowKey(vec![("actor_id".into(), Some("1".into()))]),
                values: vec![("n".into(), Written::text(Some("A".into())))],
            }],
            ..ChangeSet::default()
        };
        let out = rename_changes(changes, &names);
        assert_eq!(out.updates[0].values[0].0, "first_name");
        assert_eq!(out.updates[0].key.0[0].0, "actor_id");
    }

    #[test]
    fn messages_follow_navicat() {
        let ok: navidog_db::DbResult<Vec<ResultSet>> = Ok(vec![ResultSet {
            columns: vec![col("a", None)],
            notices: vec!["NOTICE: hi".into()],
            ..ResultSet::default()
        }]);
        assert_eq!(
            message_block("SELECT 1", &ok, Duration::from_millis(2)),
            format!(
                "SELECT 1\n> NOTICE: hi\n> OK\n> {}: 0.002s\n\n",
                t!("query.time")
            )
        );
        let upd: navidog_db::DbResult<Vec<ResultSet>> = Ok(vec![ResultSet {
            affected_rows: Some(3),
            ..ResultSet::default()
        }]);
        assert!(message_block("UPDATE t", &upd, Duration::ZERO).contains(": 3\n"));
        let err: navidog_db::DbResult<Vec<ResultSet>> =
            Err(navidog_db::DbError::coded(1146, "Table 'x' doesn't exist"));
        assert!(
            message_block("SELECT", &err, Duration::ZERO)
                .contains("> 1146 - Table 'x' doesn't exist\n")
        );
    }

    #[test]
    fn local_sort_puts_null_first_and_numbers_by_value() {
        let mut rows = vec![
            vec![Some("10".to_string())],
            vec![None],
            vec![Some("9".to_string())],
        ];
        sort_rows(&mut rows, 0, true);
        assert_eq!(
            rows,
            vec![vec![None], vec![Some("9".into())], vec![Some("10".into())]]
        );
        sort_rows(&mut rows, 0, false);
        assert_eq!(rows[0], vec![Some("10".to_string())]);
    }

    #[test]
    fn pinned_hover_lists_when_and_where() {
        let info = RunInfo {
            at: chrono::Local::now(),
            query_time: Duration::from_millis(1500),
            connection: "local".into(),
            connection_id: "c1".into(),
            database: Some("sakila".into()),
            schema: None,
            dialect: Some(Dialect::MySql),
        };
        let lines = info.lines("SELECT 1");
        assert!(lines.iter().any(|(_, v)| v == "local"));
        assert!(lines.iter().any(|(_, v)| v == "sakila"));
        assert!(lines.iter().any(|(_, v)| v == "SELECT 1"));
        assert!(lines.iter().any(|(_, v)| v == "1.500s"));
    }
}
