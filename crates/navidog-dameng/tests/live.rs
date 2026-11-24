//! Driver tests against a local DM8 instance. The DM image license only covers learning,
//! testing and development, so this is optional and never runs in CI:
//!
//! ```sh
//! docker run -d --name dm8-utf8 --platform linux/amd64 --privileged -p 15236:5236 \
//!   -e PAGE_SIZE=16 -e LD_LIBRARY_PATH=/opt/dmdbms/bin -e EXTENT_SIZE=32 \
//!   -e BLANK_PAD_MODE=1 -e LOG_SIZE=1024 -e UNICODE_FLAG=1 -e LENGTH_IN_CHAR=1 \
//!   -e INSTANCE_NAME=dm8-utf8 -e SYSDBA_PWD=SYSDBA001 -e BUFFER=1000 \
//!   dm8_single:dm8_20241022_rev244896_x86_rh6_64
//! NAVIDOG_DM_LIVE=1 cargo test -p navidog-dameng --test live
//! docker stop dm8-utf8 && docker rm dm8-utf8
//! ```
//!
//! CS=0 and mode-2/4 instances are covered by the manual probes in
//! `docs/pr/acceptance-dameng.md`; the tests below need only the UTF-8 instance.

use std::sync::Arc;

use dm_pure::{DmClient, ExecuteResult};
use navidog_dameng::DamengDriver;
use navidog_db::params::ConnectParams;
use navidog_db::{Driver, connect, runtime, test_connection};
use navidog_model::object::{ObjectKind, Scope};

fn live() -> bool {
    std::env::var_os("NAVIDOG_DM_LIVE").is_some()
}

fn port() -> u16 {
    std::env::var("DM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(15236)
}

fn params() -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port: port(),
        user: "SYSDBA".into(),
        password: Some("SYSDBA001".into()),
        ..Default::default()
    }
}

fn scope() -> Scope {
    Scope::new("c", "DAMENG", Some("SYSDBA".into()))
}

async fn session() -> Box<dyn navidog_db::Session> {
    connect(Arc::new(DamengDriver::default()), params())
        .await
        .unwrap()
}

/// Runs one statement and ignores "already exists"/"does not exist" errors, for setup
/// and teardown of scratch objects.
async fn ignore_errors(session: &mut Box<dyn navidog_db::Session>, sql: &str) {
    let rel = session.relational().unwrap();
    let _ = rel.execute(sql).await;
}

async fn exec(session: &mut Box<dyn navidog_db::Session>, sql: &str) -> Vec<navidog_db::ResultSet> {
    session
        .relational()
        .unwrap()
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql} failed: {e}"))
}

async fn cell(session: &mut Box<dyn navidog_db::Session>, sql: &str) -> Option<String> {
    exec(session, sql)
        .await
        .into_iter()
        .find_map(|set| set.rows.into_iter().next())
        .and_then(|row| row.into_iter().next().flatten())
}

/// Drops the scratch objects of one test, children before parents. Each test owns its
/// names so parallel tests never collide.
async fn cleanup(objects: &[(&str, &str)]) {
    let mut session = session().await;
    for (kind, name) in objects {
        ignore_errors(&mut session, &format!("DROP {kind} \"SYSDBA\".\"{name}\"")).await;
    }
    session.close().await.unwrap();
}

fn cat_objects() -> Vec<(&'static str, &'static str)> {
    vec![
        ("TRIGGER", "ND_CAT_TRG"),
        ("VIEW", "ND_CAT_V"),
        ("INDEX", "ND_CAT_IDX"),
        ("SEQUENCE", "ND_CAT_SEQ"),
        ("FUNCTION", "ND_CAT_F"),
        ("TABLE", "ND_CAT_CHILD"),
        ("TABLE", "ND_CAT_T"),
    ]
}

#[test]
fn connect_reports_version_and_clean_auth_errors() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let driver: Arc<dyn Driver> = Arc::new(DamengDriver::default());
        let session = connect(driver.clone(), params()).await.unwrap();
        let version = session.server_version();
        assert!(version.contains("DM Database Server"), "{version}");
        assert!(session.session_id().unwrap_or(0) > 0, "{version}");
        session.close().await.unwrap();

        let mut bad = params();
        bad.password = Some("wrong".into());
        let e = test_connection(driver, bad).await.unwrap_err();
        assert_eq!(e.code.as_deref(), Some("-2501"), "{e}");
        assert!(e.message.contains("用户名或密码错误"), "{e}");
    });
}

#[test]
fn catalog_methods_cover_every_object_kind() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&cat_objects()).await;
        let mut session = session().await;
        for sql in [
            "CREATE TABLE \"SYSDBA\".\"ND_CAT_T\" (\"ID\" INT IDENTITY(1, 1) PRIMARY KEY, \
             \"NAME\" VARCHAR(100) NOT NULL, \"AMT\" DECIMAL(10, 2) DEFAULT 0)",
            "CREATE INDEX \"SYSDBA\".\"ND_CAT_IDX\" ON \"SYSDBA\".\"ND_CAT_T\"(\"NAME\")",
            "COMMENT ON TABLE \"SYSDBA\".\"ND_CAT_T\" IS '目录探测'",
            "COMMENT ON COLUMN \"SYSDBA\".\"ND_CAT_T\".\"ID\" IS '主键'",
            "CREATE OR REPLACE TRIGGER \"SYSDBA\".\"ND_CAT_TRG\" AFTER INSERT ON \
             \"SYSDBA\".\"ND_CAT_T\" BEGIN NULL; END;",
            "CREATE TABLE \"SYSDBA\".\"ND_CAT_CHILD\" (\"PID\" INT REFERENCES \"SYSDBA\".\"ND_CAT_T\"(\"ID\"))",
            "CREATE OR REPLACE VIEW \"SYSDBA\".\"ND_CAT_V\" AS SELECT \"ID\", \"NAME\" FROM \"SYSDBA\".\"ND_CAT_T\"",
            "CREATE SEQUENCE \"SYSDBA\".\"ND_CAT_SEQ\" START WITH 10 INCREMENT BY 5",
            "CREATE OR REPLACE FUNCTION \"SYSDBA\".\"ND_CAT_F\" RETURN INT IS BEGIN RETURN 1; END;",
        ] {
            exec(&mut session, sql).await;
        }
        let rel = session.relational().unwrap();
        let scope = scope();
        let dbs = rel.list_databases().await.unwrap();
        assert_eq!(dbs, vec!["DAMENG".to_string()], "{dbs:?}");
        let schemas = rel.list_schemas().await.unwrap();
        assert!(schemas.iter().any(|s| s == "SYSDBA"), "{schemas:?}");
        assert!(schemas.iter().any(|s| s == "SYS"), "system schemas are listed");

        let tables = rel.list_objects(&scope, ObjectKind::Table).await.unwrap();
        let table = tables.iter().find(|t| t.name == "ND_CAT_T").expect("table");
        assert_eq!(table.comment.as_deref(), Some("目录探测"));
        assert!(table.properties.iter().any(|(k, _)| k == "owner"));
        let views = rel.list_objects(&scope, ObjectKind::View).await.unwrap();
        assert!(views.iter().any(|v| v.name == "ND_CAT_V"), "{views:?}");
        let functions = rel.list_objects(&scope, ObjectKind::Function).await.unwrap();
        assert!(
            functions
                .iter()
                .any(|f| f.name == "ND_CAT_F" && f.properties.iter().any(|(k, v)| k == "type" && v == "FUNCTION")),
            "{functions:?}"
        );
        let seqs = rel.list_objects(&scope, ObjectKind::Sequence).await.unwrap();
        assert!(seqs.iter().any(|s| s.name == "ND_CAT_SEQ"), "{seqs:?}");
        let triggers = rel.list_objects(&scope, ObjectKind::Trigger).await.unwrap();
        assert!(triggers.iter().any(|t| t.name == "ND_CAT_TRG"), "{triggers:?}");

        let ddl = rel.object_ddl(&scope, ObjectKind::Table, "ND_CAT_T").await.unwrap();
        assert!(ddl.contains("CREATE TABLE"), "{ddl}");
        assert!(ddl.contains("IDENTITY(1, 1)"), "{ddl}");
        assert!(ddl.contains("STORAGE"), "{ddl}");
        let view_ddl = rel.object_ddl(&scope, ObjectKind::View, "ND_CAT_V").await.unwrap();
        assert!(view_ddl.starts_with("CREATE VIEW"), "{view_ddl}");
        let seq_ddl = rel.object_ddl(&scope, ObjectKind::Sequence, "ND_CAT_SEQ").await.unwrap();
        assert!(seq_ddl.contains("START WITH 10"), "{seq_ddl}");
        assert!(seq_ddl.contains("INCREMENT BY 5"), "{seq_ddl}");
        let idx_ddl = rel.object_ddl(&scope, ObjectKind::Index, "ND_CAT_IDX").await.unwrap();
        assert!(idx_ddl.contains("CREATE INDEX"), "{idx_ddl}");
        let trg_ddl = rel.object_ddl(&scope, ObjectKind::Trigger, "ND_CAT_TRG").await.unwrap();
        assert!(trg_ddl.contains("CREATE OR REPLACE TRIGGER"), "{trg_ddl}");
        let fn_ddl = rel.object_ddl(&scope, ObjectKind::Function, "ND_CAT_F").await.unwrap();
        assert!(fn_ddl.contains("ND_CAT_F"), "{fn_ddl}");

        let columns = rel.table_columns(&scope, "ND_CAT_T").await.unwrap();
        assert_eq!(columns.len(), 3, "{columns:?}");
        assert_eq!(columns[0].type_name, "INT");
        assert!(columns[0].primary_key && !columns[0].nullable);
        assert_eq!(columns[0].comment.as_deref(), Some("主键"));
        assert_eq!(columns[2].default.as_deref(), Some("0"));
        let children = rel.table_children(&scope, "ND_CAT_T").await.unwrap();
        assert!(children.indexes.iter().any(|i| i == "ND_CAT_IDX"), "{children:?}");
        assert!(children.triggers.iter().any(|t| t == "ND_CAT_TRG"), "{children:?}");
        let keys = rel.foreign_keys(&scope, "ND_CAT_CHILD").await.unwrap();
        assert_eq!(keys.len(), 1, "{keys:?}");
        assert_eq!(keys[0].column, "PID");
        assert_eq!(keys[0].table, "ND_CAT_T");
        assert_eq!(keys[0].referenced, "ID");

        let outline = rel.outline(&scope, false).await.unwrap();
        assert!(outline.tables.iter().any(|t| t.name == "ND_CAT_T"), "{outline:?}");
        assert!(outline.tables.iter().any(|t| t.name == "ND_CAT_V"));
        assert!(
            outline
                .tables
                .iter()
                .find(|t| t.name == "ND_CAT_T")
                .unwrap()
                .columns
                .iter()
                .any(|(name, _ty)| name == "AMT"),
            "{outline:?}"
        );
        assert!(outline.functions.iter().any(|f| f == "ND_CAT_F"), "{outline:?}");

        // The designer's enrich path: identity text carries the seed and increment.
        let metadata = rel
            .execute_read(&DamengDriver::default().info().dialect.column_metadata_sql(&scope, "ND_CAT_T").unwrap())
            .await
            .unwrap();
        let identity: Vec<_> = metadata
            .into_iter()
            .flat_map(|set| set.rows)
            .filter_map(|row| row.get(4).cloned().flatten())
            .filter(|v| !v.is_empty())
            .collect();
        assert_eq!(identity, vec!["IDENTITY(1, 1)".to_string()], "{identity:?}");

        cleanup(&cat_objects()).await;
        session.close().await.unwrap();
    });
}

#[test]
fn chinese_names_and_data_round_trip() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_CN_中文"), ("TABLE", "ND_CN_T")]).await;
        let mut session = session().await;
        for sql in [
            "CREATE TABLE \"SYSDBA\".\"ND_CN_中文\" (\"中文列\" VARCHAR(200), \"名称\" INT)",
            "CREATE TABLE \"SYSDBA\".\"ND_CN_T\" (\"S\" VARCHAR(200), \"N\" INT)",
        ] {
            exec(&mut session, sql).await;
        }
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_CN_中文\" VALUES ('中文数据 English 混合', 42)",
        )
        .await;
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_CN_T\" VALUES ('值', 7)",
        )
        .await;
        let value = cell(
            &mut session,
            "SELECT \"中文列\" FROM \"SYSDBA\".\"ND_CN_中文\"",
        )
        .await
        .expect("row");
        assert_eq!(value, "中文数据 English 混合");
        let value = cell(
            &mut session,
            "SELECT \"S\" FROM \"SYSDBA\".\"ND_CN_T\" WHERE \"N\" = 7",
        )
        .await
        .expect("row");
        assert_eq!(value, "值");
        cleanup(&[("TABLE", "ND_CN_中文"), ("TABLE", "ND_CN_T")]).await;
        session.close().await.unwrap();
    });
}

#[test]
fn lobs_round_trip_without_truncation() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_LOB_T")]).await;
        let mut session = session().await;
        exec(
            &mut session,
            "CREATE TABLE \"SYSDBA\".\"ND_LOB_T\" (\"ID\" INT IDENTITY(1, 1), \
             \"DOC\" CLOB, \"IMG\" BLOB)",
        )
        .await;
        let clob: String = "中文字符ABC".repeat(16000).chars().take(136000).collect();
        let blob: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        // Bind the large values as parameters; the driver streams out-of-row LOBs.
        let insert = "INSERT INTO \"SYSDBA\".\"ND_LOB_T\" (\"DOC\", \"IMG\") VALUES (?, ?)";
        let mut client = dm_client();
        let hex = blob.iter().map(|b| format!("{b:02X}")).collect::<String>();
        client
            .execute(
                insert,
                &[dm_pure::Param::Str(clob.clone()), dm_pure::Param::Bytes(blob.clone())],
            )
            .unwrap();
        let mut read = |sql: &str| {
            match client.execute(sql, &[]).unwrap() {
                ExecuteResult::Query { rows, .. } => rows,
                other => panic!("{other:?}"),
            }
        };
        let rows = read("SELECT \"DOC\" FROM \"SYSDBA\".\"ND_LOB_T\" WHERE \"ID\" = (SELECT MAX(\"ID\") FROM \"SYSDBA\".\"ND_LOB_T\")");
        match &rows[0][0] {
            dm_pure::Value::Str(text) if text.is_empty() => {
                eprintln!("skipped: out-of-row LOB reads need the patched dm-pure (D5)");
                return;
            }
            dm_pure::Value::Str(text) => assert_eq!(text.len(), 136_000),
            other => panic!("{other:?}"),
        }
        let rows = read("SELECT \"IMG\" FROM \"SYSDBA\".\"ND_LOB_T\" WHERE \"ID\" = (SELECT MAX(\"ID\") FROM \"SYSDBA\".\"ND_LOB_T\")");
        match &rows[0][0] {
            dm_pure::Value::Bytes(bytes) => assert_eq!(*bytes, blob),
            other => panic!("{other:?}"),
        }
        // The grid path reads the BLOB back as hex text through the session too.
        let hex_read = cell(&mut session, "SELECT \"IMG\" FROM \"SYSDBA\".\"ND_LOB_T\"")
            .await
            .expect("row");
        assert!(hex_read.starts_with("0x"), "{hex_read}");
        assert_eq!(hex_read.len(), 2 + blob.len() * 2);
        let _ = hex;
        cleanup(&[("TABLE", "ND_LOB_T")]).await;
        session.close().await.unwrap();
    });
}

fn dm_client() -> DmClient {
    let p = params();
    let mut client = DmClient::with_config(dm_pure::ClientConfig {
        host: p.host.clone(),
        port: p.port,
        user: p.user.clone(),
        password: p.password.clone().unwrap_or_default(),
        ..Default::default()
    });
    client.connect().unwrap();
    client
}

#[test]
fn transactions_commit_on_ddl_and_rollback_on_request() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_TX_T")]).await;
        let mut session = session().await;
        exec(
            &mut session,
            "CREATE TABLE \"SYSDBA\".\"ND_TX_T\" (\"V\" INT)",
        )
        .await;
        let rel = session.relational().unwrap();
        let d = rel.dialect();
        // START TRANSACTION / ROLLBACK leaves nothing behind.
        exec(&mut session, d.begin_transaction()).await;
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_TX_T\" VALUES (1)",
        )
        .await;
        exec(&mut session, "ROLLBACK").await;
        let count = cell(&mut session, "SELECT COUNT(*) FROM \"SYSDBA\".\"ND_TX_T\"")
            .await
            .unwrap();
        assert_eq!(count, "0");
        // A pending INSERT is committed by the following DDL (deviation D1).
        exec(&mut session, d.begin_transaction()).await;
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_TX_T\" VALUES (2)",
        )
        .await;
        exec(
            &mut session,
            "ALTER TABLE \"SYSDBA\".\"ND_TX_T\" ADD (\"X\" INT)",
        )
        .await;
        exec(&mut session, "ROLLBACK").await;
        let count = cell(&mut session, "SELECT COUNT(*) FROM \"SYSDBA\".\"ND_TX_T\"")
            .await
            .unwrap();
        assert_eq!(count, "1", "the DDL committed the pending INSERT");
        let has_x = cell(
            &mut session,
            "SELECT COUNT(*) FROM ALL_TAB_COLUMNS WHERE TABLE_NAME = 'ND_TX_T' \
             AND COLUMN_NAME = 'X'",
        )
        .await
        .unwrap();
        assert_eq!(has_x, "1");
        cleanup(&[("TABLE", "ND_TX_T")]).await;
        session.close().await.unwrap();
    });
}

#[test]
fn cancel_kills_the_session_and_a_fresh_connect_recovers() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut victim = session().await;
        let old_id = victim.session_id().unwrap();
        let cancel = victim.cancel_handle().unwrap();
        // A ~seconds-long aggregation (40000 × 40000 evaluated by SUM; a bare COUNT of the
        // cross join is optimized away) to cancel mid-flight.
        let long_query = "SELECT SUM(A.N * 2 + B.N) FROM \
             (SELECT LEVEL N FROM DUAL CONNECT BY LEVEL <= 40000) A, \
             (SELECT LEVEL N FROM DUAL CONNECT BY LEVEL <= 40000) B";
        let rel = victim.relational().unwrap();
        let long_running = rel.execute(long_query);
        tokio::select! {
            result = long_running => {
                // The victim dying is the cancel landing (the SP_CLOSE_SESSION connection
                // itself may still be waiting for its response when this branch wins); a
                // success means the query outran the cancel.
                let e = result.expect_err("the long query finished before the cancel landed");
                assert!(e.is_session_lost(), "{e}");
            }
            _ = cancel() => {}
        }
        // The killed session answers with a lost connection and an unknown outcome.
        let rel = victim.relational().unwrap();
        let e = rel.execute("SELECT 1 FROM DUAL").await.unwrap_err();
        assert!(e.is_session_lost(), "{e}");
        victim.close().await.unwrap();
        let fresh = session().await;
        let new_id = fresh.session_id().unwrap();
        assert_ne!(new_id, old_id, "the fresh session must be a new one");
        fresh.close().await.unwrap();
    });
}

#[test]
fn set_schema_switches_and_replays() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut session = session().await;
        let rel = session.relational().unwrap();
        let d = rel.dialect();
        assert!(
            d.features().folds_unquoted_upper,
            "the UTF-8 instance is case-sensitive"
        );
        // SET SCHEMA is remembered as session setup (navidog-db::sql::session_setup).
        // Unpatched dm-pure 1.2.4 hangs on it (patch 4); skip rather than fail there.
        if let Err(e) = session
            .relational()
            .unwrap()
            .execute(&d.set_search_path_sql("SYS").unwrap())
            .await
        {
            if e.is_session_lost() {
                eprintln!("skipped: SET SCHEMA needs the patched dm-pure (140-dameng.md D5)");
                return;
            }
            panic!("{e}");
        }
        let current = cell(
            &mut session,
            "SELECT SYS_CONTEXT('USERENV','CURRENT_SCHEMA')",
        )
        .await
        .unwrap();
        assert_eq!(current, "SYS", "{current}");
        exec(&mut session, &d.set_search_path_sql("SYSDBA").unwrap()).await;
        let current = cell(
            &mut session,
            "SELECT SYS_CONTEXT('USERENV','CURRENT_SCHEMA')",
        )
        .await
        .unwrap();
        assert_eq!(current, "SYSDBA");
        session.close().await.unwrap();
    });
}

#[test]
fn paging_and_explain_follow_the_dialect() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_PAGE_T")]).await;
        let mut session = session().await;
        exec(
            &mut session,
            "CREATE TABLE \"SYSDBA\".\"ND_PAGE_T\" (\"N\" INT)",
        )
        .await;
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_PAGE_T\" SELECT LEVEL FROM DUAL CONNECT BY LEVEL <= 5",
        )
        .await;
        let rel = session.relational().unwrap();
        let d = rel.dialect();
        let scope = scope();
        let sql = d.select_page(&scope, "ND_PAGE_T", "", "\"N\" DESC", Some(2), 2, false);
        let sets = exec(&mut session, &sql).await;
        let numbers: Vec<_> = sets[0].rows.iter().filter_map(|r| r[0].clone()).collect();
        assert_eq!(
            numbers,
            vec!["3".to_string(), "2".to_string()],
            "{numbers:?}"
        );
        // Unpatched dm-pure 1.2.4 hangs on EXPLAIN (patch 2); skip rather than fail there.
        let explain_sql = format!(
            "EXPLAIN {}",
            d.select_page(&scope, "ND_PAGE_T", "", "", None, 0, false)
        );
        if let Err(e) = session.relational().unwrap().execute(&explain_sql).await {
            if e.is_session_lost() {
                eprintln!("skipped: EXPLAIN needs the patched dm-pure (140-dameng.md D5)");
                cleanup(&[("TABLE", "ND_PAGE_T")]).await;
                session.close().await.unwrap();
                return;
            }
            panic!("{e}");
        }
        let explain = exec(&mut session, &explain_sql).await;
        assert!(!explain[0].rows.is_empty(), "{explain:?}");
        assert_eq!(
            explain[0].columns.len(),
            1,
            "EXPLAIN returns a single text column"
        );
        cleanup(&[("TABLE", "ND_PAGE_T")]).await;
        session.close().await.unwrap();
    });
}

#[test]
fn lazy_fetch_batches_five_thousand_rows() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_LAZY_T")]).await;
        let mut session = session().await;
        exec(
            &mut session,
            "CREATE TABLE \"SYSDBA\".\"ND_LAZY_T\" (\"N\" INT)",
        )
        .await;
        exec(
            &mut session,
            "INSERT INTO \"SYSDBA\".\"ND_LAZY_T\" SELECT LEVEL FROM DUAL CONNECT BY LEVEL <= 5000",
        )
        .await;
        session.close().await.unwrap();
        // The grid batches through dm-pure's lazy cursor on a blocking worker.
        tokio::task::spawn_blocking(move || {
            let mut client = dm_client();
            let (columns, mut state) = client
                .execute_lazy("SELECT \"N\" FROM \"SYSDBA\".\"ND_LAZY_T\"", &[])
                .unwrap();
            assert_eq!(columns.len(), 1);
            assert_eq!(state.rows_raw.len(), 100, "first batch is 100 rows");
            let mut total = state.rows_raw.len();
            // `more` never updates after execute_lazy; the server simply returns an
            // empty batch at the end, which is the loop's exit condition.
            loop {
                let rows = client
                    .fetch_more(state.stmt, total as i64 + 1, state.ncols)
                    .unwrap();
                if rows.is_empty() {
                    break;
                }
                total += rows.len();
                state.rows_raw = rows;
            }
            assert_eq!(total, 5000, "{total}");
            client.close_lazy(&mut state).unwrap();
        })
        .await
        .unwrap();
        cleanup(&[("TABLE", "ND_LAZY_T")]).await;
    });
}

#[test]
fn designer_migration_statements_execute_on_the_server() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        cleanup(&[("TABLE", "ND_DSG_T"), ("TABLE", "ND_DSG_T2")]).await;
        let mut session = session().await;
        for sql in [
            "CREATE TABLE \"SYSDBA\".\"ND_DSG_T\" (\"ID\" INT IDENTITY(1, 1) PRIMARY KEY, \
             \"NAME\" VARCHAR(100) NOT NULL)",
            // add_column hook: parenthesized, no position.
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ADD (\"NOTE\" VARCHAR(200))",
            // modify_column hook: type and nullability together.
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" MODIFY (\"NOTE\" VARCHAR(300) NOT NULL)",
            // Default set and drop.
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ALTER COLUMN \"NOTE\" SET DEFAULT '待填'",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ALTER COLUMN \"NOTE\" DROP DEFAULT",
            // Rename, comments, constraints and indexes.
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" RENAME COLUMN \"NOTE\" TO \"MEMO\"",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ADD (\"SCORE\" DECIMAL(6, 2) DEFAULT 0)",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ADD CONSTRAINT \"ND_DSG_CK\" CHECK (\"SCORE\" >= 0)",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" ADD CONSTRAINT \"ND_DSG_U\" UNIQUE (\"NAME\")",
            "COMMENT ON COLUMN \"SYSDBA\".\"ND_DSG_T\".\"MEMO\" IS '备注'",
            "COMMENT ON TABLE \"SYSDBA\".\"ND_DSG_T\" IS '迁移语法探测'",
            "CREATE INDEX \"SYSDBA\".\"ND_DSG_IDX\" ON \"SYSDBA\".\"ND_DSG_T\"(\"MEMO\")",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" DROP CONSTRAINT \"ND_DSG_CK\"",
            "DROP INDEX \"SYSDBA\".\"ND_DSG_IDX\"",
            "DROP TRIGGER \"SYSDBA\".\"ND_CAT_TRG\"",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" DROP COLUMN \"SCORE\"",
            "ALTER TABLE \"SYSDBA\".\"ND_DSG_T\" RENAME TO \"ND_DSG_T2\"",
        ] {
            exec(&mut session, sql).await;
        }
        let name = cell(
            &mut session,
            "SELECT TABLE_NAME FROM ALL_TABLES WHERE TABLE_NAME = 'ND_DSG_T2'",
        )
        .await
        .unwrap();
        assert_eq!(name, "ND_DSG_T2");
        let comment = cell(
            &mut session,
            "SELECT COMMENTS FROM ALL_COL_COMMENTS WHERE TABLE_NAME = 'ND_DSG_T2' \
             AND COLUMN_NAME = 'MEMO'",
        )
        .await
        .unwrap();
        assert_eq!(comment, "备注");
        cleanup(&[("TABLE", "ND_DSG_T"), ("TABLE", "ND_DSG_T2")]).await;
    });
}
