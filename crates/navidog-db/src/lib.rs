//! Drivers: the traits, the tokio runtime, the built-in drivers (MySQL/MariaDB, PostgreSQL,
//! SQLite) and the SSH tunnel.

rust_i18n::i18n!("locales", fallback = "en");
// Track locale edits in Cargo as well as the translation proc-macro.
const _: &str = include_str!("../locales/database.yml");

pub mod dialect;
pub mod driver;
mod execution;
mod keywords;
pub mod log;
mod metadata;
pub mod mysql;
pub mod outline;
pub mod params;
pub mod postgres;
pub mod runtime;
pub mod sessions;
pub mod sql;
pub mod sqlite;
pub mod ssh;
pub mod table_design;

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui_kit::App;
use navidog_core::registry::Source;

pub use driver::*;
pub use metadata::referenced_values_sql;
pub use params::{ConnectParams, Secrets};

/// Tests that set `NAVIDOG_HOME` share this lock. The variable is process-global,
/// and a lock inside one module does not cover the other tests in this crate.
#[cfg(test)]
pub(crate) fn lock_config_home() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// Register the built-in drivers.
pub fn init(cx: &mut App) {
    let _ = _rust_i18n_backend();
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
            .expect("built-in driver IDs are unique");
    }
    cx.set_global(registry);
    sessions::init(cx);
    outline::init(cx);
}

/// Open a session. With SSH, open the tunnel first and connect the driver to its local port.
/// Must run inside [`runtime::spawn`].
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
        .map_err(|_| DbError::msg("SSH: connection timed out"))??;
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

/// A session that owns its tunnel. Dropping the session drops the tunnel.
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
    fn session_id(&self) -> Option<u64> {
        self.inner.session_id()
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

/// Test a connection: open a session, ping once, and close it.
pub async fn test_connection(driver: Arc<dyn Driver>, params: ConnectParams) -> DbResult<String> {
    let mut session = connect(driver, params).await?;
    session.ping().await?;
    let version = session.server_version().to_string();
    session.close().await?;
    Ok(version)
}
