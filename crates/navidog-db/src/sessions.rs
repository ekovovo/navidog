//! 已打开连接的运行时状态（11-connection.md 第 4 节）。
//! 打开连接时缓存解析后的连接参数（含秘密），之后为表查看器、查询编辑器另开会话无需再次询问密码。
//! PostgreSQL 的会话只能访问一个数据库，浏览其他数据库时按需另开元数据会话。

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

/// 关闭时取出会话，其余持有者之后拿到 `None`。
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
}

struct Live {
    driver: Arc<dyn Driver>,
    params: ConnectParams,
    dialect: Option<Dialect>,
    primary: SharedSession,
    /// 主会话所连的数据库（PostgreSQL）
    primary_db: Option<String>,
    per_db: Arc<Mutex<HashMap<String, SharedSession>>>,
    keepalive: Option<JoinHandle<()>>,
}

impl Live {
    fn close_all(self) {
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
    }
}

#[derive(Default)]
pub struct Sessions {
    states: HashMap<ConnectionId, ConnState>,
    live: HashMap<ConnectionId, Live>,
    /// 进行中的打开任务；关闭连接或重复打开时丢弃，结果随之作废
    tasks: HashMap<ConnectionId, Task<()>>,
}

impl EventEmitter<SessionsEvent> for Sessions {}

struct GlobalSessions(Entity<Sessions>);
impl Global for GlobalSessions {}

const CLOSED: ConnState = ConnState::Closed;

fn closed() -> DbError {
    DbError::msg("连接已关闭")
}

impl Sessions {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalSessions>().0.clone()
    }

    pub fn state(&self, id: &str) -> &ConnState {
        self.states.get(id).unwrap_or(&CLOSED)
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

    /// 打开连接。秘密字段由调用方解析（可能需要弹出询问密码对话框）。
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

        if let Some(old) = self.live.remove(&id) {
            old.close_all();
        }
        self.states.insert(id.clone(), ConnState::Opening);
        cx.notify();

        let job = runtime::spawn({
            let driver = driver.clone();
            let params = params.clone();
            async move {
                let mut session = crate::connect(driver, params).await?;
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
        self.tasks.remove(id);
        if let Some(live) = self.live.remove(id) {
            live.close_all();
        }
        self.states.insert(id.to_string(), ConnState::Closed);
        cx.emit(SessionsEvent::Closed(id.to_string()));
        cx.notify();
    }

    /// 重新列出数据库。
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

    /// 在连接的元数据会话上执行。`database` 为 PostgreSQL 指定要访问的数据库，必要时另开会话；
    /// 其他方言忽略它（同一会话即可访问所有数据库）。
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

    /// 另开一个独立会话（表查看器、查询编辑器），由调用方持有并负责关闭。
    pub fn new_session(
        &self,
        id: &str,
        database: Option<&str>,
    ) -> Option<JoinHandle<DbResult<Box<dyn Session>>>> {
        let live = self.live.get(id)?;
        let mut params = live.params.clone();
        if live.primary_db.is_some()
            && let Some(db) = database
        {
            params.database = Some(db.to_string());
        }
        let driver = live.driver.clone();
        Some(runtime::spawn(crate::connect(driver, params)))
    }
}

/// 在调用方自己持有的会话上执行（表查看器、查询编辑器）。
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
    let rel = session
        .relational()
        .ok_or_else(|| DbError::msg("该连接不是关系型数据库"))?;
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
