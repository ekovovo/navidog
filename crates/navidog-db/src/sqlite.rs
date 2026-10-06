//! SQLite 驱动（rusqlite，bundled）。rusqlite 是阻塞 API，调用都放到 `spawn_blocking`。

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use rusqlite::types::ValueRef;
use rusqlite::{OpenFlags, params};

use crate::driver::{
    CancelHandle, Column, DbError, DbResult, Driver, DriverInfo, Family, Relational, ResultSet,
    Session,
};
use crate::params::ConnectParams;

pub struct SqliteDriver {
    info: DriverInfo,
}

impl Default for SqliteDriver {
    fn default() -> Self {
        Self {
            info: DriverInfo {
                id: "sqlite".into(),
                name: "SQLite".into(),
                family: Family::Relational,
                default_port: None,
                supports_ssl: false,
                supports_ssh: false,
                per_database_sessions: false,
            },
        }
    }
}

fn err(e: rusqlite::Error) -> DbError {
    match &e {
        rusqlite::Error::SqliteFailure(code, msg) => DbError::coded(
            code.extended_code,
            msg.clone().unwrap_or_else(|| code.to_string()),
        ),
        _ => DbError::msg(e.to_string()),
    }
}

type Shared = Arc<Mutex<rusqlite::Connection>>;

async fn blocking<T: Send + 'static>(
    conn: &Shared,
    f: impl FnOnce(&mut rusqlite::Connection) -> DbResult<T> + Send + 'static,
) -> DbResult<T> {
    let conn = conn.clone();
    tokio::task::spawn_blocking(move || f(&mut conn.lock().unwrap_or_else(|p| p.into_inner())))
        .await
        .map_err(|e| DbError::msg(e.to_string()))?
}

impl Driver for SqliteDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        async move {
            let file = params
                .file
                .clone()
                .ok_or_else(|| DbError::msg("未指定数据库文件"))?;
            let session = tokio::task::spawn_blocking(move || -> DbResult<SqliteSession> {
                if !params.create_new && !file.exists() {
                    return Err(DbError::msg(format!(
                        "数据库文件不存在: {}",
                        file.display()
                    )));
                }
                let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI;
                if params.create_new {
                    flags |= OpenFlags::SQLITE_OPEN_CREATE;
                }
                let conn = rusqlite::Connection::open_with_flags(&file, flags).map_err(err)?;
                for (name, path) in &params.attached {
                    conn.execute(
                        "ATTACH DATABASE ?1 AS ?2",
                        params![path.to_string_lossy(), name],
                    )
                    .map_err(err)?;
                }
                if !params.initial_queries.trim().is_empty() {
                    conn.execute_batch(&params.initial_queries).map_err(err)?;
                }
                let version: String = conn
                    .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                    .map_err(err)?;
                let interrupt = Arc::new(conn.get_interrupt_handle());
                Ok(SqliteSession {
                    conn: Arc::new(Mutex::new(conn)),
                    interrupt,
                    version,
                })
            })
            .await
            .map_err(|e| DbError::msg(e.to_string()))??;
            Ok(Box::new(session) as Box<dyn Session>)
        }
        .boxed()
    }
}

pub struct SqliteSession {
    conn: Shared,
    interrupt: Arc<rusqlite::InterruptHandle>,
    version: String,
}

impl Session for SqliteSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move { blocking(&self.conn, |c| c.execute_batch("SELECT 1").map_err(err)).await }
            .boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        let handle = self.interrupt.clone();
        Some(Arc::new(move || {
            handle.interrupt();
            async { Ok(()) }.boxed()
        }))
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        async move {
            drop(self.conn);
            Ok(())
        }
        .boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

impl Relational for SqliteSession {
    fn dialect(&self) -> crate::dialect::Dialect {
        crate::dialect::Dialect::Sqlite
    }

    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>> {
        async move {
            blocking(&self.conn, |c| {
                let mut stmt = c.prepare("PRAGMA database_list").map_err(err)?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(1))
                    .map_err(err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?;
                Ok(names.into_iter().filter(|n| n != "temp").collect())
            })
            .await
        }
        .boxed()
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        let sql = sql.to_string();
        async move { blocking(&self.conn, move |c| execute_all(c, &sql)).await }.boxed()
    }
}

/// 逐条执行多语句 SQL，每条返回一个结果集。
fn execute_all(conn: &mut rusqlite::Connection, sql: &str) -> DbResult<Vec<ResultSet>> {
    let mut sets = Vec::new();
    for piece in crate::sql::split_statements(sql) {
        let mut stmt = conn.prepare(&piece).map_err(err)?;
        let columns: Vec<Column> = stmt
            .columns()
            .iter()
            .map(|c| Column {
                name: c.name().to_string(),
                type_name: c.decl_type().unwrap_or("").to_string(),
            })
            .collect();
        let mut set = ResultSet {
            columns,
            ..Default::default()
        };
        if set.columns.is_empty() {
            set.affected_rows = Some(stmt.raw_execute().map_err(err)? as u64);
        } else {
            let n = set.columns.len();
            let mut rows = stmt.raw_query();
            while let Some(row) = rows.next().map_err(err)? {
                set.rows
                    .push((0..n).map(|i| value_text(row.get_ref_unwrap(i))).collect());
            }
        }
        sets.push(set);
    }
    Ok(sets)
}

fn value_text(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Some(format!("(BLOB) {} bytes", b.len())),
    }
}
