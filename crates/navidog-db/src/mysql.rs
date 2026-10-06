//! MySQL / MariaDB 驱动（mysql_async，rustls）。

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use mysql_async::prelude::Queryable as _;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts};

use crate::driver::{
    CancelHandle, Column, DbError, DbResult, Driver, DriverInfo, Family, Relational, ResultSet,
    Session,
};
use crate::params::ConnectParams;

pub struct MySqlDriver {
    info: DriverInfo,
}

impl MySqlDriver {
    pub fn mysql() -> Self {
        Self::new("mysql", "MySQL")
    }

    pub fn mariadb() -> Self {
        Self::new("mariadb", "MariaDB")
    }

    fn new(id: &'static str, name: &'static str) -> Self {
        Self {
            info: DriverInfo {
                id: id.into(),
                name: name.into(),
                family: Family::Relational,
                default_port: Some(3306),
                supports_ssl: true,
                supports_ssh: true,
            },
        }
    }
}

fn err(e: mysql_async::Error) -> DbError {
    match e {
        mysql_async::Error::Server(s) => DbError::coded(s.code, s.message),
        other => DbError::msg(other.to_string()),
    }
}

fn split_queries(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn build_opts(p: &ConnectParams) -> Opts {
    let mut b = OptsBuilder::default()
        .ip_or_hostname(p.host.clone())
        .tcp_port(p.port)
        .user(Some(p.user.clone()))
        .pass(p.password.clone())
        .db_name(p.database.clone())
        .init(split_queries(&p.initial_queries))
        .socket(p.socket.clone())
        .prefer_socket(p.socket.is_some());
    if p.compression {
        b = b.compression(mysql_async::Compression::default());
    }
    if let Some(ssl) = &p.ssl {
        let mut opts = SslOpts::default()
            // Navicat 的“验证针对 CA 的服务器证书”只校验证书链，不校验主机名（VERIFY_CA）
            .with_danger_skip_domain_validation(true)
            .with_danger_accept_invalid_certs(!ssl.verify_server_cert);
        if let Some(ca) = &ssl.ca_cert {
            opts = opts.with_root_certs(vec![ca.clone().into()]);
        }
        if let (Some(cert), Some(key)) = (&ssl.client_cert, &ssl.client_key) {
            opts = opts.with_client_identity(Some(mysql_async::ClientIdentity::new(
                cert.clone().into(),
                key.clone().into(),
            )));
        }
        b = b.ssl_opts(opts);
    }
    Opts::from(b)
}

impl Driver for MySqlDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        async move {
            let opts = build_opts(&params);
            let mut conn = tokio::time::timeout(params.timeout(), Conn::new(opts.clone()))
                .await
                .map_err(|_| DbError::msg("连接超时"))?
                .map_err(err)?;
            let version: Option<String> =
                conn.query_first("SELECT VERSION()").await.map_err(err)?;
            Ok(Box::new(MySqlSession {
                conn,
                opts,
                version: version.unwrap_or_default(),
            }) as Box<dyn Session>)
        }
        .boxed()
    }
}

pub struct MySqlSession {
    conn: Conn,
    opts: Opts,
    version: String,
}

impl Session for MySqlSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move { self.conn.ping().await.map_err(err) }.boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        // 另开一个会话执行 KILL QUERY
        let opts = self.opts.clone();
        let id = self.conn.id();
        Some(Arc::new(move || {
            let opts = opts.clone();
            async move {
                let mut killer = Conn::new(opts).await.map_err(err)?;
                killer
                    .query_drop(format!("KILL QUERY {id}"))
                    .await
                    .map_err(err)?;
                killer.disconnect().await.map_err(err)
            }
            .boxed()
        }))
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        async move { self.conn.disconnect().await.map_err(err) }.boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

impl Relational for MySqlSession {
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>> {
        async move { self.conn.query("SHOW DATABASES").await.map_err(err) }.boxed()
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            let mut result = self.conn.query_iter(sql).await.map_err(err)?;
            let mut sets = Vec::new();
            while !result.is_empty() {
                let columns: Vec<Column> = result
                    .columns_ref()
                    .iter()
                    .map(|c| Column {
                        name: c.name_str().into_owned(),
                        type_name: format!("{:?}", c.column_type()),
                    })
                    .collect();
                let rows: Vec<mysql_async::Row> = result.collect().await.map_err(err)?;
                let rows = rows
                    .into_iter()
                    .map(|row| row.unwrap().into_iter().map(value_text).collect())
                    .collect();
                let affected = columns.is_empty().then(|| result.affected_rows());
                sets.push(ResultSet {
                    columns,
                    rows,
                    affected_rows: affected,
                });
            }
            Ok(sets)
        }
        .boxed()
    }
}

/// 文本协议下除 NULL 外的值都是字节串。
fn value_text(v: mysql_async::Value) -> Option<String> {
    use mysql_async::Value::*;
    match v {
        NULL => None,
        Bytes(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        Int(i) => Some(i.to_string()),
        UInt(u) => Some(u.to_string()),
        Float(f) => Some(f.to_string()),
        Double(d) => Some(d.to_string()),
        other => Some(other.as_sql(true).trim_matches('\'').to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::split_queries;

    #[test]
    fn splits_initial_queries() {
        assert_eq!(split_queries(" SET a=1; ;SET b=2;"), ["SET a=1", "SET b=2"]);
        assert!(split_queries("  ").is_empty());
    }
}
