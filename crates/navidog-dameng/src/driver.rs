//! Dameng driver. dm-pure is a synchronous client, so every protocol call runs on
//! `spawn_blocking`, with the same retirement fence as `navidog-sqlite`.

use std::sync::{Arc, Mutex};

use dm_pure::{ClientConfig, DmClient, ExecuteResult, Value};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use tokio::sync::watch;

use navidog_db::driver::{
    CancelHandle, Column, ColumnOrigin, DbError, DbResult, Driver, DriverInfo, Relational,
    ResultSet, Session,
};
use navidog_db::execution::{Cancellation, RecoveryState};
use navidog_db::log::{Tag, logged};
use navidog_db::params::ConnectParams;

use super::DamengDialect;
use crate::metadata;
use navidog_sql::Dialect as _;
use rust_i18n::t;

pub struct DamengDriver {
    info: DriverInfo,
}

impl Default for DamengDriver {
    fn default() -> Self {
        Self {
            info: DriverInfo {
                default_port: navidog_model::connection::driver_defaults("dameng").port,
                icon: "icons/db/dameng.svg",
                supports_network: true,
                supports_ssh: true,
                // dm-pure 1.2.4 has no TLS path; the SSL page is hidden (D3).
                supports_ssl: false,
                ..DriverInfo::relational("dameng", "Dameng", Arc::new(DamengDialect::default()))
            },
        }
    }
}

/// Maps a dm-pure error. Server codes arrive negative (-2501, -2103) and are shown as-is;
/// `-6852` means the connection is gone, which also covers the session killed by a cancel.
fn err(e: dm_pure::DmError) -> DbError {
    let disconnected = e.code == -6852;
    let mut out = if e.code == 0 {
        DbError::msg(e.msg)
    } else {
        DbError::coded(e.code, e.msg)
    };
    out.disconnected = disconnected;
    out
}

type Shared = Arc<Mutex<DmClient>>;

async fn blocking<T: Send + 'static>(
    client: &Shared,
    workers: &watch::Sender<bool>,
    f: impl FnOnce(&mut DmClient) -> DbResult<T> + Send + 'static,
) -> DbResult<T> {
    let client = client.clone();
    // Register before spawning. Dropping this async wait must not detach an untracked worker.
    let registration = workers.subscribe();
    tokio::task::spawn_blocking(move || {
        let result = {
            let mut guard = client.lock().unwrap_or_else(|p| p.into_inner());
            let retired = *registration.borrow();
            if retired {
                Err(
                    DbError::msg(t!("database.dameng_queued_session_closed").to_string())
                        .with_session_loss(false),
                )
            } else {
                f(&mut guard)
            }
        };
        drop(client);
        drop(registration);
        result
    })
    .await
    .map_err(|e| DbError::msg(e.to_string()))?
}

fn worker_scope() -> watch::Sender<bool> {
    watch::channel(false).0
}

/// Reads the first value of the first row; `None` when the query returned nothing.
fn scalar(client: &mut DmClient, sql: &str) -> DbResult<Option<Value>> {
    match client.execute(sql, &[]).map_err(err)? {
        ExecuteResult::Query { rows, .. } => {
            Ok(rows.into_iter().next().and_then(|r| r.into_iter().next()))
        }
        ExecuteResult::Affected(_) => Ok(None),
        ExecuteResult::Call { out, .. } => Ok(out.into_iter().next()),
    }
}

fn scalar_text(client: &mut DmClient, sql: &str) -> DbResult<Option<String>> {
    Ok(scalar(client, sql)?.and_then(|v| value_text(&v)))
}

fn scalar_i64(client: &mut DmClient, sql: &str) -> DbResult<Option<i64>> {
    Ok(scalar(client, sql)?.and_then(|v| v.as_i64()))
}

/// Handshake, capability probe, initial queries, version and session id of one connection.
/// Shared by the first connect and a reconnect.
fn establish(params: &ConnectParams) -> DbResult<(Shared, DamengDialect, String, u64)> {
    let cfg = ClientConfig {
        host: params.host.clone(),
        port: params.port,
        user: params.user.clone(),
        password: params.password.clone().unwrap_or_default(),
        timeout: params.timeout(),
        // Reconnecting is orchestrated by navidog-db's recovery macros, never by the driver.
        auto_reconnect: false,
        ..ClientConfig::default()
    };
    let mut client = DmClient::with_config(cfg);
    client.connect().map_err(err)?;
    // Probe failures never block the connection; the documented defaults stand instead
    // (140-dameng.md §3.2).
    let case_sensitive = scalar_i64(&mut client, "SELECT CASE_SENSITIVE()")
        .map(|v| v.unwrap_or(1) != 0)
        .unwrap_or_else(|e| {
            tracing::warn!("CASE_SENSITIVE() probe failed: {e}");
            true
        });
    let compatible_mode = scalar_text(
        &mut client,
        "SELECT PARA_VALUE FROM V$DM_INI WHERE PARA_NAME = 'COMPATIBLE_MODE'",
    )
    .ok()
    .flatten()
    .and_then(|v| v.trim().parse::<u8>().ok())
    .unwrap_or(0);
    let dialect = DamengDialect {
        case_sensitive,
        compatible_mode,
    };
    for q in navidog_db::sql::split_statements(&params.initial_queries, dialect.lex()) {
        client.execute(&q, &[]).map_err(err)?;
    }
    let banner = scalar_text(&mut client, "SELECT BANNER FROM V$VERSION WHERE ROWNUM = 1")?;
    let id_code = scalar_text(&mut client, "SELECT ID_CODE()")?;
    let version = [banner, id_code]
        .into_iter()
        .flatten()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let session_id = scalar_i64(&mut client, "SELECT SESSID()")?.unwrap_or(0);
    tracing::debug!(session = session_id, "Dameng session established");
    Ok((
        Arc::new(Mutex::new(client)),
        dialect,
        version,
        session_id.max(0) as u64,
    ))
}

impl Driver for DamengDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        let tag = Tag::new(&params.name, &self.info.name);
        async move {
            let recovery = RecoveryState::new(&params.initial_queries, &DamengDialect::default());
            let cancellation = Cancellation::default();
            let connect_params = params.clone();
            let (client, dialect, version, session_id) =
                tokio::task::spawn_blocking(move || establish(&connect_params))
                    .await
                    .map_err(|e| DbError::msg(e.to_string()))??;
            cancellation.replace(cancel_target(&params, session_id));
            Ok(Box::new(DamengSession {
                client,
                params,
                dialect: Arc::new(dialect),
                version,
                tag,
                workers: worker_scope(),
                recovery,
                cancellation,
                session_id,
                setup: None,
            }) as Box<dyn Session>)
        }
        .boxed()
    }
}

pub struct DamengSession {
    client: Shared,
    params: ConnectParams,
    dialect: Arc<DamengDialect>,
    version: String,
    tag: Tag,
    workers: watch::Sender<bool>,
    recovery: RecoveryState,
    cancellation: Cancellation,
    session_id: u64,
    /// Last successful `SET SCHEMA`; replayed after a transparent reconnect.
    setup: Option<String>,
}

impl DamengSession {
    async fn reconnect(&mut self) -> DbResult<()> {
        self.cancellation.clear();
        let params = self.params.clone();
        let (client, dialect, version, session_id) =
            tokio::task::spawn_blocking(move || establish(&params))
                .await
                .map_err(|e| DbError::msg(e.to_string()))??;
        self.client = client;
        self.dialect = Arc::new(dialect);
        self.version = version;
        self.tag.session = session_id;
        self.session_id = session_id;
        self.cancellation
            .replace(cancel_target(&self.params, session_id));
        if let Some(setup) = self.setup.clone() {
            let (owned, logged_sql, tag) = (setup.clone(), setup, self.tag.clone());
            logged(
                &tag,
                &logged_sql,
                blocking(&self.client, &self.workers, move |c| {
                    c.execute(&owned, &[]).map(|_| ()).map_err(err)
                }),
            )
            .await?;
        }
        Ok(())
    }

    fn remember(&mut self, sql: &str) {
        if let Some(setup) = navidog_db::sql::session_setup(sql) {
            self.setup = Some(setup);
        }
    }

    fn lost(&self, e: &DbError) -> bool {
        e.disconnected
    }

    async fn attempt(&mut self, sql: &str) -> DbResult<Vec<ResultSet>> {
        let tag = self.tag.clone();
        let lex = self.dialect.lex();
        let owned = sql.to_string();
        logged(
            &tag,
            sql,
            blocking(&self.client, &self.workers, move |c| {
                let mut sets = Vec::new();
                for piece in navidog_db::sql::split_statements(&owned, lex) {
                    sets.extend(result_sets(c.execute(&piece, &[]).map_err(err)?));
                }
                Ok(sets)
            }),
        )
        .await
    }
}

impl Session for DamengSession {
    fn server_version(&self) -> &str {
        &self.version
    }

    navidog_db::recovering_ping!(self, async {
        blocking(&self.client, &self.workers, |c| {
            c.execute("SELECT 1 FROM DUAL", &[])
                .map(|_| ())
                .map_err(err)
        })
        .await
    });

    fn cancel_handle(&self) -> Option<CancelHandle> {
        Some(self.cancellation.handle())
    }

    fn session_id(&self) -> Option<u64> {
        Some(self.session_id)
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        self.workers.send_replace(true);
        async move {
            // A canceled execute future can leave a queued/running spawn_blocking task. Wait
            // for its actual completion before the operation retirement fence is released.
            self.workers.closed().await;
            self.client
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .close();
            Ok(())
        }
        .boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

impl Relational for DamengSession {
    fn dialect(&self) -> std::sync::Arc<dyn navidog_sql::Dialect> {
        self.dialect.clone()
    }

    navidog_db::recovering_execute!();
    navidog_db::relational_catalog!(metadata);
}

/// The cancel handle kills the whole session with `SP_CLOSE_SESSION` from a short-lived
/// connection: dm-pure has no protocol-level cancel and DM has no statement-only
/// equivalent (140-dameng.md §3.4, deviation D2).
fn cancel_target(params: &ConnectParams, session_id: u64) -> CancelHandle {
    let params = params.clone();
    Arc::new(move || {
        let params = params.clone();
        async move {
            tokio::task::spawn_blocking(move || -> DbResult<()> {
                let mut client = DmClient::with_config(ClientConfig {
                    host: params.host.clone(),
                    port: params.port,
                    user: params.user.clone(),
                    password: params.password.clone().unwrap_or_default(),
                    timeout: params.timeout(),
                    auto_reconnect: false,
                    ..ClientConfig::default()
                });
                client.connect().map_err(err)?;
                client
                    .execute(&format!("CALL SP_CLOSE_SESSION({session_id})"), &[])
                    .map(|_| ())
                    .map_err(err)?;
                client.close();
                Ok(())
            })
            .await
            .map_err(|e| DbError::msg(e.to_string()))?
        }
        .boxed()
    })
}

/// Converts one dm-pure execution into navidog result sets. A `CALL` returns its OUT
/// values as one row, which is how `SP_TABLEDEF` reaches the DDL view.
fn result_sets(result: ExecuteResult) -> Vec<ResultSet> {
    match result {
        ExecuteResult::Query { columns, rows } => {
            vec![ResultSet {
                columns: columns
                    .iter()
                    .map(|c| Column {
                        name: c.name.clone(),
                        type_name: c.type_name.clone(),
                        origin: (!(c.schema_name.is_empty() || c.table_name.is_empty())).then(
                            || ColumnOrigin {
                                schema: c.schema_name.clone(),
                                table: c.table_name.clone(),
                                column: c.name.clone(),
                            },
                        ),
                    })
                    .collect(),
                rows: rows
                    .iter()
                    .map(|row| row.iter().map(value_text).collect())
                    .collect(),
                ..ResultSet::default()
            }]
        }
        ExecuteResult::Affected(n) => vec![ResultSet {
            affected_rows: Some(n),
            ..ResultSet::default()
        }],
        ExecuteResult::Call { affected, out } => {
            if out.is_empty() {
                vec![ResultSet {
                    affected_rows: Some(affected),
                    ..ResultSet::default()
                }]
            } else {
                vec![ResultSet {
                    columns: (0..out.len())
                        .map(|i| Column {
                            name: format!("OUT_{}", i + 1),
                            ..Column::default()
                        })
                        .collect(),
                    rows: vec![out.iter().map(value_text).collect()],
                    affected_rows: Some(affected),
                    ..ResultSet::default()
                }]
            }
        }
    }
}

/// Grid text of one cell (140-dameng.md §5.5). Binary values become `0x`-prefixed hex so
/// the text is unambiguous and matches the SQL hex literal the grid writes back.
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
        Value::TinyInt(v) => Some(v.to_string()),
        Value::SmallInt(v) => Some(v.to_string()),
        Value::Int(v) => Some(v.to_string()),
        Value::BigInt(v) => Some(v.to_string()),
        Value::Real(f) => Some(f.to_string()),
        Value::Double(f) => Some(f.to_string()),
        Value::Decimal(s) | Value::Str(s) | Value::Interval(s) => Some(s.clone()),
        Value::Bytes(b) | Value::Raw(b) => Some(format!("0x{}", hex::encode(b))),
        Value::Date(d) => Some(d.to_string()),
        Value::Time(t) => Some(time_text(&t.to_string())),
        Value::DateTime(dt) => {
            let (date, time) = (dt.date.to_string(), time_text(&dt.time.to_string()));
            Some(format!("{date} {time}"))
        }
    }
}

/// Trims the always-six-digit microseconds to `HH:MM:SS[.ffffff]` with trailing zeros gone.
fn time_text(formatted: &str) -> String {
    let (base, fraction) = formatted.split_once('.').unwrap_or((formatted, ""));
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        base.to_string()
    } else {
        format!("{base}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_errors_keep_their_negative_code() {
        let e = err(dm_pure::DmError::server(-2501, "用户名或密码错误"));
        assert_eq!(e.code.as_deref(), Some("-2501"));
        assert!(!e.disconnected);
        let e = err(dm_pure::DmError::conn("timeout"));
        assert_eq!(e.code.as_deref(), Some("-6852"));
        assert!(e.disconnected);
    }

    #[test]
    fn values_render_as_grid_text() {
        use dm_pure::{DmDate, DmDateTime, DmTime};
        assert_eq!(value_text(&Value::Bool(true)).unwrap(), "1");
        assert_eq!(value_text(&Value::Decimal("1.50".into())).unwrap(), "1.50");
        assert_eq!(
            value_text(&Value::Bytes(vec![0x89, 0x50])).unwrap(),
            "0x8950"
        );
        let date = DmDateTime::new(DmDate::new(2026, 11, 24), DmTime::new(8, 9, 10, 111222));
        assert_eq!(
            value_text(&Value::DateTime(date)).unwrap(),
            "2026-11-24 08:09:10.111222"
        );
        let plain = DmDateTime::new(DmDate::new(2026, 11, 24), DmTime::new(8, 9, 10, 0));
        assert_eq!(
            value_text(&Value::DateTime(plain)).unwrap(),
            "2026-11-24 08:09:10"
        );
        assert_eq!(value_text(&Value::Null), None);
    }

    #[test]
    fn time_text_trims_trailing_zeros() {
        assert_eq!(time_text("08:09:10.000000"), "08:09:10");
        assert_eq!(time_text("08:09:10.100000"), "08:09:10.1");
        assert_eq!(time_text("08:09:10"), "08:09:10");
    }

    #[test]
    fn call_results_surface_out_values_as_rows() {
        let sets = result_sets(ExecuteResult::Call {
            affected: 0,
            out: vec![Value::Str("CREATE TABLE \"SYSDBA\".\"T\"".into())],
        });
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].columns[0].name, "OUT_1");
        assert_eq!(
            sets[0].rows[0][0].as_deref(),
            Some("CREATE TABLE \"SYSDBA\".\"T\"")
        );
        let sets = result_sets(ExecuteResult::Call {
            affected: 7,
            out: vec![],
        });
        assert_eq!(sets[0].affected_rows, Some(7));
        assert!(sets[0].rows.is_empty());
    }

    #[test]
    fn result_columns_keep_their_origin() {
        use dm_pure::Column as DmColumn;
        let sets = result_sets(ExecuteResult::Query {
            columns: vec![DmColumn {
                col_type: 2,
                prec: 100,
                scale: 0,
                nullable: true,
                name: "NAME".into(),
                type_name: "VARCHAR".into(),
                table_name: "T".into(),
                schema_name: "SYSDBA".into(),
                lob: false,
            }],
            rows: vec![vec![Value::Str("x".into())]],
        });
        let origin = sets[0].columns[0].origin.as_ref().unwrap();
        assert_eq!(origin.schema, "SYSDBA");
        assert_eq!(origin.table, "T");
        assert_eq!(origin.column, "NAME");
    }
}
