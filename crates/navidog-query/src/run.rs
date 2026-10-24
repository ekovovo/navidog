//! Running SQL from a query editor (31-query-results.md §2–3): the editor's own session, the
//! statements to run, parameters, and the statement loop.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::{RopeExt as _, WindowExt as _};
use gpui_kit::{App, AsyncWindowContext, Context, WeakEntity, Window};
use navidog_core::actions::OpenConnectionId;
use navidog_core::settings::{ResultsPosition, Settings};
use navidog_core::workspace::Workspace;
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{Sessions, SharedSession, run_on};
use navidog_db::sql::{Statement, session_setup, split_script};
use navidog_db::{CancelHandle, DbError, DbResult, Relational, ResultSet};
use navidog_ui::assets::lucide;
use navidog_util::sql_text;
use rust_i18n::t;

use crate::editor::QueryEditor;

/// The final submitted database job publishes a batch's refresh even after its panel closes.
#[derive(Clone)]
pub(crate) struct RefreshAfterRun {
    _lifetime: Arc<tokio::sync::oneshot::Sender<()>>,
    submitted: Arc<AtomicBool>,
}

impl RefreshAfterRun {
    pub(crate) fn new(connection: String, cx: &mut App) -> Self {
        let (lifetime, completed) = tokio::sync::oneshot::channel();
        let submitted = Arc::new(AtomicBool::new(false));
        let observed = submitted.clone();
        // This application task holds neither the panel nor its window. The sender is only
        // dropped after both the batch owner and every submitted database closure finish.
        cx.spawn(async move |cx| {
            let _ = completed.await;
            if observed.load(Ordering::Acquire) {
                cx.update(|cx| {
                    Workspace::global(cx).update(cx, |workspace, cx| {
                        workspace.database_changed(connection, cx)
                    });
                });
            }
        })
        .detach();
        Self {
            _lifetime: Arc::new(lifetime),
            submitted,
        }
    }

    pub(crate) fn mark_submitted(&self) {
        self.submitted.store(true, Ordering::Release);
    }
}

/// The session a query editor runs on. It stays open between runs, so `BEGIN`, `SET` and
/// temporary tables carry over (31 §2).
pub(crate) struct Live {
    pub session: SharedSession,
    pub cancel: Option<CancelHandle>,
    pub connection: String,
    /// PostgreSQL: the database the session reached
    pub database: Option<String>,
    /// MySQL: the database chosen with `USE`; PostgreSQL: the schema put on the search path
    pub used: Option<String>,
    /// SQL can change the physical context without changing the toolbar's last applied choice.
    pub context: Option<SessionContext>,
}

impl Live {
    pub fn close(self) {
        navidog_grid::commit::close_session(self.session);
    }

    pub fn matches_source(&self, info: &crate::results::RunInfo) -> bool {
        if self.connection != info.connection_id {
            return false;
        }
        if !info.context_known {
            return self
                .context
                .as_ref()
                .is_some_and(|context| Some(context.id) == info.context_id);
        }
        match &self.context {
            Some(context) => {
                context.known
                    && context.database == info.database
                    && context.schema == info.schema
                    && context.search_path == info.search_path
            }
            None => match info.dialect {
                Some(Dialect::MySql) => self.used == info.database,
                Some(Dialect::Postgres) => {
                    self.database == info.database
                        && self.used == info.schema
                        && info.search_path.is_none()
                }
                _ => self.database == info.database,
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionContext {
    id: u64,
    known: bool,
    database: Option<String>,
    schema: Option<String>,
    search_path: Option<String>,
}

impl SessionContext {
    fn apply(&self, info: &mut crate::results::RunInfo) {
        info.database = self.database.clone();
        info.schema = self.schema.clone();
        info.search_path = self.search_path.clone();
        info.context_id = Some(self.id);
        info.context_known = self.known;
    }

    fn unknown(dialect: Dialect, info: &crate::results::RunInfo) -> Self {
        Self {
            id: next_context_id(),
            known: false,
            database: (dialect == Dialect::Postgres)
                .then(|| info.database.clone())
                .flatten(),
            schema: None,
            search_path: None,
        }
    }

    /// Read server state after a recognized context command; do not parse user expressions.
    async fn read(rel: &mut dyn Relational) -> DbResult<Self> {
        let dialect = rel.dialect();
        let sql = match dialect {
            Dialect::MySql => "SELECT DATABASE()",
            Dialect::Postgres => {
                "SELECT pg_catalog.current_database(), pg_catalog.current_schema(), \
                 pg_catalog.current_setting('search_path')"
            }
            Dialect::Sqlite => return Err(DbError::msg("SQLite has no USE/search_path context")),
        };
        let sets = rel.execute_read(sql).await?;
        let row = sets
            .iter()
            .find_map(|set| set.rows.first())
            .ok_or_else(|| DbError::msg("The session context query returned no row."))?;
        let value = |ix| row.get(ix).cloned().flatten();
        Ok(Self {
            id: next_context_id(),
            known: true,
            database: value(0),
            schema: (dialect == Dialect::Postgres).then(|| value(1)).flatten(),
            search_path: (dialect == Dialect::Postgres).then(|| value(2)).flatten(),
        })
    }
}

fn next_context_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

async fn execute_with_context(
    rel: &mut dyn Relational,
    sql: &str,
) -> DbResult<(Vec<ResultSet>, Option<DbResult<SessionContext>>)> {
    let sets = rel.execute_statement(sql).await?;
    let context = if session_setup(sql).is_some() {
        Some(SessionContext::read(rel).await)
    } else {
        None
    };
    Ok((sets, context))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunKind {
    /// The selection, else everything
    All,
    /// The statement at the caret
    Current,
    /// The selection only
    Selected,
    /// Query plans of the selection, else of everything (31 §5)
    Explain,
}

/// A statement to send: the SQL after parameter substitution, the text as typed, and where it sits
/// in the editor.
#[derive(Clone, Debug)]
pub(crate) struct Planned {
    pub sql: String,
    pub shown: String,
    pub range: Range<usize>,
}

/// How one statement went, for the messages and summary pages.
pub(crate) struct Outcome {
    pub planned: Planned,
    pub result: DbResult<Vec<ResultSet>>,
    pub started: chrono::DateTime<chrono::Local>,
    pub elapsed: Duration,
}

/// The statement around `cursor`; between two statements, the one before (31 §2).
pub(crate) fn statement_at(statements: &[Statement], cursor: usize) -> Option<&Statement> {
    statements
        .iter()
        .find(|s| s.range.contains(&cursor) || s.range.end == cursor)
        .or_else(|| statements.iter().rev().find(|s| s.range.end <= cursor))
        .or_else(|| statements.first())
}

/// A parameter value: a string literal, or the text itself in raw mode (31 §3).
pub(crate) fn param_value(dialect: Dialect, value: &str, raw: bool) -> String {
    if raw {
        value.to_string()
    } else {
        dialect.string_literal(value)
    }
}

impl QueryEditor {
    /// The statements `kind` covers, with ranges in the editor text.
    fn statements(&self, kind: RunKind, dialect: Dialect, cx: &App) -> Vec<Statement> {
        let e = self.editor.read(cx);
        let text = e.value().to_string();
        let selection = e.selected_range();
        let within = |range: Range<usize>| {
            split_script(&text[range.clone()], dialect)
                .into_iter()
                .map(|s| Statement {
                    text: s.text,
                    range: s.range.start + range.start..s.range.end + range.start,
                })
                .collect()
        };
        match kind {
            RunKind::All | RunKind::Explain if !selection.is_empty() => within(selection),
            RunKind::All | RunKind::Explain => split_script(&text, dialect),
            RunKind::Selected if selection.is_empty() => Vec::new(),
            RunKind::Selected => within(selection),
            RunKind::Current => {
                let all = split_script(&text, dialect);
                statement_at(&all, e.cursor())
                    .cloned()
                    .into_iter()
                    .collect()
            }
        }
    }

    pub(crate) fn run(&mut self, kind: RunKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let Some(conn) = self.target.connection.clone() else {
            alert(t!("query.no_connection").to_string(), window, cx);
            return;
        };
        if !Sessions::global(cx).read(cx).state(&conn).is_open() {
            self.pending_run = Some(kind);
            window.dispatch_action(Box::new(OpenConnectionId { id: conn.into() }), cx);
            return;
        }
        let Some(dialect) = self.dialect(cx) else {
            return;
        };
        let statements = self.statements(kind, dialect, cx);
        if statements.is_empty() {
            alert(t!("query.empty").to_string(), window, cx);
            return;
        }
        let names: Vec<String> = statements
            .iter()
            .flat_map(|s| sql_text::param_names(&s.text))
            .fold(Vec::new(), |mut acc, n| {
                if !acc.contains(&n) {
                    acc.push(n);
                }
                acc
            });
        if names.is_empty() {
            self.start(kind, statements, dialect, window, cx);
            return;
        }
        let prefilled = names
            .iter()
            .map(|n| {
                let (v, raw) = self.params.get(n).cloned().unwrap_or_default();
                (n.clone(), v, raw)
            })
            .collect();
        let this = cx.entity().downgrade();
        crate::params::ask(prefilled, window, cx, move |values, window, cx| {
            let statements = statements.clone();
            let _ = this.update(cx, |this, cx| {
                for (name, value, raw) in values {
                    this.params.insert(name, (value, raw));
                }
                this.start(kind, statements, dialect, window, cx);
            });
        });
    }

    fn plan(&self, statements: Vec<Statement>, dialect: Dialect) -> Vec<Planned> {
        statements
            .into_iter()
            .map(|s| Planned {
                sql: sql_text::substitute_params(&s.text, |name| {
                    let (v, raw) = self.params.get(name).cloned().unwrap_or_default();
                    param_value(dialect, &v, raw)
                }),
                shown: s.text,
                range: s.range,
            })
            .collect()
    }

    fn start(
        &mut self,
        kind: RunKind,
        statements: Vec<Statement>,
        dialect: Dialect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let planned = self.plan(statements, dialect);
        let Some(session) = self.session_job(dialect, cx) else {
            return;
        };
        self.results
            .begin_run(Settings::global(cx).keep_results, cx);
        self.clear_diagnostics(cx);
        self.running = true;
        let stop = Arc::new(AtomicBool::new(false));
        self.stop_flag = stop.clone();
        let continue_on_error = Settings::global(cx).continue_on_error;
        let profile = kind != RunKind::Explain
            && dialect == Dialect::MySql
            && Settings::global(cx).show_profile;
        let mariadb = self.server_version(cx).contains("MariaDB");
        let mut source = crate::results::RunInfo {
            at: chrono::Local::now(),
            query_time: Duration::ZERO,
            connection: self.connection_name(cx),
            connection_id: self.target.connection.clone().unwrap_or_default(),
            database: self.target.database.clone(),
            schema: self.target.schema.clone(),
            search_path: None,
            context_id: None,
            context_known: true,
            dialect: Some(dialect),
        };
        let refresh = (kind != RunKind::Explain)
            .then(|| RefreshAfterRun::new(source.connection_id.clone(), cx));
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let session = match session.await {
                Ok((session, context)) => {
                    if let Some(context) = context {
                        context.apply(&mut source);
                    }
                    session
                }
                Err(e) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.results.failed_to_start(&e.to_string(), window, cx);
                        this.finish(kind, window, cx);
                    });
                    return;
                }
            };
            if kind == RunKind::Explain {
                explain_all(this.clone(), session, source, mariadb, planned, stop, cx).await;
                let _ = this.update_in(cx, |this, window, cx| this.finish(kind, window, cx));
                return;
            }
            let before = if profile {
                crate::run::profile_begin(&session).await
            } else {
                None
            };
            let refresh = refresh.expect("ordinary SQL runs own a refresh completion guard");
            for p in planned {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let started = chrono::Local::now();
                let clock = Instant::now();
                let sql = p.sql.clone();
                refresh.mark_submitted();
                let completed = refresh.clone();
                let result = run_on(session.clone(), move |rel| {
                    Box::pin(async move {
                        let result = execute_with_context(rel, &sql).await;
                        drop(completed);
                        result
                    })
                })
                .await
                .map_err(|e| DbError::msg(e.to_string()))
                .and_then(|r| r);
                let result = result.map(|(sets, context)| {
                    if let Some(context) = context {
                        // A metadata probe cannot turn successful user SQL into a failure.
                        let context =
                            context.unwrap_or_else(|_| SessionContext::unknown(dialect, &source));
                        context.apply(&mut source);
                        let _ = this.update(cx, |this, _| {
                            if let Some(live) = &mut this.live
                                && Arc::ptr_eq(&live.session, &session)
                            {
                                live.context = Some(context);
                            }
                        });
                    }
                    sets
                });
                let failed = result.is_err();
                let outcome = Outcome {
                    planned: p,
                    result,
                    started,
                    elapsed: clock.elapsed(),
                };
                let _ = this.update_in(cx, |this, window, cx| {
                    this.record(outcome, source.clone(), window, cx)
                });
                if failed && !continue_on_error {
                    break;
                }
            }
            // Errors and cancellation can follow committed writes; release the batch owner
            // without depending on the editor or window still being alive.
            drop(refresh);
            if let Some(before) = before {
                let profiled = crate::run::profile_end(&session, before).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.results.show_profile(profiled, window, cx)
                });
            }
            let _ = this.update_in(cx, |this, window, cx| this.finish(kind, window, cx));
        }));
    }

    /// The editor's session pointed at the picked database and schema, opening it on first use.
    fn session_job(
        &mut self,
        dialect: Dialect,
        cx: &mut Context<Self>,
    ) -> Option<
        impl std::future::Future<Output = DbResult<(SharedSession, Option<SessionContext>)>> + use<>,
    > {
        let conn = self.target.connection.clone()?;
        let pg = dialect == Dialect::Postgres;
        // Another connection, or another PostgreSQL database: that session cannot be reused
        if let Some(live) = self.live.take() {
            let same_db = !pg || live.database == self.target.database;
            if live.connection == conn && same_db {
                self.live = Some(live);
            } else {
                live.close();
            }
        }
        let open = match &self.live {
            Some(_) => None,
            None => Some(
                Sessions::global(cx)
                    .read(cx)
                    .new_session(&conn, self.target.database.as_deref())?,
            ),
        };
        let existing = self
            .live
            .as_ref()
            .map(|l| (l.session.clone(), l.used.clone(), l.context.clone()));
        let want = match dialect {
            Dialect::MySql => self.target.database.clone(),
            Dialect::Postgres => self.target.schema.clone(),
            Dialect::Sqlite => None,
        };
        let database = self.target.database.clone();
        let this = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        Some(async move {
            let (session, used, mut context) = match (existing, open) {
                (Some(found), _) => found,
                (None, Some(job)) => {
                    let s = job.await.map_err(|e| DbError::msg(e.to_string()))??;
                    let cancel = s.cancel_handle();
                    let session: SharedSession = Arc::new(tokio::sync::Mutex::new(Some(s)));
                    let live = Live {
                        session: session.clone(),
                        cancel,
                        connection: conn.clone(),
                        database: database.clone(),
                        used: None,
                        context: None,
                    };
                    let _ = this.update(&mut async_cx, |this, _| {
                        if let Some(old) = this.live.replace(live) {
                            old.close();
                        }
                    });
                    (session, None, None)
                }
                (None, None) => unreachable!("no session and nothing opening one"),
            };
            if let Some(name) = want.filter(|w| Some(w) != used.as_ref()) {
                let sql = match dialect {
                    Dialect::MySql => format!("USE {}", dialect.quote_ident(&name)),
                    _ => search_path(&name),
                };
                run_on(session.clone(), move |rel| {
                    Box::pin(async move { rel.execute(&sql).await })
                })
                .await
                .map_err(|e| DbError::msg(e.to_string()))??;
                let _ = this.update(&mut async_cx, |this, _| {
                    if let Some(live) = this.live.as_mut() {
                        live.used = Some(name);
                        live.context = None;
                    }
                });
                context = None;
            }
            Ok((session, context))
        })
    }

    pub(crate) fn stop(&mut self, _: &mut Context<Self>) {
        if !self.running {
            return;
        }
        self.stop_flag.store(true, Ordering::Release);
        if let Some(cancel) = self.live.as_ref().and_then(|l| l.cancel.clone()) {
            navidog_db::runtime::spawn(async move {
                if let Err(e) = cancel().await {
                    tracing::warn!("cancel query failed: {e}");
                }
            });
        }
    }

    fn record(
        &mut self,
        outcome: Outcome,
        mut info: crate::results::RunInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = &outcome.result {
            self.mark_error(
                outcome.planned.range.clone(),
                &crate::results::db_text(e),
                cx,
            );
        }
        let editor = cx.entity().downgrade();
        info.at = outcome.started;
        info.query_time = outcome.elapsed;
        self.results.record(outcome, info, editor, window, cx);
        cx.notify();
    }

    fn finish(&mut self, kind: RunKind, _: &mut Window, cx: &mut Context<Self>) {
        self.running = false;
        self.results.end_run(kind == RunKind::Explain, cx);
        if Settings::global(cx).results_position == ResultsPosition::NewPage {
            self.results_page = true;
        }
        cx.notify();
    }

    pub(crate) fn server_version(&self, cx: &App) -> String {
        match self
            .target
            .connection
            .as_deref()
            .map(|id| Sessions::global(cx).read(cx).state(id).clone())
        {
            Some(navidog_db::sessions::ConnState::Open { version, .. }) => version,
            _ => String::new(),
        }
    }

    pub(crate) fn mark_error(
        &mut self,
        range: Range<usize>,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        let message = message.to_string();
        self.editor.update(cx, |e, cx| {
            let text = e.text().clone();
            let end = range.end.min(text.len());
            let start = range.start.min(end);
            if let Some(d) = e.diagnostics_mut() {
                d.push(
                    Diagnostic::new(
                        text.offset_to_position(start)..text.offset_to_position(end),
                        message,
                    )
                    .with_severity(DiagnosticSeverity::Error),
                );
            }
            cx.notify();
        });
    }

    fn clear_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            if let Some(d) = e.diagnostics_mut() {
                d.clear();
            }
            cx.notify();
        });
    }

    /// Selects a statement in the editor (double-click in the summary).
    pub(crate) fn select_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.results_page = false;
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(range, cx);
            e.focus(window, cx);
        });
        cx.notify();
    }
}

/// `SET search_path` for a picked schema; `public` stays on it so extensions installed there keep
/// resolving (31 §2).
pub(crate) fn search_path(schema: &str) -> String {
    let d = Dialect::Postgres;
    if schema == "public" {
        format!("SET search_path TO {}", d.quote_ident(schema))
    } else {
        format!(
            "SET search_path TO {}, {}",
            d.quote_ident(schema),
            d.quote_ident("public")
        )
    }
}

pub(crate) fn alert(message: String, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert.icon(lucide("info")).title(message.clone())
    });
}

/// Session status before the run, after `SET profiling = 1` (31 §4.5).
pub(crate) async fn profile_begin(session: &SharedSession) -> Option<Vec<(String, String)>> {
    let job = run_on(session.clone(), |rel| {
        Box::pin(async move {
            rel.execute("SET profiling = 1").await?;
            rel.execute("SHOW SESSION STATUS").await
        })
    });
    let sets = job.await.ok()?.ok()?;
    Some(pairs(sets))
}

/// The last statement's profile and the status variables that changed.
pub(crate) async fn profile_end(
    session: &SharedSession,
    before: Vec<(String, String)>,
) -> Option<(ResultSet, Vec<(String, String)>)> {
    let job = run_on(session.clone(), |rel| {
        Box::pin(async move {
            let profile = rel.execute("SHOW PROFILE").await?;
            let status = rel.execute("SHOW SESSION STATUS").await?;
            Ok::<_, DbError>((profile, status))
        })
    });
    let (profile, status) = job.await.ok()?.ok()?;
    let after = pairs(status);
    Some((
        profile.into_iter().next().unwrap_or_default(),
        status_changes(&before, &after),
    ))
}

fn pairs(sets: Vec<ResultSet>) -> Vec<(String, String)> {
    sets.into_iter()
        .next()
        .map(|s| {
            s.rows
                .into_iter()
                .map(|r| {
                    let cell = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
                    (cell(0), cell(1))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Status variables whose value changed, with the difference for numbers.
pub(crate) fn status_changes(
    before: &[(String, String)],
    after: &[(String, String)],
) -> Vec<(String, String)> {
    after
        .iter()
        .filter_map(|(name, now)| {
            let was = before
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str());
            if was == Some(now.as_str()) {
                return None;
            }
            let delta = match (was.and_then(|w| w.parse::<i64>().ok()), now.parse::<i64>()) {
                (Some(w), Ok(n)) => (n - w).to_string(),
                _ => now.clone(),
            };
            Some((name.clone(), delta))
        })
        .collect()
}

/// Fetches the plans for "Explain" and opens one explain tab per statement (31 §5).
pub(crate) async fn explain_all(
    this: WeakEntity<QueryEditor>,
    session: SharedSession,
    source: crate::results::RunInfo,
    mariadb: bool,
    planned: Vec<Planned>,
    stop: Arc<AtomicBool>,
    cx: &mut AsyncWindowContext,
) {
    let dialect = source.dialect.expect("a run has a dialect");
    for p in planned {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let started = chrono::Local::now();
        let clock = Instant::now();
        let sql = p.sql.clone();
        let result = run_on(session.clone(), move |rel| {
            Box::pin(async move { crate::explain::fetch_plan(rel, dialect, mariadb, &sql).await })
        })
        .await
        .map_err(|e| DbError::msg(e.to_string()))
        .and_then(|r| r);
        let elapsed = clock.elapsed();
        let _ = this.update_in(cx, |this, window, cx| {
            let mut info = source.clone();
            info.at = started;
            info.query_time = elapsed;
            this.record_plan(p, result, info, window, cx)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_context_probe_keeps_success_and_requires_the_same_context() {
        type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
        struct ProbeFailure(usize);
        impl Relational for ProbeFailure {
            fn dialect(&self) -> Dialect {
                Dialect::MySql
            }
            fn execute<'a>(&'a mut self, _: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
                self.0 += 1;
                Box::pin(async {
                    Ok(vec![ResultSet {
                        affected_rows: Some(0),
                        ..Default::default()
                    }])
                })
            }
            fn execute_read<'a>(
                &'a mut self,
                _: &'a str,
            ) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
                Box::pin(async { Err(DbError::msg("injected context probe failure")) })
            }
        }
        let mut probe = ProbeFailure(0);
        let (sets, context) = navidog_db::runtime::runtime()
            .block_on(execute_with_context(&mut probe, "USE other"))
            .unwrap();
        assert_eq!(probe.0, 1, "successful user SQL is never replayed");
        assert_eq!(sets[0].affected_rows, Some(0));
        assert!(context.unwrap().is_err());
        let mut info = crate::results::RunInfo {
            at: chrono::Local::now(),
            query_time: Duration::ZERO,
            connection: "fixture".into(),
            connection_id: "fixture".into(),
            database: Some("original".into()),
            schema: None,
            search_path: None,
            context_id: None,
            context_known: true,
            dialect: Some(Dialect::MySql),
        };
        let context = SessionContext::unknown(Dialect::MySql, &info);
        context.apply(&mut info);
        assert!(!info.context_known);
        assert!(
            info.database.is_none(),
            "unknown context cannot claim the toolbar's database"
        );
        let mut live = Live {
            session: Arc::new(tokio::sync::Mutex::new(None)),
            cancel: None,
            connection: "fixture".into(),
            database: Some("original".into()),
            used: Some("original".into()),
            context: Some(context),
        };
        assert!(live.matches_source(&info));
        live.context = Some(SessionContext::unknown(Dialect::MySql, &info));
        assert!(
            !live.matches_source(&info),
            "a later unknown context cannot stand in for the original"
        );
        live.context = None;
        assert!(!live.matches_source(&info));
    }

    #[test]
    fn current_statement_prefers_the_one_before_a_gap() {
        let sql = "SELECT 1;\n\nSELECT 2;";
        let stmts = split_script(sql, Dialect::MySql);
        assert_eq!(statement_at(&stmts, 3).unwrap().text, "SELECT 1");
        // On the blank line between the two statements
        assert_eq!(statement_at(&stmts, 10).unwrap().text, "SELECT 1");
        assert_eq!(statement_at(&stmts, 13).unwrap().text, "SELECT 2");
    }

    #[test]
    fn parameters_are_literals_unless_raw() {
        assert_eq!(param_value(Dialect::MySql, "O'Hara", false), "'O''Hara'");
        assert_eq!(param_value(Dialect::MySql, "NOW()", true), "NOW()");
    }

    #[test]
    fn search_path_keeps_public() {
        assert_eq!(search_path("public"), "SET search_path TO \"public\"");
        assert_eq!(
            search_path("inventory"),
            "SET search_path TO \"inventory\", \"public\""
        );
    }

    #[test]
    fn status_diff_keeps_changed_values() {
        let before = vec![
            ("Bytes_sent".into(), "100".into()),
            ("Same".into(), "1".into()),
        ];
        let after = vec![
            ("Bytes_sent".into(), "250".into()),
            ("Same".into(), "1".into()),
            ("Ssl_cipher".into(), "TLS".into()),
        ];
        assert_eq!(
            status_changes(&before, &after),
            [
                ("Bytes_sent".to_string(), "150".to_string()),
                ("Ssl_cipher".to_string(), "TLS".to_string())
            ]
        );
    }
}
