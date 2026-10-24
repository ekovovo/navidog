//! Console (31-query-results.md §7, F-QRY-014): a mysql / psql style prompt on its own session.
//! Enter runs the input once it ends with the statement terminator; ↑ and ↓ walk the history.

use std::time::Instant;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Editor, EditorState, Enter, MoveDown, MoveUp};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Styled as _,
    Subscription, Task, WeakEntity, Window, div,
};
use navidog_core::store::ConnectionStore;
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{Sessions, SessionsEvent, SharedSession, run_on};
use navidog_db::{CancelHandle, DbError, ResultSet};
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tabs, tokens};
use navidog_util::console as fmt;
use navidog_util::sql_type::{TypeKind, type_kind};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::editor::Stop;
use crate::run::RefreshAfterRun;

pub(crate) const PANEL: &str = "Console";
const CONTEXT: &str = "Console";
/// Output lines kept. ponytail: older output is dropped; a "save output" would keep it.
const MAX_LINES: usize = 10_000;

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-.", Stop, Some(CONTEXT)),
        KeyBinding::new("cmd-.", Stop, Some("Console > Input")),
    ]);
    register_panel(cx, PANEL, |ctx, window, cx| {
        let target = match ctx.info() {
            PanelInfo::Panel(v) => serde_json::from_value::<Dumped>(v.clone()).ok(),
            _ => None,
        };
        let target = target.map(|d| d.console).unwrap_or_default();
        panel_handle(Console::new(target.connection, target.database, window, cx))
    });
}

#[derive(Default, Serialize, Deserialize)]
struct Target {
    connection: String,
    database: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Dumped {
    console: Target,
}

/// Whether the input is a complete statement: it ends with the terminator.
pub(crate) fn complete(input: &str, delimiter: &str) -> bool {
    input.trim_end().ends_with(delimiter)
}

/// The prompt of the first line, or of continuation lines.
pub(crate) fn prompt(
    dialect: Dialect,
    database: &str,
    superuser: bool,
    continuation: bool,
) -> String {
    match (dialect, continuation) {
        (Dialect::MySql, false) => "mysql>".into(),
        (Dialect::MySql, true) => "    ->".into(),
        (Dialect::Sqlite, false) => "sqlite>".into(),
        (Dialect::Sqlite, true) => "   ...>".into(),
        (Dialect::Postgres, cont) => {
            let mark = if superuser { '#' } else { '>' };
            format!("{database}{}{mark}", if cont { '-' } else { '=' })
        }
    }
}

/// Output for one statement in the client's style.
pub(crate) fn format_result(
    dialect: Dialect,
    result: &navidog_db::DbResult<Vec<ResultSet>>,
    secs: f64,
) -> String {
    let mut out = String::new();
    match result {
        Ok(sets) => {
            for set in sets {
                if set.columns.is_empty() {
                    out.push_str(&fmt::mysql_ok(set.affected_rows.unwrap_or(0), secs));
                    out.push('\n');
                    continue;
                }
                let names: Vec<String> = set.columns.iter().map(|c| c.name.clone()).collect();
                let numeric: Vec<bool> = set
                    .columns
                    .iter()
                    .map(|c| type_kind(&c.type_name) == TypeKind::Number)
                    .collect();
                if dialect == Dialect::Postgres {
                    out.push_str(&fmt::psql_table(&names, &set.rows, &numeric));
                    out.push_str(&fmt::psql_rows_footer(set.rows.len()));
                } else {
                    if !set.rows.is_empty() {
                        out.push_str(&fmt::mysql_table(&names, &set.rows, &numeric));
                    }
                    out.push_str(&fmt::mysql_rows_footer(set.rows.len(), secs));
                }
                out.push('\n');
            }
            for set in sets {
                for n in &set.notices {
                    out.push_str(&format!("{n}\n"));
                }
            }
        }
        Err(e) if e.is_session_lost() => {
            out.push_str(&format!("ERROR: {}\n", crate::errors::error_message(e)));
        }
        Err(e) => match (dialect, &e.code) {
            (Dialect::Postgres, _) => out.push_str(&format!("ERROR:  {}\n", e.message)),
            (_, Some(code)) => out.push_str(&format!("ERROR {code}: {}\n", e.message)),
            (_, None) => out.push_str(&format!("ERROR: {}\n", e.message)),
        },
    }
    out.push('\n');
    out
}

pub struct Console {
    focus_handle: FocusHandle,
    connection: String,
    database: Option<String>,
    output: Entity<EditorState>,
    lines: Vec<String>,
    input: Entity<EditorState>,
    history: Vec<String>,
    /// Position while walking the history with ↑ / ↓
    history_ix: Option<usize>,
    session: Option<SharedSession>,
    cancel: Option<CancelHandle>,
    dialect: Option<Dialect>,
    superuser: bool,
    delimiter: String,
    running: bool,
    window: Option<AnyWindowHandle>,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for Console {}

impl Focusable for Console {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl BasePanel for Console {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(
                serde_json::to_value(Dumped {
                    console: Target {
                        connection: self.connection.clone(),
                        database: self.database.clone(),
                    },
                })
                .unwrap_or_default(),
            ),
        }
    }
    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tabs::added(PanelId::from(cx.entity_id()), group, cx);
        self.window = Some(window.window_handle());
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if tabs::removed(PanelId::from(cx.entity_id()), cx) {
            return;
        }
        if let Some(s) = self.session.take() {
            navidog_grid::commit::close_session(s);
        }
    }
}

impl Panel for Console {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conn = ConnectionStore::global(cx)
            .read(cx)
            .get(&self.connection)
            .cloned();
        let name = conn.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let color = conn
            .and_then(|c| c.color)
            .and_then(|c| tokens::connection_color(&c, cx));
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("square-terminal")),
            t!("query.console_title", name = name).to_string(),
            color,
            false,
        )
    }
}

impl Console {
    pub fn new(
        connection: String,
        database: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let output = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("text")
                .line_number(false)
        });
        output.update(cx, |e, cx| e.set_readonly(true, cx));
        let input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(false)
                .auto_close(false)
        });
        cx.new(|cx: &mut Context<Self>| {
            let subs = vec![cx.subscribe_in(
                &Sessions::global(cx),
                window,
                |this, _, e: &SessionsEvent, window, cx| match e {
                    SessionsEvent::Opened(id)
                        if *id == this.connection && this.session.is_none() =>
                    {
                        this.connect(window, cx)
                    }
                    // Bound to its session: it closes with the connection, like a table viewer
                    SessionsEvent::Closed(id) if *id == this.connection => this.close_tab(cx),
                    SessionsEvent::Reopening(id) if *id == this.connection => {
                        if let Some(s) = this.session.take() {
                            navidog_grid::commit::close_session(s);
                        }
                    }
                    _ => {}
                },
            )];
            let mut this = Self {
                focus_handle: cx.focus_handle(),
                connection,
                database,
                output,
                lines: Vec::new(),
                input,
                history: Vec::new(),
                history_ix: None,
                session: None,
                cancel: None,
                dialect: None,
                superuser: false,
                delimiter: ";".into(),
                running: false,
                window: None,
                task: None,
                _subscriptions: subs,
            };
            this.connect(window, cx);
            this
        })
    }

    fn connect(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let sessions = Sessions::global(cx).read(cx);
        if !sessions.state(&self.connection).is_open() {
            return;
        }
        self.dialect = sessions.dialect(&self.connection);
        let Some(job) = sessions.new_session(&self.connection, self.database.as_deref()) else {
            return;
        };
        let (dialect, database) = (self.dialect, self.database.clone());
        self.task = Some(cx.spawn(async move |this, cx| {
            let session = match job.await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.append(format!("ERROR: {}\n", crate::errors::error_message(&e)), cx)
                    });
                    return;
                }
                Err(e) => {
                    let _ = this.update(cx, |this, cx| this.append(format!("ERROR: {e}\n"), cx));
                    return;
                }
            };
            let cancel = session.cancel_handle();
            let shared: SharedSession = std::sync::Arc::new(tokio::sync::Mutex::new(Some(session)));
            // MySQL picks the database with USE; PostgreSQL asks whether the user is a superuser
            let setup = run_on(shared.clone(), move |rel| {
                Box::pin(async move {
                    match (dialect, database) {
                        (Some(Dialect::MySql), Some(db)) => {
                            rel.execute(&format!("USE {}", Dialect::MySql.quote_ident(&db)))
                                .await?;
                            Ok(false)
                        }
                        (Some(Dialect::Postgres), _) => {
                            let sets = rel
                                .execute(
                                    "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
                                )
                                .await?;
                            let su = sets
                                .first()
                                .and_then(|s| s.rows.first())
                                .and_then(|r| r.first().cloned().flatten());
                            Ok(su.as_deref() == Some("t"))
                        }
                        _ => Ok::<_, DbError>(false),
                    }
                })
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.session = Some(shared);
                this.cancel = cancel;
                if let Ok(Ok(su)) = setup {
                    this.superuser = su;
                }
                cx.notify();
            });
        }));
    }

    fn close_tab(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.window else { return };
        let panel = PanelId::from(cx.entity_id());
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let close = tabs::CloseTabs {
                    panel: Some(panel),
                    which: tabs::Close::This,
                };
                window.dispatch_action(Box::new(close), cx);
            });
        });
    }

    fn database_name(&self) -> String {
        self.database.clone().unwrap_or_else(|| "postgres".into())
    }

    fn prompt(&self, continuation: bool) -> String {
        prompt(
            self.dialect.unwrap_or(Dialect::MySql),
            &self.database_name(),
            self.superuser,
            continuation,
        )
    }

    fn append(&mut self, text: String, cx: &mut Context<Self>) {
        self.lines.extend(text.lines().map(String::from));
        if self.lines.len() > MAX_LINES {
            let extra = self.lines.len() - MAX_LINES;
            self.lines.drain(..extra);
        }
        let all = self.lines.join("\n");
        let Some(handle) = self.window else { return };
        let output = self.output.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                output.update(cx, |e, cx| {
                    e.set_value(all, window, cx);
                    crate::history::scroll_to_end(e, cx);
                });
            });
        });
        cx.notify();
    }

    /// Enter: runs a complete statement; otherwise the newline goes into the input.
    fn on_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.input.read(cx).value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return true;
        }
        if matches!(
            trimmed.trim_end_matches(';').to_ascii_lowercase().as_str(),
            "exit" | "quit"
        ) {
            self.close_tab(cx);
            return true;
        }
        let dialect = self.dialect.unwrap_or(Dialect::MySql);
        if dialect == Dialect::MySql
            && let Some(rest) = navidog_util::sql_text::strip_keyword(trimmed, "DELIMITER")
        {
            let next = rest.trim();
            if !next.is_empty() {
                self.delimiter = next.to_string();
            }
            self.echo(&text, cx);
            self.clear_input(window, cx);
            return true;
        }
        if !complete(&text, &self.delimiter) || self.running {
            return false;
        }
        self.echo(&text, cx);
        self.history.push(text.trim_end().to_string());
        self.history_ix = None;
        self.clear_input(window, cx);
        let sql = text
            .trim_end()
            .trim_end_matches(self.delimiter.as_str())
            .trim()
            .to_string();
        let Some(session) = self.session.clone() else {
            self.append(
                format!("ERROR: {}\n", t!("query.console_not_connected")),
                cx,
            );
            return true;
        };
        self.running = true;
        let refresh = RefreshAfterRun::new(self.connection.clone(), cx);
        refresh.mark_submitted();
        let started = Instant::now();
        let job = run_on(session, move |rel| {
            Box::pin(async move {
                let result = rel.execute_statement(&sql).await;
                drop(refresh);
                result
            })
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = job
                .await
                .map_err(|e| DbError::msg(e.to_string()))
                .and_then(|r| r);
            let secs = started.elapsed().as_secs_f64();
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                let text = format_result(this.dialect.unwrap_or(Dialect::MySql), &result, secs);
                this.append(text, cx);
            });
        }));
        true
    }

    fn echo(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut out = String::new();
        for (i, line) in text.trim_end().lines().enumerate() {
            out.push_str(&format!("{} {line}\n", self.prompt(i > 0)));
        }
        self.append(out, cx);
    }

    fn clear_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |e, cx| e.set_value("", window, cx));
    }

    /// ↑ on the first line or ↓ on the last walks this tab's history.
    fn walk_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let e = self.input.read(cx);
        let text = e.value().to_string();
        let caret = e.cursor();
        let on_edge = if back {
            !text[..caret].contains('\n')
        } else {
            !text[caret..].contains('\n')
        };
        if !on_edge || self.history.is_empty() {
            return false;
        }
        let next = match (self.history_ix, back) {
            (None, true) => Some(self.history.len() - 1),
            (None, false) => return false,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.history_ix = next;
        let value = next.map(|i| self.history[i].clone()).unwrap_or_default();
        let end = value.len();
        self.input.update(cx, |e, cx| {
            e.set_value(value, window, cx);
            e.set_selected_range(end..end, cx);
        });
        true
    }

    #[cfg(test)]
    pub(crate) fn session_busy(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.try_lock().is_err())
    }

    #[cfg(test)]
    pub(crate) fn has_session(&self) -> bool {
        self.session.is_some()
    }

    #[cfg(test)]
    pub(crate) fn input_state(&self) -> Entity<EditorState> {
        self.input.clone()
    }

    #[cfg(test)]
    pub(crate) fn output_text(&self) -> String {
        self.lines.join("\n")
    }

    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.clone().filter(|_| self.running) {
            navidog_db::runtime::spawn(async move {
                let _ = cancel().await;
            });
        }
    }
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let continuation = self.input.read(cx).value().contains('\n');
        v_flex()
            .id("console")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.control_bg)
            .on_action(cx.listener(|this, _: &Stop, _, _| this.stop()))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                if this.on_enter(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.walk_history(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.walk_history(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.output).bordered(false).size_full()),
            )
            .child(
                h_flex()
                    .items_start()
                    .min_h(tokens::query::TOOLBAR_HEIGHT.rems())
                    .max_h(tokens::snippet::CODE_HEIGHT.rems())
                    .border_t_1()
                    .border_color(p.separator)
                    .child(
                        div()
                            .flex_none()
                            .px_2()
                            .pt_1()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(p.label_secondary)
                            .child(self.prompt(continuation)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(tokens::snippet::CODE_HEIGHT.rems() * 0.5)
                            .child(Editor::new(&self.input).bordered(false).size_full()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_db::Column;

    #[test]
    fn prompts_follow_the_clients() {
        assert_eq!(prompt(Dialect::MySql, "", false, false), "mysql>");
        assert_eq!(prompt(Dialect::MySql, "", false, true), "    ->");
        assert_eq!(prompt(Dialect::Postgres, "pagila", true, false), "pagila=#");
        assert_eq!(prompt(Dialect::Postgres, "pagila", false, true), "pagila->");
        assert_eq!(prompt(Dialect::Sqlite, "", false, true), "   ...>");
        assert!(complete("SELECT 1;  ", ";"));
        assert!(!complete("SELECT 1", ";"));
        assert!(complete("END$$", "$$"));
    }

    #[test]
    fn results_and_errors_in_client_style() {
        let set = ResultSet {
            columns: vec![
                Column {
                    name: "id".into(),
                    type_name: "int".into(),
                    origin: None,
                },
                Column {
                    name: "n".into(),
                    type_name: "varchar".into(),
                    origin: None,
                },
            ],
            rows: vec![vec![Some("1".into()), None]],
            ..ResultSet::default()
        };
        let my = format_result(Dialect::MySql, &Ok(vec![set.clone()]), 0.0);
        assert!(
            my.contains("| id | n    |") && my.contains("|  1 | NULL |"),
            "{my}"
        );
        assert!(my.contains("1 row in set (0.00 sec)"));
        let pg = format_result(Dialect::Postgres, &Ok(vec![set]), 0.0);
        assert!(
            pg.contains("(1 row)") && pg.starts_with(" id |  n\n"),
            "{pg}"
        );
        let ok = format_result(
            Dialect::Sqlite,
            &Ok(vec![ResultSet {
                affected_rows: Some(2),
                ..ResultSet::default()
            }]),
            0.01,
        );
        assert!(ok.starts_with("Query OK, 2 rows affected (0.01 sec)"));
        let err = format_result(
            Dialect::MySql,
            &Err(DbError::coded(1146, "Table 'x' doesn't exist")),
            0.0,
        );
        assert!(err.starts_with("ERROR 1146: Table 'x' doesn't exist"));
        let pg_err = format_result(
            Dialect::Postgres,
            &Err(DbError::coded("42P01", "relation \"nope\" does not exist")),
            0.0,
        );
        assert!(pg_err.starts_with("ERROR:  relation \"nope\" does not exist"));
    }
}
