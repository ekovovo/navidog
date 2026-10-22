//! MySQL / MariaDB driver (mysql_async, rustls).

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable as _;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts};

use crate::dialect::Dialect;
use crate::driver::{
    CancelHandle, Column, ColumnOrigin, DbError, DbResult, Driver, DriverInfo, Family, Relational,
    ResultSet, Session,
};
use crate::execution::{Cancellation, RecoveryState, interrupted};
use crate::log::{Tag, logged};
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
                per_database_sessions: false,
            },
        }
    }
}

fn err(e: mysql_async::Error) -> DbError {
    let disconnected = mysql_disconnected(&e);
    let mut out = match e {
        mysql_async::Error::Server(s) => DbError::coded(s.code, s.message),
        other => DbError::msg(other.to_string()),
    };
    out.disconnected = disconnected;
    out
}

/// The socket is gone or the protocol stream is corrupt. A server SQL error is not in here:
/// that connection can still be used.
fn mysql_disconnected(e: &mysql_async::Error) -> bool {
    use mysql_async::{DriverError, Error};
    matches!(
        e,
        Error::Io(_)
            | Error::Driver(
                DriverError::ConnectionClosed
                    | DriverError::PacketOutOfOrder
                    | DriverError::UnexpectedPacket { .. }
                    | DriverError::BadCompressedPacketHeader,
            )
    )
}

fn build_opts(p: &ConnectParams) -> Opts {
    let mut b = OptsBuilder::default()
        .ip_or_hostname(p.host.clone())
        .tcp_port(p.port)
        .user(Some(p.user.clone()))
        .pass(p.password.clone())
        .db_name(p.database.clone())
        .init(crate::sql::split_statements(
            &p.initial_queries,
            Dialect::MySql,
        ))
        .socket(p.socket.clone())
        .prefer_socket(p.socket.is_some());
    if p.compression {
        b = b.compression(mysql_async::Compression::default());
    }
    if let Some(ssl) = &p.ssl {
        let mut opts = SslOpts::default()
            // Navicat's "verify server certificate against CA" checks the chain only, not the
            // host name (VERIFY_CA)
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
        let mut tag = Tag::new(&params.name, &self.info.name);
        async move {
            let opts = build_opts(&params);
            let mut conn = tokio::time::timeout(params.timeout(), Conn::new(opts.clone()))
                .await
                .map_err(|_| DbError::msg("连接超时"))?
                .map_err(err)?;
            let version: Option<String> =
                conn.query_first("SELECT VERSION()").await.map_err(err)?;
            tag.session = conn.id() as u64;
            let cancellation = Cancellation::default();
            let cancel_opts = opts.clone();
            let id = conn.id();
            cancellation.replace(Arc::new(move || {
                kill_query(cancel_opts.clone(), id).boxed()
            }));
            Ok(Box::new(MySqlSession {
                conn,
                opts,
                version: version.unwrap_or_default(),
                read_timeout: params.read_timeout,
                connect_timeout: params.timeout(),
                tag,
                setup: None,
                recovery: RecoveryState::new(&params.initial_queries, Dialect::MySql),
                cancellation,
            }) as Box<dyn Session>)
        }
        .boxed()
    }
}

pub struct MySqlSession {
    conn: Conn,
    opts: Opts,
    version: String,
    read_timeout: Option<Duration>,
    connect_timeout: Duration,
    tag: Tag,
    /// Last successful `USE`; replayed after a transparent reconnect.
    setup: Option<String>,
    recovery: RecoveryState,
    cancellation: Cancellation,
}

impl MySqlSession {
    /// Opens a new connection with the options from the original connect and drops the dead one.
    async fn reconnect(&mut self) -> DbResult<()> {
        self.cancellation.clear();
        let conn = tokio::time::timeout(self.connect_timeout, Conn::new(self.opts.clone()))
            .await
            .map_err(|_| DbError::msg("连接超时"))?
            .map_err(err)?;
        drop(std::mem::replace(&mut self.conn, conn));
        self.tag.session = self.conn.id() as u64;
        self.replay().await?;
        let opts = self.opts.clone();
        let id = self.conn.id();
        self.cancellation
            .replace(Arc::new(move || kill_query(opts.clone(), id).boxed()));
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

    async fn attempt(&mut self, sql: &str) -> DbResult<Vec<ResultSet>> {
        let (opts, id, limit) = (self.opts.clone(), self.conn.id(), self.read_timeout);
        let tag = self.tag.clone();
        let run = query(&mut self.conn, sql);
        let Some(limit) = limit else {
            return logged(&tag, sql, run).await;
        };
        // On timeout the statement is killed and its interrupted result read off the wire,
        // so the session stays usable. A timeout is not a dropped connection.
        let guarded = async {
            tokio::pin!(run);
            tokio::select! {
                result = &mut run => result,
                _ = tokio::time::sleep(limit) => {
                    if let Err(e) = kill_query(opts, id).await {
                        tracing::warn!("KILL QUERY after the read timeout failed: {e}");
                    }
                    let _ = run.await;
                    Err(DbError::msg(format!("读取超时（{} 秒）", limit.as_secs())))
                }
            }
        };
        logged(&tag, sql, guarded).await
    }
}

/// Kills the statement session `id` is running, from a separate session.
async fn kill_query(opts: Opts, id: u32) -> DbResult<()> {
    let mut killer = Conn::new(opts).await.map_err(err)?;
    killer
        .query_drop(format!("KILL QUERY {id}"))
        .await
        .map_err(err)?;
    killer.disconnect().await.map_err(err)
}

impl Session for MySqlSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move {
            let result = match self.conn.ping().await.map_err(err) {
                Err(e) if e.disconnected && self.recovery.can_reconnect() => {
                    tracing::warn!("MySQL metadata session lost ({e}); reconnecting");
                    self.reconnect().await?;
                    self.conn.ping().await.map_err(err)
                }
                other => other,
            };
            result.map_err(|e| {
                if e.disconnected {
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
        Some(self.conn.id() as u64)
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        self.cancellation.clear();
        async move { self.conn.disconnect().await.map_err(err) }.boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

impl Relational for MySqlSession {
    fn dialect(&self) -> Dialect {
        Dialect::MySql
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        async move {
            self.recovery.observe(sql);
            let result = self.attempt(sql).await.map_err(|e| {
                if e.disconnected {
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
                Err(e) if e.disconnected && self.recovery.can_reconnect() => {
                    self.reconnect().await?;
                    self.attempt(sql).await
                }
                other => other,
            };
            result.map_err(|e| {
                if e.disconnected {
                    self.cancellation.clear();
                    interrupted(e, false)
                } else {
                    e
                }
            })
        }
        .boxed()
    }
}

async fn query(conn: &mut Conn, sql: &str) -> DbResult<Vec<ResultSet>> {
    let mut result = conn.query_iter(sql).await.map_err(err)?;
    let mut sets = Vec::new();
    while !result.is_empty() {
        let columns: Vec<Column> = result
            .columns_ref()
            .iter()
            .map(|c| {
                let table = c.org_table_str();
                Column {
                    name: c.name_str().into_owned(),
                    type_name: sql_type(c.column_type(), c.flags(), c.character_set()),
                    origin: (!table.is_empty()).then(|| ColumnOrigin {
                        schema: c.schema_str().into_owned(),
                        table: table.into_owned(),
                        column: c.org_name_str().into_owned(),
                    }),
                }
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
            notices: Vec::new(),
        });
    }
    Ok(sets)
}

/// The SQL type name of a result column, so type icons and number alignment work for query
/// results too. The protocol gives no length, so `varchar` has no `(n)`.
fn sql_type(ty: ColumnType, flags: ColumnFlags, charset: u16) -> String {
    use ColumnType::*;
    // Charset 63 is `binary`: the column holds bytes, not text
    let binary = charset == 63;
    let text_or = |text: &'static str, bytes: &'static str| if binary { bytes } else { text };
    let name = match ty {
        MYSQL_TYPE_TINY => "tinyint",
        MYSQL_TYPE_SHORT => "smallint",
        MYSQL_TYPE_INT24 => "mediumint",
        MYSQL_TYPE_LONG => "int",
        MYSQL_TYPE_LONGLONG => "bigint",
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_ENUM => "enum",
        MYSQL_TYPE_SET => "set",
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => text_or("varchar", "varbinary"),
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::ENUM_FLAG) => "enum",
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::SET_FLAG) => "set",
        MYSQL_TYPE_STRING => text_or("char", "binary"),
        MYSQL_TYPE_TINY_BLOB => text_or("tinytext", "tinyblob"),
        MYSQL_TYPE_BLOB => text_or("text", "blob"),
        MYSQL_TYPE_MEDIUM_BLOB => text_or("mediumtext", "mediumblob"),
        MYSQL_TYPE_LONG_BLOB => text_or("longtext", "longblob"),
        _ => "",
    };
    let numeric = matches!(
        ty,
        MYSQL_TYPE_TINY
            | MYSQL_TYPE_SHORT
            | MYSQL_TYPE_INT24
            | MYSQL_TYPE_LONG
            | MYSQL_TYPE_LONGLONG
            | MYSQL_TYPE_FLOAT
            | MYSQL_TYPE_DOUBLE
            | MYSQL_TYPE_DECIMAL
            | MYSQL_TYPE_NEWDECIMAL
    );
    if numeric && flags.contains(ColumnFlags::UNSIGNED_FLAG) {
        format!("{name} unsigned")
    } else {
        name.to_string()
    }
}

/// Over the text protocol every value except NULL is a byte string.
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
    use super::*;

    #[test]
    fn server_error_is_not_a_dropped_connection() {
        use mysql_async::{DriverError, Error, IoError, ServerError};
        let server = err(Error::Server(ServerError {
            code: 1064,
            message: "syntax".into(),
            state: "42000".into(),
        }));
        assert!(!server.disconnected);
        assert!(err(Error::Driver(DriverError::ConnectionClosed)).disconnected);
        assert!(err(Error::Driver(DriverError::PacketOutOfOrder)).disconnected);
        assert!(
            err(Error::Io(IoError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "reset",
            ))))
            .disconnected
        );
        // A bad statement, not a dead socket.
        assert!(!err(Error::Driver(DriverError::MixedParams)).disconnected);
    }

    #[test]
    fn protocol_types_become_sql_type_names() {
        let none = ColumnFlags::empty();
        assert_eq!(
            sql_type(ColumnType::MYSQL_TYPE_LONG, ColumnFlags::UNSIGNED_FLAG, 63),
            "int unsigned"
        );
        assert_eq!(
            sql_type(ColumnType::MYSQL_TYPE_VAR_STRING, none, 255),
            "varchar"
        );
        assert_eq!(sql_type(ColumnType::MYSQL_TYPE_BLOB, none, 63), "blob");
        assert_eq!(sql_type(ColumnType::MYSQL_TYPE_BLOB, none, 255), "text");
        assert_eq!(
            sql_type(ColumnType::MYSQL_TYPE_STRING, ColumnFlags::ENUM_FLAG, 255),
            "enum"
        );
    }
}
