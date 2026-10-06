//! 驱动层：trait、tokio 运行时、内置驱动（MySQL/MariaDB、PostgreSQL、SQLite）与 SSH 隧道。

pub mod dialect;
pub mod driver;
mod metadata;
pub mod mysql;
pub mod params;
pub mod postgres;
pub mod runtime;
pub mod sessions;
pub mod sql;
pub mod sqlite;
pub mod ssh;

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui_kit::App;
use navidog_core::registry::Source;

pub use driver::*;
pub use params::{ConnectParams, Secrets};

/// 注册内置驱动。
pub fn init(cx: &mut App) {
    let mut registry = DriverRegistry::default();
    let builtin: [Arc<dyn Driver>; 4] = [
        Arc::new(mysql::MySqlDriver::mysql()),
        Arc::new(mysql::MySqlDriver::mariadb()),
        Arc::new(postgres::PostgresDriver::default()),
        Arc::new(sqlite::SqliteDriver::default()),
    ];
    for d in builtin {
        let id = d.info().id.clone();
        registry
            .0
            .register(id, Source::Builtin, d)
            .expect("内置驱动不会重复");
    }
    cx.set_global(registry);
    sessions::init(cx);
}

/// 建立会话：启用 SSH 时先开隧道，再让驱动连接隧道的本地端口。
/// 必须在 [`runtime::spawn`] 中执行。
pub fn connect(
    driver: Arc<dyn Driver>,
    mut params: ConnectParams,
) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
    async move {
        let Some(ssh) = params.ssh.take() else {
            return driver.connect(params).await;
        };
        let tunnel = tokio::time::timeout(
            params.timeout(),
            ssh::open(&ssh, params.host.clone(), params.port),
        )
        .await
        .map_err(|_| DbError::msg("SSH: 连接超时"))??;
        params.tls_host = Some(std::mem::replace(&mut params.host, "127.0.0.1".into()));
        params.port = tunnel.local_port;
        let inner = driver.connect(params).await?;
        Ok(Box::new(Tunneled {
            inner,
            _tunnel: tunnel,
        }) as Box<dyn Session>)
    }
    .boxed()
}

/// 持有隧道的会话；会话关闭时隧道随之释放。
struct Tunneled {
    inner: Box<dyn Session>,
    _tunnel: ssh::Tunnel,
}

impl Session for Tunneled {
    fn server_version(&self) -> &str {
        self.inner.server_version()
    }
    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        self.inner.ping()
    }
    fn cancel_handle(&self) -> Option<CancelHandle> {
        self.inner.cancel_handle()
    }
    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        let Tunneled { inner, _tunnel } = *self;
        async move {
            let res = inner.close().await;
            drop(_tunnel);
            res
        }
        .boxed()
    }
    fn relational(&mut self) -> Option<&mut dyn Relational> {
        self.inner.relational()
    }
    fn document(&mut self) -> Option<&mut dyn Document> {
        self.inner.document()
    }
    fn key_value(&mut self) -> Option<&mut dyn KeyValue> {
        self.inner.key_value()
    }
}

/// 测试连接：建立会话、执行一次心跳并关闭。
pub async fn test_connection(driver: Arc<dyn Driver>, params: ConnectParams) -> DbResult<String> {
    let mut session = connect(driver, params).await?;
    session.ping().await?;
    let version = session.server_version().to_string();
    session.close().await?;
    Ok(version)
}
