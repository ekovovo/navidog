//! 已打开连接的运行时状态（11-connection.md 第 4 节）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use navidog_core::connection::ConnectionId;
use navidog_core::store::ConnectionStore;
use navidog_db::{ConnectParams, DriverRegistry, Secrets, Session, runtime};
use tokio::sync::Mutex;

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
    session: SharedSession,
    keepalive: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(k) = self.keepalive.take() {
            k.abort();
        }
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

impl Sessions {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalSessions>().0.clone()
    }

    pub fn state(&self, id: &str) -> &ConnState {
        self.states.get(id).unwrap_or(&CLOSED)
    }

    pub fn session_count(&self, id: &str) -> usize {
        usize::from(self.live.contains_key(id))
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

        self.live.remove(&id);
        self.states.insert(id.clone(), ConnState::Opening);
        cx.notify();

        let job = runtime::spawn(async move {
            let mut session = navidog_db::connect(driver, params).await?;
            let databases = match session.relational() {
                Some(rel) => rel.list_databases().await?,
                None => Vec::new(),
            };
            Ok::<_, navidog_db::DbError>((session, databases))
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
                    Ok((session, mut databases)) => {
                        if custom.custom_list {
                            databases.retain(|d| custom.items.iter().any(|i| &i.name == d));
                        }
                        let version = session.server_version().to_string();
                        let session = Arc::new(Mutex::new(Some(session)));
                        let keepalive =
                            keepalive.map(|secs| start_keepalive(session.clone(), secs));
                        this.live
                            .insert(task_id.clone(), Live { session, keepalive });
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
            let session = live.session.clone();
            drop(live);
            runtime::spawn(async move {
                if let Some(s) = session.lock().await.take() {
                    let _ = s.close().await;
                }
            });
        }
        self.states.insert(id.to_string(), ConnState::Closed);
        cx.emit(SessionsEvent::Closed(id.to_string()));
        cx.notify();
    }

    /// 重新列出数据库。
    pub fn refresh(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(live) = self.live.get(id) else {
            return;
        };
        let session = live.session.clone();
        let id = id.to_string();
        let job = runtime::spawn(async move {
            let mut guard = session.lock().await;
            match guard.as_mut().and_then(|s| s.relational()) {
                Some(rel) => rel.list_databases().await,
                None => Ok(Vec::new()),
            }
        });
        let task_id = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let Ok(Ok(dbs)) = job.await else { return };
            let _ = this.update(cx, |this, cx| {
                if let Some(ConnState::Open { databases, .. }) = this.states.get_mut(&task_id) {
                    *databases = dbs;
                    cx.notify();
                }
            });
        });
        self.tasks.insert(id, task);
    }
}

fn start_keepalive(session: SharedSession, secs: u32) -> tokio::task::JoinHandle<()> {
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
