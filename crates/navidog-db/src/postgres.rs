//! PostgreSQL 驱动（tokio-postgres，native-tls）。

use futures::FutureExt as _;
use futures::future::BoxFuture;
use native_tls::{Certificate, Identity, TlsConnector};
use navidog_core::connection::SslMode;
use postgres_native_tls::MakeTlsConnector;
use tokio_postgres::config::SslMode as PgSslMode;
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::{CancelToken, Client, Config, NoTls, SimpleQueryMessage, Socket};

use std::sync::Arc;

use crate::driver::{
    CancelHandle, Column, DbError, DbResult, Driver, DriverInfo, Family, Relational, ResultSet,
    Session,
};
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
            },
        }
    }
}

fn err(e: tokio_postgres::Error) -> DbError {
    match e.as_db_error() {
        Some(db) => DbError::coded(db.code().code(), db.message()),
        None => DbError::msg(e.to_string()),
    }
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
        tracing::warn!("native-tls 不支持证书撤消列表，已忽略");
    }
    Ok(MakeTlsConnector::new(b.build().map_err(tls_err)?))
}

async fn connect_with<T>(cfg: &Config, tls: T) -> DbResult<Client>
where
    T: MakeTlsConnect<Socket> + 'static + Send,
    T::TlsConnect: Send,
    T::Stream: Send + 'static,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let (client, connection) = cfg.connect(tls).await.map_err(err)?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::debug!("PostgreSQL 连接结束: {e}");
        }
    });
    Ok(client)
}

/// 按 libpq 的 sslmode 语义建立连接。
async fn connect(p: &ConnectParams) -> DbResult<Client> {
    let mut cfg = base_config(p)?;
    let Some(ssl) = &p.ssl else {
        cfg.ssl_mode(PgSslMode::Disable);
        return connect_with(&cfg, NoTls).await;
    };
    // ponytail: 经 SSH 隧道时主机名是 127.0.0.1，verify-full 退化为只校验证书链
    let tunneled = p.tls_host.is_some();
    match ssl.mode {
        SslMode::Allow => {
            cfg.ssl_mode(PgSslMode::Disable);
            match connect_with(&cfg, NoTls).await {
                Ok(c) => Ok(c),
                Err(_) => {
                    cfg.ssl_mode(PgSslMode::Require);
                    connect_with(&cfg, tls(ssl, false, false)?).await
                }
            }
        }
        SslMode::Prefer => {
            cfg.ssl_mode(PgSslMode::Prefer);
            connect_with(&cfg, tls(ssl, false, false)?).await
        }
        SslMode::Require => {
            cfg.ssl_mode(PgSslMode::Require);
            connect_with(&cfg, tls(ssl, false, false)?).await
        }
        SslMode::VerifyCa => {
            cfg.ssl_mode(PgSslMode::Require);
            connect_with(&cfg, tls(ssl, true, false)?).await
        }
        SslMode::VerifyFull => {
            cfg.ssl_mode(PgSslMode::Require);
            connect_with(&cfg, tls(ssl, true, !tunneled)?).await
        }
    }
}

impl Driver for PostgresDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        async move {
            let client = tokio::time::timeout(params.timeout(), connect(&params))
                .await
                .map_err(|_| DbError::msg("连接超时"))??;
            for q in params
                .initial_queries
                .split(';')
                .map(str::trim)
                .filter(|q| !q.is_empty())
            {
                client.batch_execute(q).await.map_err(err)?;
            }
            let version = first_value(&client, "SHOW server_version")
                .await?
                .unwrap_or_default();
            let cancel = client.cancel_token();
            Ok(Box::new(PgSession {
                client,
                cancel,
                version,
            }) as Box<dyn Session>)
        }
        .boxed()
    }
}

async fn first_value(client: &Client, sql: &str) -> DbResult<Option<String>> {
    let msgs = client.simple_query(sql).await.map_err(err)?;
    Ok(msgs.into_iter().find_map(|m| match m {
        SimpleQueryMessage::Row(r) => r.get(0).map(String::from),
        _ => None,
    }))
}

pub struct PgSession {
    client: Client,
    cancel: CancelToken,
    version: String,
}

impl Session for PgSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move {
            self.client
                .simple_query("SELECT 1")
                .await
                .map(|_| ())
                .map_err(err)
        }
        .boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        let token = self.cancel.clone();
        Some(Arc::new(move || {
            let token = token.clone();
            // ponytail: 取消请求走明文连接；服务器强制 SSL 时需要改为 TLS 连接器
            async move { token.cancel_query(NoTls).await.map_err(err) }.boxed()
        }))
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
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

impl Relational for PgSession {
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>> {
        async move {
            let msgs = self
                .client
                .simple_query(
                    "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY datname",
                )
                .await
                .map_err(err)?;
            Ok(msgs
                .into_iter()
                .filter_map(|m| match m {
                    SimpleQueryMessage::Row(r) => r.get(0).map(String::from),
                    _ => None,
                })
                .collect())
        }
        .boxed()
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            let msgs = self.client.simple_query(sql).await.map_err(err)?;
            let mut sets = Vec::new();
            let mut current = ResultSet::default();
            for m in msgs {
                match m {
                    SimpleQueryMessage::RowDescription(cols) => {
                        current.columns = cols
                            .iter()
                            .map(|c| Column {
                                name: c.name().to_string(),
                                type_name: String::new(),
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
            Ok(sets)
        }
        .boxed()
    }
}
