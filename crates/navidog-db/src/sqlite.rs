//! SQLite driver (rusqlite, bundled). rusqlite is blocking, so calls run in `spawn_blocking`.

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use rusqlite::types::ValueRef;
use rusqlite::{OpenFlags, params};
use tokio::sync::watch;

use crate::dialect::Dialect;
use crate::driver::{
    CancelHandle, Column, ColumnOrigin, DbError, DbResult, Driver, DriverInfo, Family, Relational,
    ResultSet, Session,
};
use crate::log::{Tag, logged};
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
    workers: &watch::Sender<bool>,
    f: impl FnOnce(&mut rusqlite::Connection) -> DbResult<T> + Send + 'static,
) -> DbResult<T> {
    let conn = conn.clone();
    // Register before spawning. Dropping this async wait must not detach an untracked worker.
    let registration = workers.subscribe();
    tokio::task::spawn_blocking(move || {
        let result = {
            let mut guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let retired = *registration.borrow();
            if retired {
                Err(DbError::msg(
                    rust_i18n::t!("database.sqlite_queued_session_closed").to_string(),
                )
                .with_session_loss(false))
            } else {
                f(&mut guard)
            }
        };
        drop(conn);
        drop(registration);
        result
    })
    .await
    .map_err(|e| DbError::msg(e.to_string()))?
}

fn worker_scope() -> watch::Sender<bool> {
    watch::channel(false).0
}

impl Driver for SqliteDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        let tag = Tag::new(&params.name, &self.info.name);
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
                    tag,
                    workers: worker_scope(),
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
    tag: Tag,
    workers: watch::Sender<bool>,
}

impl Session for SqliteSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async move {
            blocking(&self.conn, &self.workers, |c| {
                c.execute_batch("SELECT 1").map_err(err)
            })
            .await
        }
        .boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        let handle = self.interrupt.clone();
        Some(Arc::new(move || {
            handle.interrupt();
            async { Ok(()) }.boxed()
        }))
    }

    fn session_id(&self) -> Option<u64> {
        None
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        self.workers.send_replace(true);
        async move {
            // A canceled execute future can leave a queued/running spawn_blocking task. Wait
            // for its actual completion before the operation retirement fence is released.
            self.workers.closed().await;
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
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        let owned = sql.to_string();
        let conn = self.conn.clone();
        let tag = self.tag.clone();
        let workers = self.workers.clone();
        async move {
            logged(
                &tag,
                sql,
                blocking(&conn, &workers, move |c| execute_all(c, &owned)),
            )
            .await
        }
        .boxed()
    }
}

/// Runs a multi-statement script one statement at a time; each returns one result set.
fn execute_all(conn: &mut rusqlite::Connection, sql: &str) -> DbResult<Vec<ResultSet>> {
    let mut sets = Vec::new();
    for piece in crate::sql::split_statements(sql, Dialect::Sqlite) {
        let mut stmt = conn.prepare(&piece).map_err(err)?;
        let origins = stmt.columns_with_metadata();
        let columns: Vec<Column> = stmt
            .columns()
            .iter()
            .zip(origins)
            .map(|(c, m)| Column {
                name: c.name().to_string(),
                type_name: c.decl_type().unwrap_or("").to_string(),
                origin: match (m.database_name(), m.table_name(), m.origin_name()) {
                    (Some(schema), Some(table), Some(column)) => Some(ColumnOrigin {
                        schema: schema.to_string(),
                        table: table.to_string(),
                        column: column.to_string(),
                    }),
                    _ => None,
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_drains_dropped_execute_and_skips_work_queued_behind_the_connection_lock() {
        crate::runtime::runtime().block_on(async {
            let connection = rusqlite::Connection::open_in_memory().unwrap();
            connection
                .execute_batch("CREATE TABLE audit (n INTEGER)")
                .unwrap();
            let interrupt = Arc::new(connection.get_interrupt_handle());
            let conn = Arc::new(Mutex::new(connection));
            let mut session = Box::new(SqliteSession {
                conn: conn.clone(),
                interrupt,
                version: "test".into(),
                tag: Tag::new("test", "SQLite"),
                workers: worker_scope(),
            });
            let workers = session.workers.clone();
            let (locked, locked_wait) = tokio::sync::oneshot::channel();
            let (release, release_wait) = std::sync::mpsc::channel();
            let held = conn.clone();
            let holder = std::thread::spawn(move || {
                let guard = held.lock().unwrap();
                locked.send(()).unwrap();
                release_wait.recv().unwrap();
                drop(guard);
            });
            locked_wait.await.unwrap();
            let cancel = session.cancel_handle().unwrap();
            let mut execute = session.execute("INSERT INTO audit VALUES (1)");
            assert!(futures::poll!(&mut execute).is_pending());
            assert_eq!(workers.receiver_count(), 1);
            drop(execute);
            // SQLite's interrupt does not stop a statement that has not started. close must
            // therefore revoke queued work and keep waiting for the actual worker to exit.
            cancel().await.unwrap();
            let mut close = session.close();
            assert!(futures::poll!(&mut close).is_pending());
            assert_eq!(workers.receiver_count(), 1);
            release.send(()).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), close)
                .await
                .expect("close must complete after the queued worker exits")
                .unwrap();
            holder.join().unwrap();
            assert_eq!(workers.receiver_count(), 0);
            let rows: i64 = conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get(0))
                .unwrap();
            assert_eq!(rows, 0, "retired queued SQL must never start");
        });
    }
}
