//! PostgreSQL driver (tokio-postgres, native-tls).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use native_tls::{Certificate, Identity, TlsConnector};
use navidog_core::connection::{HostPreference, SslMode};
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::config::{SslMode as PgSslMode, TargetSessionAttrs};
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::types::Kind;
use tokio_postgres::{
    AsyncMessage, CancelToken, Client, Config, NoTls, SimpleQueryMessage, Socket,
};

use crate::dialect::Dialect;
use crate::driver::{
    CancelHandle, Column, ColumnOrigin, DbError, DbResult, Driver, DriverInfo, Family, Relational,
    ResultSet, Session,
};
use crate::execution::{Cancellation, RecoveryState, interrupted};
use crate::log::{Tag, logged};
use crate::params::{ConnectParams, SslParams};

pub struct PostgresDriver {
    info: DriverInfo,
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self {
            info: DriverInfo {
                id: "postgres".into(),
                name: "PostgreSQL".into(),
                family: Family::Relational,
                default_port: Some(5432),
                supports_ssl: true,
                supports_ssh: true,
                per_database_sessions: true,
            },
        }
    }
}

fn err(e: tokio_postgres::Error) -> DbError {
    if let Some(db) = e.as_db_error() {
        let mut out = DbError::coded(db.code().code(), db.message());
        // Most SQL errors leave the session usable. A few mean the backend is already gone
        // (idle timeout, admin terminate). Query cancel (57014) is not one of them.
        out.disconnected = session_closed_sqlstate(db.code().code());
        return out;
    }
    // Closed, or an IO failure with no SQLSTATE.
    let mut out = DbError::msg(e.to_string());
    out.disconnected =
        e.is_closed() || std::error::Error::source(&e).is_some_and(|s| s.is::<std::io::Error>());
    out
}

/// SQLSTATEs after which the backend will not take another command.
fn session_closed_sqlstate(code: &str) -> bool {
    code.starts_with("08") || matches!(code, "57P01" | "57P02" | "57P03" | "57P04" | "57P05")
}

fn tls_err(e: impl std::fmt::Display) -> DbError {
    DbError::msg(format!("SSL: {e}"))
}

fn base_config(p: &ConnectParams) -> DbResult<Config> {
    let mut cfg: Config = if p.params.trim().is_empty() {
        Config::new()
    } else {
        p.params
            .parse()
            .map_err(|e| DbError::msg(format!("连接参数无效: {e}")))?
    };
    cfg.host(&p.host)
        .port(p.port)
        .user(&p.user)
        .dbname(p.database.as_deref().unwrap_or("postgres"))
        .connect_timeout(p.timeout());
    // ponytail: the SSH tunnel forwards only `host`, so members are skipped when tunneled
    if p.tls_host.is_none() {
        for (host, port) in &p.members {
            cfg.host(host).port(*port);
        }
    }
    cfg.target_session_attrs(match p.host_preference {
        HostPreference::Any => TargetSessionAttrs::Any,
        HostPreference::ReadWrite => TargetSessionAttrs::ReadWrite,
        HostPreference::ReadOnly => TargetSessionAttrs::ReadOnly,
    });
    if let Some(pw) = &p.password {
        cfg.password(pw);
    }
    Ok(cfg)
}

fn tls(ssl: &SslParams, verify_chain: bool, verify_host: bool) -> DbResult<MakeTlsConnector> {
    let mut b = TlsConnector::builder();
    b.danger_accept_invalid_certs(!verify_chain)
        .danger_accept_invalid_hostnames(!verify_host);
    if let Some(ca) = &ssl.ca_cert {
        let pem = std::fs::read(ca).map_err(tls_err)?;
        b.add_root_certificate(Certificate::from_pem(&pem).map_err(tls_err)?);
    }
    if let (Some(cert), Some(key)) = (&ssl.client_cert, &ssl.client_key) {
        let cert = std::fs::read(cert).map_err(tls_err)?;
        let key = std::fs::read(key).map_err(tls_err)?;
        b.identity(Identity::from_pkcs8(&cert, &key).map_err(tls_err)?);
    }
    if ssl.crl.is_some() {
        tracing::warn!("native-tls does not support certificate revocation lists; ignored");
    }
    Ok(MakeTlsConnector::new(b.build().map_err(tls_err)?))
}

/// NOTICE and WARNING messages from the server, collected while a statement runs.
type Notices = Arc<Mutex<Vec<String>>>;

async fn connect_with<T>(cfg: &Config, tls: T, notices: Notices) -> DbResult<Client>
where
    T: MakeTlsConnect<Socket> + 'static + Send,
    T::TlsConnect: Send,
    T::Stream: Send + 'static,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let (client, mut connection) = cfg.connect(tls).await.map_err(err)?;
    tokio::spawn(async move {
        let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(message) = messages.next().await {
            match message {
                Ok(AsyncMessage::Notice(n)) => notices
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(format!("{}: {}", n.severity(), n.message())),
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("PostgreSQL connection ended: {e}");
                    break;
                }
            }
        }
    });
    Ok(client)
}

/// Connects following libpq's sslmode semantics. Returns the TLS connector when TLS may be in
/// use, so cancel requests go the same way.
async fn connect(
    p: &ConnectParams,
    notices: Notices,
) -> DbResult<(Client, Option<MakeTlsConnector>)> {
    let mut cfg = base_config(p)?;
    let Some(ssl) = &p.ssl else {
        cfg.ssl_mode(PgSslMode::Disable);
        return Ok((connect_with(&cfg, NoTls, notices).await?, None));
    };
    // ponytail: through an SSH tunnel the host is 127.0.0.1, so verify-full only checks the chain
    let tunneled = p.tls_host.is_some();
    let (mode, connector) = match ssl.mode {
        SslMode::Allow => {
            cfg.ssl_mode(PgSslMode::Disable);
            if let Ok(c) = connect_with(&cfg, NoTls, notices.clone()).await {
                return Ok((c, None));
            }
            (PgSslMode::Require, tls(ssl, false, false)?)
        }
        SslMode::Prefer => (PgSslMode::Prefer, tls(ssl, false, false)?),
        SslMode::Require => (PgSslMode::Require, tls(ssl, false, false)?),
        SslMode::VerifyCa => (PgSslMode::Require, tls(ssl, true, false)?),
        SslMode::VerifyFull => (PgSslMode::Require, tls(ssl, true, !tunneled)?),
    };
    cfg.ssl_mode(mode);
    let client = connect_with(&cfg, connector.clone(), notices).await?;
    Ok((client, Some(connector)))
}

/// Handshake, initial queries, and the backend pid. Shared by the first connect and a reconnect.
async fn establish(
    params: &ConnectParams,
    notices: Notices,
) -> DbResult<(Client, Option<MakeTlsConnector>, String, u64)> {
    let (client, tls) = tokio::time::timeout(params.timeout(), connect(params, notices))
        .await
        .map_err(|_| DbError::msg("连接超时"))??;
    for q in crate::sql::split_statements(&params.initial_queries, Dialect::Postgres) {
        client.batch_execute(&q).await.map_err(err)?;
    }
    let msgs = client
        .simple_query("SELECT current_setting('server_version'), pg_backend_pid()")
        .await
        .map_err(err)?;
    let (version, pid) = msgs
        .into_iter()
        .find_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some((
                r.get(0).unwrap_or_default().to_string(),
                r.get(1).and_then(|p| p.parse().ok()).unwrap_or(0),
            )),
            _ => None,
        })
        .unwrap_or_default();
    Ok((client, tls, version, pid))
}

impl Driver for PostgresDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        let mut tag = Tag::new(&params.name, &self.info.name);
        async move {
            let notices = Notices::default();
            let (client, tls, version, pid) = establish(&params, notices.clone()).await?;
            tag.session = pid;
            let cancel = client.cancel_token();
            let cancellation = Cancellation::default();
            cancellation.replace(cancel_target(cancel.clone(), tls.clone()));
            Ok(Box::new(PgSession {
                client,
                cancel,
                recovery: RecoveryState::new(&params.initial_queries, Dialect::Postgres),
                cancellation,
                params,
                tls,
                version,
                notices,
                tag,
                origins: HashMap::new(),
                setup: None,
            }) as Box<dyn Session>)
        }
        .boxed()
    }
}

pub struct PgSession {
    client: Client,
    cancel: CancelToken,
    params: ConnectParams,
    tls: Option<MakeTlsConnector>,
    version: String,
    notices: Notices,
    tag: Tag,
    /// `(table oid, column number)` → where a result column comes from; `None` when unknown
    origins: HashMap<(u32, i16), Option<ColumnOrigin>>,
    /// Last successful `SET search_path`; replayed after a transparent reconnect.
    setup: Option<String>,
    recovery: RecoveryState,
    cancellation: Cancellation,
}

impl PgSession {
    async fn reconnect(&mut self) -> DbResult<()> {
        self.cancellation.clear();
        let (client, tls, _version, pid) = establish(&self.params, self.notices.clone()).await?;
        self.client = client;
        self.tls = tls;
        self.cancel = self.client.cancel_token();
        self.tag.session = pid;
        self.origins.clear();
        self.replay().await?;
        self.cancellation
            .replace(cancel_target(self.cancel.clone(), self.tls.clone()));
        Ok(())
    }

    fn remember(&mut self, sql: &str) {
        if let Some(setup) = crate::sql::session_setup(sql) {
            self.setup = Some(setup);
        }
    }

    async fn replay(&mut self) -> DbResult<()> {
        if let Some(sql) = self.setup.clone() {
            self.attempt(&sql).await?;
        }
        Ok(())
    }

    fn lost(&self, e: &DbError) -> bool {
        e.disconnected || self.client.is_closed()
    }

    async fn attempt(&mut self, sql: &str) -> DbResult<Vec<ResultSet>> {
        self.notices
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        let client = &self.client;
        let msgs = logged(&self.tag, sql, async {
            client.simple_query(sql).await.map_err(err)
        })
        .await?;
        let mut sets = Vec::new();
        let mut current = ResultSet::default();
        for m in msgs {
            match m {
                SimpleQueryMessage::RowDescription(cols) => {
                    current.columns = cols
                        .iter()
                        .map(|c| Column {
                            name: c.name().to_string(),
                            ..Column::default()
                        })
                        .collect();
                }
                SimpleQueryMessage::Row(r) => {
                    current
                        .rows
                        .push((0..r.len()).map(|i| r.get(i).map(String::from)).collect());
                }
                SimpleQueryMessage::CommandComplete(n) => {
                    if current.columns.is_empty() {
                        current.affected_rows = Some(n);
                    }
                    sets.push(std::mem::take(&mut current));
                }
                _ => {}
            }
        }
        let notices = std::mem::take(&mut *self.notices.lock().unwrap_or_else(|p| p.into_inner()));
        if let Some(last) = sets.last_mut() {
            last.notices = notices;
        } else if !notices.is_empty() {
            sets.push(ResultSet {
                notices,
                ..ResultSet::default()
            });
        }
        Ok(sets)
    }
}

fn cancel_target(token: CancelToken, tls: Option<MakeTlsConnector>) -> CancelHandle {
    Arc::new(move || {
        let (token, tls) = (token.clone(), tls.clone());
        async move {
            match tls {
                Some(tls) => token.cancel_query(tls).await,
                None => token.cancel_query(NoTls).await,
            }
            .map_err(err)
        }
        .boxed()
    })
}

impl Session for PgSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move {
            let result = match self
                .client
                .simple_query("SELECT 1")
                .await
                .map(|_| ())
                .map_err(err)
            {
                Err(e) if self.lost(&e) && self.recovery.can_reconnect() => {
                    tracing::warn!("PostgreSQL metadata session lost ({e}); reconnecting");
                    self.reconnect().await?;
                    self.client
                        .simple_query("SELECT 1")
                        .await
                        .map(|_| ())
                        .map_err(err)
                }
                other => other,
            };
            result.map_err(|e| {
                if self.lost(&e) {
                    self.cancellation.clear();
                    interrupted(e, false)
                } else {
                    e
                }
            })
        }
        .boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        Some(self.cancellation.handle())
    }

    fn session_id(&self) -> Option<u64> {
        Some(self.tag.session)
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        self.cancellation.clear();
        async move {
            drop(self.client);
            Ok(())
        }
        .boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

/// Statements `prepare` can describe; utility statements (SET, SHOW, DDL) cannot be prepared.
fn describable(sql: &str) -> bool {
    let word = navidog_util::sql_text::first_keyword(sql);
    ["SELECT", "WITH", "VALUES", "TABLE"]
        .iter()
        .any(|k| word.eq_ignore_ascii_case(k))
}

fn type_name(ty: &tokio_postgres::types::Type) -> String {
    match ty.kind() {
        Kind::Array(inner) => format!("{}[]", inner.name()),
        _ => ty.name().to_string(),
    }
}

impl PgSession {
    /// Column types and origins of a SELECT, from its prepared form. `None` when it cannot be
    /// prepared; the result is then shown without types and stays read-only.
    async fn describe(&mut self, sql: &str) -> Option<Vec<Column>> {
        let stmt = self.client.prepare(sql).await.ok()?;
        let wanted: Vec<(u32, i16)> = stmt
            .columns()
            .iter()
            .filter_map(|c| Some((c.table_oid()?, c.column_id()?)))
            .filter(|key| !self.origins.contains_key(key))
            .collect();
        if !wanted.is_empty() {
            let pairs = wanted
                .iter()
                .map(|(t, c)| format!("({t}, {c})"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT a.attrelid, a.attnum, n.nspname, c.relname, a.attname \
                 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE (a.attrelid, a.attnum) IN ({pairs})"
            );
            let client = &self.client;
            let msgs = logged(&self.tag, &sql, async {
                client.simple_query(&sql).await.map_err(err)
            })
            .await
            .ok()?;
            for key in &wanted {
                self.origins.insert(*key, None);
            }
            for m in msgs {
                if let SimpleQueryMessage::Row(r) = m {
                    let key = (
                        r.get(0).and_then(|v| v.parse().ok()).unwrap_or(0),
                        r.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                    );
                    let text = |i| r.get(i).unwrap_or_default().to_string();
                    self.origins.insert(
                        key,
                        Some(ColumnOrigin {
                            schema: text(2),
                            table: text(3),
                            column: text(4),
                        }),
                    );
                }
            }
        }
        Some(
            stmt.columns()
                .iter()
                .map(|c| Column {
                    name: c.name().to_string(),
                    type_name: type_name(c.type_()),
                    origin: c
                        .table_oid()
                        .zip(c.column_id())
                        .and_then(|key| self.origins.get(&key).cloned().flatten()),
                })
                .collect(),
        )
    }
}

impl Relational for PgSession {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            self.recovery.observe(sql);
            let result = self.attempt(sql).await.map_err(|e| {
                if self.lost(&e) {
                    self.cancellation.clear();
                    interrupted(e, true)
                } else {
                    e
                }
            });
            if result.is_ok() {
                self.remember(sql);
            }
            result
        }
        .boxed()
    }

    fn execute_read<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            let result = match self.attempt(sql).await {
                Err(e) if self.lost(&e) && self.recovery.can_reconnect() => {
                    self.reconnect().await?;
                    self.attempt(sql).await
                }
                other => other,
            };
            result.map_err(|e| {
                if self.lost(&e) {
                    self.cancellation.clear();
                    interrupted(e, false)
                } else {
                    e
                }
            })
        }
        .boxed()
    }

    fn execute_statement<'a>(
        &'a mut self,
        sql: &'a str,
    ) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            let described = if describable(sql) {
                self.describe(sql).await
            } else {
                None
            };
            let mut sets = self.execute(sql).await?;
            if let (Some(columns), Some(first)) = (described, sets.first_mut())
                && columns.len() == first.columns.len()
            {
                first.columns = columns;
            }
            Ok(sets)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::session_closed_sqlstate;

    #[test]
    fn query_cancel_is_not_a_dropped_session() {
        assert!(!session_closed_sqlstate("57014"));
        assert!(!session_closed_sqlstate("42601"));
        assert!(session_closed_sqlstate("57P01"));
        assert!(session_closed_sqlstate("57P05"));
        assert!(session_closed_sqlstate("08006"));
    }
}
