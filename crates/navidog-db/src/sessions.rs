//! Runtime state of open connections (11-connection.md §4).
//! Opening caches the resolved parameters (secrets included), so later sessions for table viewers
//! and query editors do not ask for passwords again.
//! A PostgreSQL session reaches one database; browsing another opens a metadata session on demand.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use navidog_core::connection::ConnectionId;
use navidog_core::store::ConnectionStore;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::dialect::Dialect;
use crate::{
    ConnectParams, DbError, DbResult, Driver, DriverRegistry, Relational, Secrets, Session, runtime,
};
use operation::OperationScope;

mod operation;

/// Closing takes the session out; other holders get `None` afterwards.
pub type SharedSession = Arc<Mutex<Option<Box<dyn Session>>>>;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnState {
    Closed,
    Opening,
    Open {
        version: String,
        databases: Vec<String>,
    },
    Failed(String),
}

impl ConnState {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

pub enum SessionsEvent {
    Opened(ConnectionId),
    Failed(ConnectionId, String),
    Closed(ConnectionId),
    /// Closed only to open again with new settings (profile switch, "Reopen Now"). Sessions and
    /// caches go as with `Closed`, but editor tabs stay and reconnect on `Opened`.
    Reopening(ConnectionId),
}

struct Live {
    driver: Arc<dyn Driver>,
    params: ConnectParams,
    dialect: Option<Dialect>,
    primary: SharedSession,
    /// The database the primary session is connected to (PostgreSQL)
    primary_db: Option<String>,
    per_db: Arc<Mutex<HashMap<String, SharedSession>>>,
    keepalive: Option<JoinHandle<()>>,
    operations: OperationScope,
}

impl Live {
    fn close_all(self) -> OperationScope {
        self.operations.revoke();
        if let Some(k) = &self.keepalive {
            k.abort();
        }
        let primary = self.primary.clone();
        let per_db = self.per_db.clone();
        runtime::spawn(async move {
            let mut all = vec![primary];
            all.extend(per_db.lock().await.drain().map(|(_, s)| s));
            for shared in all {
                if let Some(s) = shared.lock().await.take() {
                    let _ = s.close().await;
                }
            }
        });
        self.operations
    }
}

#[derive(Default)]
pub struct Sessions {
    states: HashMap<ConnectionId, ConnState>,
    live: HashMap<ConnectionId, Live>,
    generations: HashMap<ConnectionId, u64>,
    /// Retired domains stay available to every subsequent open until their operations drain.
    retiring: HashMap<ConnectionId, Vec<OperationScope>>,
    /// Opens in flight; closing or opening again drops the task, so its result is discarded
    tasks: HashMap<ConnectionId, Task<()>>,
}

impl EventEmitter<SessionsEvent> for Sessions {}

struct GlobalSessions(Entity<Sessions>);
impl Global for GlobalSessions {}

const CLOSED: ConnState = ConnState::Closed;

fn closed() -> DbError {
    DbError::msg("连接已关闭")
}

fn not_relational() -> DbError {
    DbError::msg(rust_i18n::t!("database.not_relational").to_string())
}

impl Sessions {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalSessions>().0.clone()
    }

    pub fn state(&self, id: &str) -> &ConnState {
        self.states.get(id).unwrap_or(&CLOSED)
    }

    /// Changes synchronously when opening or closing, including an explicit reopen. Async UI
    /// work captures this value and ignores its result if the connection has since changed.
    pub fn generation(&self, id: &str) -> u64 {
        self.generations.get(id).copied().unwrap_or(0)
    }

    fn advance_generation(&mut self, id: &str) {
        let generation = self.generations.entry(id.to_string()).or_default();
        *generation = generation
            .checked_add(1)
            .expect("connection generation overflow");
    }

    fn retire(&mut self, id: &str) {
        let retiring = self.retiring.entry(id.to_string()).or_default();
        retiring.retain(|scope| !scope.is_drained());
        if let Some(live) = self.live.remove(id) {
            let scope = live.close_all();
            if !scope.is_drained() {
                retiring.push(scope);
            }
        }
    }

    pub fn dialect(&self, id: &str) -> Option<Dialect> {
        self.live.get(id).and_then(|l| l.dialect)
    }

    pub fn session_count(&self, id: &str) -> usize {
        self.live
            .get(id)
            .map(|l| 1 + l.per_db.try_lock().map(|m| m.len()).unwrap_or(0))
            .unwrap_or(0)
    }

    pub fn open_ids(&self) -> impl Iterator<Item = &ConnectionId> {
        self.live.keys()
    }

    /// Opens a connection. The caller resolves the secret fields (possibly asking for a password).
    pub fn open(&mut self, id: ConnectionId, secrets: Secrets, cx: &mut Context<Self>) {
        let store = ConnectionStore::global(cx);
        let Some(conn) = store.read(cx).get(&id).cloned() else {
            return;
        };
        let Some(driver) = DriverRegistry::get(cx, &conn.driver) else {
            self.fail(id, format!("未知的驱动: {}", conn.driver), cx);
            return;
        };
        let params = ConnectParams::from_connection(&conn, secrets);
        let custom = conn.databases.clone();
        let keepalive = conn.advanced.keepalive_secs.filter(|s| *s > 0);

        let replacing = self.live.contains_key(&id) || self.tasks.contains_key(&id);
        self.advance_generation(&id);
        if replacing {
            // Direct open callers can replace a live connection without close_for_reopen.
            // Its editors must retire their owned sessions before Opened installs new ones.
            cx.emit(SessionsEvent::Reopening(id.clone()));
        }
        self.retire(&id);
        let retiring = self.retiring.get(&id).cloned().unwrap_or_default();
        self.states.insert(id.clone(), ConnState::Opening);
        cx.notify();

        let job = runtime::spawn({
            let driver = driver.clone();
            let params = params.clone();
            async move {
                let mut session = operation::connect_after_drain(retiring, driver, params).await?;
                let (dialect, databases) = match session.relational() {
                    Some(rel) => (Some(rel.dialect()), rel.list_databases().await?),
                    None => (None, Vec::new()),
                };
                Ok::<_, DbError>((session, dialect, databases))
            }
        });
        let task_id = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.tasks.remove(&task_id);
                match result {
                    Ok((session, dialect, mut databases)) => {
                        if custom.custom_list {
                            databases.retain(|d| custom.items.iter().any(|i| &i.name == d));
                        }
                        let version = session.server_version().to_string();
                        let primary = Arc::new(Mutex::new(Some(session)));
                        let keepalive =
                            keepalive.map(|secs| start_keepalive(primary.clone(), secs));
                        let primary_db = driver
                            .info()
                            .per_database_sessions
                            .then(|| params.database.clone().unwrap_or_else(|| "postgres".into()));
                        this.live.insert(
                            task_id.clone(),
                            Live {
                                driver,
                                params,
                                dialect,
                                primary,
                                primary_db,
                                per_db: Default::default(),
                                keepalive,
                                operations: OperationScope::default(),
                            },
                        );
                        this.states
                            .insert(task_id.clone(), ConnState::Open { version, databases });
                        cx.emit(SessionsEvent::Opened(task_id));
                        cx.notify();
                    }
                    Err(msg) => this.fail(task_id, msg, cx),
                }
            });
        });
        self.tasks.insert(id, task);
    }

    fn fail(&mut self, id: ConnectionId, msg: String, cx: &mut Context<Self>) {
        self.states
            .insert(id.clone(), ConnState::Failed(msg.clone()));
        cx.emit(SessionsEvent::Failed(id, msg));
        cx.notify();
    }

    pub fn close(&mut self, id: &str, cx: &mut Context<Self>) {
        self.shut(id);
        cx.emit(SessionsEvent::Closed(id.to_string()));
        cx.notify();
    }

    /// Closes before opening again with new settings; see `SessionsEvent::Reopening`.
    pub fn close_for_reopen(&mut self, id: &str, cx: &mut Context<Self>) {
        self.shut(id);
        cx.emit(SessionsEvent::Reopening(id.to_string()));
        cx.notify();
    }

    fn shut(&mut self, id: &str) {
        self.advance_generation(id);
        self.tasks.remove(id);
        self.retire(id);
        self.states.insert(id.to_string(), ConnState::Closed);
    }

    /// Lists the databases again.
    pub fn refresh(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(job) = self.run_rel(id, None, |r| r.list_databases()) else {
            return;
        };
        let task_id = id.to_string();
        let task = cx.spawn(async move |this, cx| {
            let Ok(Ok(dbs)) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(ConnState::Open { databases, .. }) = this.states.get_mut(&task_id) {
                    *databases = dbs;
                    cx.notify();
                }
            });
        });
        self.tasks.insert(id.to_string(), task);
    }

    /// Runs safe metadata reads on the connection's shared metadata session. `database` picks the
    /// PostgreSQL database to reach, opening another session when needed; other dialects ignore it.
    /// Generated writes and DDL belong in `run_operation_rel`, never on this long-lived session.
    pub fn run_rel<T: Send + 'static>(
        &self,
        id: &str,
        database: Option<&str>,
        f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>> + Send + 'static,
    ) -> Option<JoinHandle<DbResult<T>>> {
        let live = self.live.get(id)?;
        let target = database
            .filter(|db| live.primary_db.as_deref().is_some_and(|p| p != *db))
            .map(String::from);
        let primary = live.primary.clone();
        let per_db = live.per_db.clone();
        let driver = live.driver.clone();
        let mut params = live.params.clone();
        Some(runtime::spawn(async move {
            let shared = match target {
                None => primary,
                Some(db) => {
                    let mut map = per_db.lock().await;
                    match map.get(&db) {
                        Some(s) => s.clone(),
                        None => {
                            params.database = Some(db.clone());
                            let s =
                                Arc::new(Mutex::new(Some(crate::connect(driver, params).await?)));
                            map.insert(db, s.clone());
                            s
                        }
                    }
                }
            };
            with_rel(&shared, f).await
        }))
    }

    /// Runs one generated write/DDL operation on a separate session and closes it afterwards.
    /// The operation is invoked once and never replayed; its outcome is preserved even if closing
    /// fails. Closing or reopening revokes pending work; an interrupted operation that started
    /// its business closure has an unknown outcome. New opens wait for local cancellation and
    /// cleanup, which cannot guarantee server rollback. Shared metadata stays recoverable.
    pub fn run_operation_rel<T: Send + 'static>(
        &self,
        id: &str,
        database: Option<&str>,
        f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>> + Send + 'static,
    ) -> Option<JoinHandle<DbResult<T>>> {
        let (driver, params) = self.session_target(id, database)?;
        let operations = &self.live.get(id)?.operations;
        Some(operations.run(driver, params, f))
    }

    /// Compatibility entry point for callers of the original public operation API.
    #[deprecated(note = "Use run_operation_rel instead.")]
    pub fn run_once<T: Send + 'static>(
        &self,
        id: &str,
        database: Option<&str>,
        f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>> + Send + 'static,
    ) -> Option<JoinHandle<DbResult<T>>> {
        self.run_operation_rel(id, database, f)
    }

    /// Opens a separate session (table viewer, query editor); the caller holds and closes it.
    pub fn new_session(
        &self,
        id: &str,
        database: Option<&str>,
    ) -> Option<JoinHandle<DbResult<Box<dyn Session>>>> {
        let (driver, params) = self.session_target(id, database)?;
        Some(runtime::spawn(crate::connect(driver, params)))
    }

    fn session_target(
        &self,
        id: &str,
        database: Option<&str>,
    ) -> Option<(Arc<dyn Driver>, ConnectParams)> {
        let live = self.live.get(id)?;
        let mut params = live.params.clone();
        if live.primary_db.is_some()
            && let Some(db) = database
        {
            params.database = Some(db.to_string());
        }
        Some((live.driver.clone(), params))
    }
}

/// MySQL sessions start in the server's default database. Point one at `database` before an
/// application write. PostgreSQL already connected to that database via `new_session`.
pub async fn prepare_database(
    rel: &mut dyn Relational,
    dialect: Dialect,
    database: &str,
) -> DbResult<()> {
    if dialect == Dialect::MySql && !database.is_empty() {
        rel.execute(&format!("USE {}", dialect.quote_ident(database)))
            .await?;
    }
    Ok(())
}

/// Runs on a session the caller holds (table viewer, query editor).
pub fn run_on<T: Send + 'static>(
    shared: SharedSession,
    f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>> + Send + 'static,
) -> JoinHandle<DbResult<T>> {
    runtime::spawn(async move { with_rel(&shared, f).await })
}

async fn with_rel<T>(
    shared: &SharedSession,
    f: impl for<'a> FnOnce(&'a mut dyn Relational) -> BoxFuture<'a, DbResult<T>>,
) -> DbResult<T> {
    let mut guard = shared.lock().await;
    let session = guard.as_mut().ok_or_else(closed)?;
    let rel = session.relational().ok_or_else(not_relational)?;
    f(rel).await
}

fn start_keepalive(session: SharedSession, secs: u32) -> JoinHandle<()> {
    runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(secs as u64));
        tick.tick().await;
        loop {
            tick.tick().await;
            let mut guard = session.lock().await;
            let Some(s) = guard.as_mut() else { return };
            if let Err(e) = s.ping().await {
                tracing::warn!("保持连接失败: {e}");
            }
        }
    })
}

pub(crate) fn init(cx: &mut App) {
    let sessions = cx.new(|_| Sessions::default());
    cx.set_global(GlobalSessions(sessions));
}

#[cfg(test)]
mod tests;
