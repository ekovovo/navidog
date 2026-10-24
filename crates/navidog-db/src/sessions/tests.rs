use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::Semaphore;

use futures::FutureExt as _;

use super::*;
use crate::{CancelHandle, DriverInfo, Family, ResultSet};

async fn sessions_with_metadata(
    driver: Arc<dyn Driver>,
    params: ConnectParams,
) -> (Sessions, SharedSession) {
    let mut session = crate::connect(driver.clone(), params.clone())
        .await
        .unwrap();
    let dialect = session.relational().map(|rel| rel.dialect());
    let primary = Arc::new(Mutex::new(Some(session)));
    let primary_db = driver
        .info()
        .per_database_sessions
        .then(|| params.database.clone().unwrap_or_else(|| "postgres".into()));
    let mut sessions = Sessions::default();
    sessions.live.insert(
        "test".into(),
        Live {
            driver,
            params,
            dialect,
            primary: primary.clone(),
            primary_db,
            per_db: Default::default(),
            keepalive: None,
            operations: OperationScope::default(),
        },
    );
    (sessions, primary)
}

#[derive(Default)]
struct Calls {
    opened: AtomicUsize,
    submitted: AtomicUsize,
    initialized: AtomicUsize,
    closed: AtomicUsize,
    fail_connect: AtomicBool,
    cancelled: AtomicUsize,
    connect_gate: StdMutex<Option<Arc<Gate>>>,
    execute_gate: StdMutex<Option<Arc<Gate>>>,
    close_gate: StdMutex<Option<Arc<Gate>>>,
}

struct Gate {
    entered: Semaphore,
    release: Semaphore,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        })
    }

    async fn block(&self) {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }

    async fn entered(&self) {
        self.entered.acquire().await.unwrap().forget();
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

struct CountingDriver {
    info: DriverInfo,
    calls: Arc<Calls>,
}

impl CountingDriver {
    fn new(calls: Arc<Calls>) -> Self {
        Self {
            info: DriverInfo {
                id: "counting".into(),
                name: "Counting".into(),
                family: Family::Relational,
                default_port: None,
                supports_ssl: false,
                supports_ssh: false,
                per_database_sessions: false,
            },
            calls,
        }
    }
}

impl Driver for CountingDriver {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>> {
        let calls = self.calls.clone();
        let gate = calls.connect_gate.lock().unwrap().take();
        async move {
            calls.opened.fetch_add(1, Ordering::SeqCst);
            if !params.initial_queries.trim().is_empty() {
                calls.initialized.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = gate {
                gate.block().await;
            }
            if calls.fail_connect.load(Ordering::SeqCst) {
                return Err(DbError::coded("CONNECT", "connection refused"));
            }
            Ok(Box::new(CountingSession(calls)) as Box<dyn Session>)
        }
        .boxed()
    }
}

struct CountingSession(Arc<Calls>);

impl Session for CountingSession {
    fn server_version(&self) -> &str {
        "test"
    }

    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>> {
        async { Ok(()) }.boxed()
    }

    fn cancel_handle(&self) -> Option<CancelHandle> {
        let calls = self.0.clone();
        Some(Arc::new(move || {
            calls.cancelled.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }.boxed()
        }))
    }

    fn session_id(&self) -> Option<u64> {
        None
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>> {
        let gate = self.0.close_gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                gate.block().await;
            }
            self.0.closed.fetch_add(1, Ordering::SeqCst);
            Err(DbError::coded("CLOSE", "close response lost"))
        }
        .boxed()
    }

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        Some(self)
    }
}

impl Relational for CountingSession {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn execute<'a>(&'a mut self, _: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>> {
        self.0.submitted.fetch_add(1, Ordering::SeqCst);
        let gate = self.0.execute_gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                gate.block().await;
            }
            Ok(Vec::new())
        }
        .boxed()
    }
}

#[test]
fn operation_cleanup_preserves_success_and_failure_without_replaying() {
    runtime::runtime().block_on(async {
        let calls = Arc::new(Calls::default());
        let driver = Arc::new(CountingDriver::new(calls.clone()));
        let (sessions, metadata) = sessions_with_metadata(driver, ConnectParams::default()).await;
        for expected in [
            Ok(7),
            Err(DbError::coded("WRITE", "original write error")),
            Err(crate::execution::interrupted(
                DbError::coded("08006", "write response lost"),
                true,
            )),
        ] {
            let outcome = expected.clone();
            let result = sessions
                .run_operation_rel("test", None, move |rel| {
                    async move {
                        rel.execute("INSERT INTO fixture VALUES (1)").await?;
                        outcome
                    }
                    .boxed()
                })
                .unwrap()
                .await
                .unwrap();
            assert_eq!(
                result, expected,
                "close failure must preserve the operation result"
            );
        }
        assert_eq!(calls.opened.load(Ordering::SeqCst), 4);
        assert_eq!(calls.submitted.load(Ordering::SeqCst), 3);
        assert_eq!(calls.closed.load(Ordering::SeqCst), 3);
        assert_eq!(sessions.session_count("test"), 1);

        calls.fail_connect.store(true, Ordering::SeqCst);
        let error = sessions
            .run_operation_rel::<()>("test", None, |_| {
                async { panic!("failed connection must not invoke the operation") }.boxed()
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code.as_deref(), Some("CONNECT"));
        assert_eq!(error.message, "connection refused");
        assert!(!error.is_outcome_unknown());
        assert_eq!(calls.submitted.load(Ordering::SeqCst), 3);
        assert_eq!(calls.closed.load(Ordering::SeqCst), 3);
        let _ = metadata.lock().await.take().unwrap().close().await;
    });
}

async fn operation_result<T>(job: JoinHandle<DbResult<T>>) -> DbResult<T> {
    tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .expect("operation must finish without releasing its blocked driver future")
        .unwrap()
}

#[test]
fn closing_during_connect_revokes_the_operation_before_submission() {
    runtime::runtime().block_on(async {
        let calls = Arc::new(Calls::default());
        let driver = Arc::new(CountingDriver::new(calls.clone()));
        let (mut sessions, _) = sessions_with_metadata(driver, ConnectParams::default()).await;
        let scope = sessions.live["test"].operations.clone();
        assert!(scope.is_drained());
        assert_eq!(sessions.generation("test"), 0);
        let gate = Gate::new();
        *calls.connect_gate.lock().unwrap() = Some(gate.clone());
        let job = sessions
            .run_operation_rel::<()>("test", None, |_| {
                async { panic!("a revoked connection must not start the operation") }.boxed()
            })
            .unwrap();
        gate.entered().await;
        assert!(!scope.is_drained());
        sessions.shut("test");
        assert_eq!(sessions.generation("test"), 1);
        assert!(
            !job.is_finished(),
            "retirement waits for an already-polled connect"
        );
        gate.release();
        let error = operation_result(job).await.unwrap_err();
        assert!(error.is_session_lost());
        assert!(!error.is_outcome_unknown());
        assert_eq!(calls.submitted.load(Ordering::SeqCst), 0);
        assert_eq!(calls.cancelled.load(Ordering::SeqCst), 0);
        assert!(
            scope.is_drained(),
            "completed operations deregister themselves"
        );
    });
}

#[test]
fn retired_connect_waits_for_initialization_cleanup_before_reopen() {
    runtime::runtime().block_on(async {
        let calls = Arc::new(Calls::default());
        let driver = Arc::new(CountingDriver::new(calls.clone()));
        let (mut sessions, _) =
            sessions_with_metadata(driver.clone(), ConnectParams::default()).await;
        sessions
            .live
            .get_mut("test")
            .unwrap()
            .params
            .initial_queries = "INSERT INTO initialization_audit VALUES (1)".into();
        let scope = sessions.live["test"].operations.clone();
        let gate = Gate::new();
        *calls.connect_gate.lock().unwrap() = Some(gate.clone());
        let job = sessions
            .run_operation_rel::<()>("test", None, |_| {
                async { panic!("retired initialization must not enter the business closure") }
                    .boxed()
            })
            .unwrap();
        gate.entered().await;
        assert_eq!(calls.initialized.load(Ordering::SeqCst), 1);
        sessions.shut("test");
        let reopening = operation::connect_after_drain(
            sessions.retiring["test"].clone(),
            driver,
            ConnectParams::default(),
        );
        tokio::pin!(reopening);
        assert!(futures::poll!(reopening.as_mut()).is_pending());
        assert!(!scope.is_drained());
        assert!(!job.is_finished());
        assert_eq!(calls.opened.load(Ordering::SeqCst), 2);
        gate.release();
        let error = operation_result(job).await.unwrap_err();
        assert!(error.is_session_lost());
        assert!(error.is_outcome_unknown());
        assert_eq!(calls.initialized.load(Ordering::SeqCst), 1);
        assert_eq!(calls.submitted.load(Ordering::SeqCst), 0);
        assert!(scope.is_drained());
        reopening.await.unwrap().close().await.unwrap_err();
        assert_eq!(calls.opened.load(Ordering::SeqCst), 3);
    });
}

#[test]
fn closing_during_execution_cancels_and_never_continues_the_batch() {
    runtime::runtime().block_on(async {
        let calls = Arc::new(Calls::default());
        let driver = Arc::new(CountingDriver::new(calls.clone()));
        let (mut sessions, _) = sessions_with_metadata(driver, ConnectParams::default()).await;
        let scope = sessions.live["test"].operations.clone();
        let gate = Gate::new();
        *calls.execute_gate.lock().unwrap() = Some(gate.clone());
        let job = sessions
            .run_operation_rel("test", None, |rel| {
                async move {
                    rel.execute("FIRST WRITE").await?;
                    rel.execute("SECOND WRITE").await?;
                    Ok(())
                }
                .boxed()
            })
            .unwrap();
        gate.entered().await;
        sessions.shut("test");
        let error = operation_result(job).await.unwrap_err();
        assert!(error.is_session_lost());
        assert!(error.is_outcome_unknown());
        assert_eq!(calls.submitted.load(Ordering::SeqCst), 1);
        assert_eq!(calls.cancelled.load(Ordering::SeqCst), 1);
        assert!(scope.is_drained());
        gate.release();
    });
}

#[test]
fn reopening_waits_for_retired_cleanup_and_preserves_its_known_result() {
    runtime::runtime().block_on(async {
        for expected in [Ok(7), Err(DbError::coded("WRITE", "known write failure"))] {
            let calls = Arc::new(Calls::default());
            let driver = Arc::new(CountingDriver::new(calls.clone()));
            let params = ConnectParams::default();
            let (mut sessions, _) = sessions_with_metadata(driver.clone(), params.clone()).await;
            let scope = sessions.live["test"].operations.clone();
            let gate = Gate::new();
            *calls.close_gate.lock().unwrap() = Some(gate.clone());
            let outcome = expected.clone();
            let job = sessions
                .run_operation_rel("test", None, move |rel| {
                    async move {
                        rel.execute("WRITE").await?;
                        outcome
                    }
                    .boxed()
                })
                .unwrap();
            gate.entered().await;
            sessions.shut("test");
            let retired = sessions.retiring["test"].clone();
            let first = operation::connect_after_drain(retired, driver.clone(), params.clone());
            let second = operation::connect_after_drain(
                sessions.retiring["test"].clone(),
                driver.clone(),
                params.clone(),
            );
            tokio::pin!(first, second);
            assert!(futures::poll!(first.as_mut()).is_pending());
            assert!(futures::poll!(second.as_mut()).is_pending());
            assert_eq!(calls.opened.load(Ordering::SeqCst), 2);
            assert!(!scope.is_drained());
            gate.release();
            assert_eq!(operation_result(job).await, expected);
            assert!(scope.is_drained());
            assert_eq!(calls.cancelled.load(Ordering::SeqCst), 0);
            let primary = Arc::new(Mutex::new(Some(first.await.unwrap())));
            second.await.unwrap().close().await.unwrap_err();
            sessions.advance_generation("test");
            assert_eq!(sessions.generation("test"), 2);
            sessions.live.insert(
                "test".into(),
                Live {
                    driver,
                    params,
                    dialect: Some(Dialect::Sqlite),
                    primary,
                    primary_db: None,
                    per_db: Default::default(),
                    keepalive: None,
                    operations: OperationScope::default(),
                },
            );
            sessions
                .run_operation_rel("test", None, |rel| {
                    async move { rel.execute("NEW GENERATION WRITE").await }.boxed()
                })
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(calls.submitted.load(Ordering::SeqCst), 2);
            sessions.shut("test");
            assert_eq!(sessions.generation("test"), 3);
            assert!(sessions.retiring["test"].is_empty());
        }
    });
}

fn network_drivers() -> Vec<(Arc<dyn Driver>, ConnectParams)> {
    [
        (
            Arc::new(crate::mysql::MySqlDriver::mysql()) as Arc<dyn Driver>,
            13306,
            "root",
            "sakila",
        ),
        (
            Arc::new(crate::mysql::MySqlDriver::mariadb()),
            13307,
            "root",
            "sakila",
        ),
        (
            Arc::new(crate::postgres::PostgresDriver::default()),
            15432,
            "postgres",
            "pagila",
        ),
    ]
    .into_iter()
    .map(|(driver, port, user, database)| {
        (
            driver,
            ConnectParams {
                host: "127.0.0.1".into(),
                port,
                user: user.into(),
                password: Some("navidog".into()),
                database: Some(database.into()),
                ..Default::default()
            },
        )
    })
    .collect()
}

#[test]
fn completed_operation_does_not_pin_or_replay_on_metadata_recovery() {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, params) in network_drivers() {
            let mut observer = crate::connect(driver.clone(), params.clone())
                .await
                .unwrap();
            let dialect = observer.relational().unwrap().dialect();
            let table = dialect.quote_ident(&format!("navidog_operation_{}", std::process::id()));
            observer
                .relational()
                .unwrap()
                .execute(&format!(
                    "DROP TABLE IF EXISTS {table}; CREATE TABLE {table} (n INTEGER)"
                ))
                .await
                .unwrap();
            let (sessions, metadata) = sessions_with_metadata(driver, params).await;
            let old_id = metadata
                .lock()
                .await
                .as_ref()
                .unwrap()
                .session_id()
                .unwrap();
            let insert = format!("INSERT INTO {table} VALUES (1)");
            sessions
                .run_operation_rel("test", None, move |rel| {
                    async move { rel.execute(&insert).await }.boxed()
                })
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(sessions.session_count("test"), 1);
            assert_eq!(
                metadata.lock().await.as_ref().unwrap().session_id(),
                Some(old_id)
            );

            let kill = match dialect {
                Dialect::MySql => format!("KILL {old_id}"),
                Dialect::Postgres => format!("SELECT pg_terminate_backend({old_id})"),
                _ => unreachable!(),
            };
            observer.relational().unwrap().execute(&kill).await.unwrap();
            let select = format!("SELECT COUNT(*) FROM {table}");
            let result = sessions
                .run_rel("test", None, move |rel| {
                    async move { rel.execute_read(&select).await }.boxed()
                })
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result[0].rows[0][0].as_deref(), Some("1"));
            assert_ne!(
                metadata.lock().await.as_ref().unwrap().session_id(),
                Some(old_id)
            );
            assert_eq!(sessions.session_count("test"), 1);
            metadata.lock().await.take().unwrap().close().await.unwrap();
            observer
                .relational()
                .unwrap()
                .execute(&format!("DROP TABLE {table}"))
                .await
                .unwrap();
            observer.close().await.unwrap();
        }
    });
}

#[test]
fn retired_network_operation_keeps_committed_write_and_stops_remaining_batch() {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, params) in network_drivers() {
            let mut observer = crate::connect(driver.clone(), params.clone())
                .await
                .unwrap();
            let dialect = observer.relational().unwrap().dialect();
            let table =
                dialect.quote_ident(&format!("navidog_operation_cancel_{}", std::process::id()));
            observer
                .relational()
                .unwrap()
                .execute(&format!(
                    "DROP TABLE IF EXISTS {table}; CREATE TABLE {table} (n INTEGER)"
                ))
                .await
                .unwrap();
            let (mut sessions, _) = sessions_with_metadata(driver.clone(), params.clone()).await;
            let gate = Gate::new();
            let blocked = gate.clone();
            let first = format!("INSERT INTO {table} VALUES (1)");
            let second = format!("INSERT INTO {table} VALUES (2)");
            let job = sessions
                .run_operation_rel("test", None, move |rel| {
                    async move {
                        rel.execute(&first).await?;
                        // The first autocommit write is known to the server, but the whole
                        // business closure has not completed when its generation is retired.
                        blocked.block().await;
                        rel.execute(&second).await?;
                        Ok(())
                    }
                    .boxed()
                })
                .unwrap();
            gate.entered().await;
            sessions.shut("test");
            let error = operation_result(job).await.unwrap_err();
            assert!(error.is_session_lost());
            assert!(error.is_outcome_unknown());
            let mut reopened =
                operation::connect_after_drain(sessions.retiring["test"].clone(), driver, params)
                    .await
                    .unwrap();
            let result = reopened
                .relational()
                .unwrap()
                .execute_read(&format!("SELECT COUNT(*) FROM {table}"))
                .await
                .unwrap();
            assert_eq!(
                result[0].rows[0][0].as_deref(),
                Some("1"),
                "retirement must not replay the first write or submit the remaining write"
            );
            gate.release();
            reopened.close().await.unwrap();
            observer
                .relational()
                .unwrap()
                .execute(&format!("DROP TABLE {table}"))
                .await
                .unwrap();
            observer.close().await.unwrap();
        }
    });
}
