//! Driver tests against the docker-compose databases. Run with
//! `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-db --test live`.
//! Ports and accounts match docker-compose.yml at the repository root.

use std::sync::Arc;

use navidog_core::connection::SslMode;
use navidog_db::dialect::Dialect;
use navidog_db::params::{SshParams, SslParams};
use navidog_db::{
    ConnectParams, Driver, connect, mysql::MySqlDriver, postgres::PostgresDriver, runtime,
    test_connection,
};

fn live() -> bool {
    std::env::var_os("NAVIDOG_LIVE").is_some()
}

fn mysql_params(port: u16) -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "root".into(),
        password: Some("navidog".into()),
        ..Default::default()
    }
}

fn pg_params() -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port: 15432,
        user: "postgres".into(),
        password: Some("navidog".into()),
        database: Some("pagila".into()),
        ..Default::default()
    }
}

fn ssh() -> SshParams {
    SshParams {
        host: "127.0.0.1".into(),
        port: 12222,
        user: "navidog".into(),
        password: Some("navidog".into()),
        ..Default::default()
    }
}

async fn databases(driver: Arc<dyn Driver>, params: ConnectParams) -> Vec<String> {
    let mut s = connect(driver, params).await.unwrap();
    let dbs = s.relational().unwrap().list_databases().await.unwrap();
    s.close().await.unwrap();
    dbs
}

#[test]
fn mysql_and_mariadb_list_databases_and_report_auth_errors() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, port) in [
            (MySqlDriver::mysql(), 13306),
            (MySqlDriver::mariadb(), 13307),
        ] {
            let driver: Arc<dyn Driver> = Arc::new(driver);
            let dbs = databases(driver.clone(), mysql_params(port)).await;
            assert!(dbs.iter().any(|d| d == "sakila"), "{dbs:?}");
            let mut bad = mysql_params(port);
            bad.password = Some("wrong".into());
            let e = test_connection(driver, bad).await.unwrap_err();
            assert_eq!(e.code.as_deref(), Some("1045"), "{e}");
        }
    });
}

#[test]
fn postgres_ssl_modes() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let driver: Arc<dyn Driver> = Arc::new(PostgresDriver::default());
        assert!(
            databases(driver.clone(), pg_params())
                .await
                .contains(&"pagila".to_string())
        );
        for mode in [SslMode::Prefer, SslMode::Require] {
            let mut p = pg_params();
            p.ssl = Some(SslParams {
                mode,
                ..Default::default()
            });
            test_connection(driver.clone(), p).await.unwrap();
        }
        let mut p = pg_params();
        p.ssl = Some(SslParams {
            mode: SslMode::VerifyCa,
            ..Default::default()
        });
        // Must fail with a self-signed certificate and no root certificate provided
        assert!(test_connection(driver, p).await.is_err());
    });
}

#[test]
fn postgres_multiple_nodes() {
    use navidog_core::connection::HostPreference;
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let driver: Arc<dyn Driver> = Arc::new(PostgresDriver::default());
        // The first node is down, so the member is used
        let mut p = pg_params();
        p.port = 1;
        p.members = vec![("127.0.0.1".into(), 15432)];
        test_connection(driver.clone(), p.clone()).await.unwrap();
        p.host_preference = HostPreference::ReadWrite;
        test_connection(driver.clone(), p.clone()).await.unwrap();
        // The only reachable node is a primary, so asking for a read-only one fails
        p.host_preference = HostPreference::ReadOnly;
        assert!(test_connection(driver, p).await.is_err());
    });
}

#[test]
fn mysql_and_postgres_metadata() {
    use navidog_core::object::{ObjectKind, Scope};
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut my = connect(Arc::new(MySqlDriver::mysql()), mysql_params(13306))
            .await
            .unwrap();
        let rel = my.relational().unwrap();
        let scope = Scope::new("c", "sakila", None);
        let tables = rel.list_objects(&scope, ObjectKind::Table).await.unwrap();
        assert_eq!(
            tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["actor", "film", "film_actor"]
        );
        assert_eq!(tables[0].comment.as_deref(), Some("演员"));
        assert!(
            tables[0]
                .properties
                .iter()
                .any(|(k, v)| k == "engine" && v == "InnoDB")
        );
        let views = rel.list_objects(&scope, ObjectKind::View).await.unwrap();
        assert_eq!(views[0].name, "actor_info");
        let ddl = rel
            .object_ddl(&scope, ObjectKind::Table, "actor")
            .await
            .unwrap();
        assert!(ddl.contains("CREATE TABLE `actor`"), "{ddl}");
        let cols = rel.table_columns(&scope, "film_actor").await.unwrap();
        assert!(cols.iter().all(|c| c.primary_key));
        my.close().await.unwrap();

        let mut pg = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        let rel = pg.relational().unwrap();
        let schemas = rel.list_schemas().await.unwrap();
        let user: Vec<_> = schemas
            .iter()
            .filter(|s| !Dialect::Postgres.is_system_schema(s))
            .collect();
        assert_eq!(user, ["inventory", "public"]);
        assert!(
            schemas.iter().any(|s| s == "pg_catalog"),
            "system schemas are listed for Show Hidden Items"
        );
        let scope = Scope::new("c", "pagila", Some("public".into()));
        let tables = rel.list_objects(&scope, ObjectKind::Table).await.unwrap();
        assert_eq!(
            tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["actor", "film"]
        );
        assert_eq!(tables[0].comment.as_deref(), Some("演员"));
        let mv = rel
            .list_objects(&scope, ObjectKind::MaterializedView)
            .await
            .unwrap();
        assert_eq!(mv[0].name, "film_count");
        let seqs = rel
            .list_objects(&scope, ObjectKind::Sequence)
            .await
            .unwrap();
        assert!(seqs.iter().any(|s| s.name == "actor_actor_id_seq"));
        let ddl = rel
            .object_ddl(&scope, ObjectKind::Table, "actor")
            .await
            .unwrap();
        assert!(
            ddl.contains("CREATE TABLE \"public\".\"actor\"") && ddl.contains("PRIMARY KEY"),
            "{ddl}"
        );
        let ddl = rel
            .object_ddl(&scope, ObjectKind::MaterializedView, "film_count")
            .await
            .unwrap();
        assert!(ddl.starts_with("CREATE MATERIALIZED VIEW"), "{ddl}");
        let cols = rel.table_columns(&scope, "actor").await.unwrap();
        assert!(cols[0].primary_key && !cols[0].nullable && cols[0].default.is_some());
        pg.close().await.unwrap();
    });
}

/// SQL generated by the table viewer: paging, counting, change sets. Changes run in a transaction
/// that is rolled back, leaving the seed data untouched.
#[test]
fn table_viewer_sql_on_mysql_and_postgres() {
    use navidog_core::object::Scope;
    use navidog_db::dialect::{ChangeSet, RowKey, RowUpdate, Written};
    if !live() {
        return;
    }
    let v = |c: &str, s: &str| (c.to_string(), Some(s.to_string()));
    let w = |c: &str, s: &str| (c.to_string(), Written::text(Some(s.to_string())));
    let changes = ChangeSet {
        updates: vec![RowUpdate {
            key: RowKey(vec![v("actor_id", "1")]),
            values: vec![w("first_name", "O'Hara\\")],
        }],
        inserts: vec![vec![w("first_name", "NEW"), w("last_name", "ROW")]],
        deletes: vec![],
    };
    runtime::runtime().block_on(async {
        let mut my = connect(Arc::new(MySqlDriver::mysql()), mysql_params(13306))
            .await
            .unwrap();
        let mut pg = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        let cases = [
            (&mut my, Scope::new("c", "sakila", None), 3, "4"),
            (
                &mut pg,
                Scope::new("c", "pagila", Some("public".into())),
                2,
                "3",
            ),
        ];
        for (session, scope, after_filter, max_id) in cases {
            let rel = session.relational().unwrap();
            let d = rel.dialect();
            let order = format!("{} DESC", d.quote_ident("actor_id"));
            let page = rel
                .execute(&d.select_page(&scope, "actor", "actor_id > 1", &order, Some(2), 0, false))
                .await
                .unwrap();
            assert_eq!(page[0].rows.len(), 2);
            assert_eq!(page[0].rows[0][0].as_deref(), Some(max_id));
            let count = rel
                .execute(&d.count(&scope, "actor", "actor_id > 1"))
                .await
                .unwrap();
            assert_eq!(
                count[0].rows[0][0].as_deref(),
                Some(after_filter.to_string().as_str())
            );

            rel.execute(d.begin_transaction()).await.unwrap();
            for sql in d.render_changes(&scope, "actor", &changes, true) {
                rel.execute(&sql)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"));
            }
            let name = rel
                .execute(&d.select_page(&scope, "actor", "actor_id = 1", "", None, 0, false))
                .await
                .unwrap();
            assert_eq!(name[0].rows[0][1].as_deref(), Some("O'Hara\\"));
            let n = rel
                .execute(&d.count(&scope, "actor", "first_name = 'NEW'"))
                .await
                .unwrap();
            assert_eq!(n[0].rows[0][0].as_deref(), Some("1"));
            rel.execute("ROLLBACK").await.unwrap();
            let n = rel
                .execute(&d.count(&scope, "actor", "first_name = 'NEW'"))
                .await
                .unwrap();
            assert_eq!(n[0].rows[0][0].as_deref(), Some("0"));
        }
        my.close().await.unwrap();
        pg.close().await.unwrap();
    });
}

/// Navigator object operations: table children, duplicate (structure + data) and rename.
/// MySQL DDL commits implicitly, so the copies are dropped explicitly.
#[test]
fn object_operations_on_mysql_and_postgres() {
    use navidog_core::object::{ObjectKind, Scope};
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut my = connect(Arc::new(MySqlDriver::mysql()), mysql_params(13306))
            .await
            .unwrap();
        let mut pg = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        let cases = [
            (
                &mut my,
                Scope::new("c", "sakila", None),
                "film_actor",
                "fk_film_actor_actor",
            ),
            (
                &mut pg,
                Scope::new("c", "pagila", Some("public".into())),
                "actor",
                "",
            ),
        ];
        for (session, scope, table, fk) in cases {
            let rel = session.relational().unwrap();
            let d = rel.dialect();
            let children = rel.table_children(&scope, table).await.unwrap();
            assert!(!children.indexes.is_empty(), "{children:?}");
            if !fk.is_empty() {
                assert!(
                    children.foreign_keys.iter().any(|f| f == fk),
                    "{children:?}"
                );
            }
            for sql in d
                .duplicate_table(&scope, "actor", "actor_copy1", true, None)
                .unwrap()
            {
                rel.execute(&sql)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"));
            }
            let rename = d
                .rename_object(&scope, ObjectKind::Table, "actor_copy1", "actor_copy2")
                .unwrap();
            rel.execute(&rename).await.unwrap();
            let n = rel
                .execute(&d.count(&scope, "actor_copy2", ""))
                .await
                .unwrap();
            let orig = rel.execute(&d.count(&scope, "actor", "")).await.unwrap();
            assert_eq!(n[0].rows, orig[0].rows, "the copy has the same rows");
            rel.execute(&format!(
                "DROP TABLE {}",
                d.qualified(&scope, "actor_copy2")
            ))
            .await
            .unwrap();
        }
        my.close().await.unwrap();
        pg.close().await.unwrap();
    });
}

/// Query editor needs (31-query-results.md §1): column origins and SQL type names, session IDs,
/// PostgreSQL notices, DELIMITER scripts and the MySQL read timeout.
#[test]
fn query_editor_driver_features() {
    use navidog_db::sql::split_script;
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut params = mysql_params(13306);
        params.database = Some("sakila".into());
        params.read_timeout = Some(std::time::Duration::from_secs(1));
        let mut my = connect(Arc::new(MySqlDriver::mysql()), params)
            .await
            .unwrap();
        assert!(my.session_id().unwrap() > 0);
        let rel = my.relational().unwrap();
        let sets = rel
            .execute_statement("SELECT a.actor_id, a.first_name AS n, 1 + 1 FROM actor a LIMIT 1")
            .await
            .unwrap();
        let cols = &sets[0].columns;
        let origin = cols[1].origin.clone().unwrap();
        assert_eq!(
            (
                origin.schema.as_str(),
                origin.table.as_str(),
                origin.column.as_str()
            ),
            ("sakila", "actor", "first_name")
        );
        assert!(cols[0].type_name.contains("int"), "{:?}", cols[0]);
        assert_eq!(cols[1].type_name, "varchar");
        assert!(cols[2].origin.is_none());

        let script = "DROP PROCEDURE IF EXISTS navidog_p;\nDELIMITER $$\n\
                      CREATE PROCEDURE navidog_p() BEGIN SELECT 1; SELECT 2; END$$\n\
                      DELIMITER ;\nCALL navidog_p();\nDROP PROCEDURE navidog_p";
        let mut sets = Vec::new();
        for stmt in split_script(script, Dialect::MySql) {
            sets.extend(rel.execute(&stmt.text).await.unwrap());
        }
        assert!(sets.iter().filter(|s| !s.columns.is_empty()).count() >= 2);

        let outline = navidog_db::outline::fetch(
            &mut *rel,
            &navidog_core::object::Scope::new("c", "sakila", None),
            false,
        )
        .await
        .unwrap();
        assert!(
            outline
                .table("actor")
                .unwrap()
                .columns
                .iter()
                .any(|(c, _)| c == "first_name")
        );
        assert!(outline.table("actor_info").unwrap().view);
        assert!(outline.schemas.contains(&"sakila".to_string()));
        assert!(
            !outline.schemas.contains(&"mysql".to_string()),
            "system databases are off"
        );

        let e = rel.execute("SELECT SLEEP(5)").await.unwrap_err();
        assert!(e.message.contains("超时"), "{e}");
        // The session survives the kill
        let one = rel.execute("SELECT 1").await.unwrap();
        assert_eq!(one[0].rows[0][0].as_deref(), Some("1"));
        my.close().await.unwrap();

        let mut pg = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        assert!(pg.session_id().unwrap() > 0);
        let rel = pg.relational().unwrap();
        let sets = rel
            .execute_statement("SELECT actor_id, first_name AS n, now() FROM actor LIMIT 1")
            .await
            .unwrap();
        let cols = &sets[0].columns;
        assert_eq!(cols[0].type_name, "int4");
        assert_eq!(
            cols[1].origin.as_ref().map(|o| (
                o.schema.as_str(),
                o.table.as_str(),
                o.column.as_str()
            )),
            Some(("public", "actor", "first_name"))
        );
        assert!(cols[2].origin.is_none());
        let sets = rel
            .execute("DO $$ BEGIN RAISE NOTICE 'hello %', 1; END $$")
            .await
            .unwrap();
        assert_eq!(sets.last().unwrap().notices, ["NOTICE: hello 1"]);
        let outline = navidog_db::outline::fetch(
            &mut *rel,
            &navidog_core::object::Scope::new("c", "pagila", Some("public".into())),
            true,
        )
        .await
        .unwrap();
        let actor = outline.table("actor").unwrap();
        assert!(
            actor
                .columns
                .iter()
                .any(|(c, t)| c == "first_name" && !t.is_empty()),
            "{actor:?}"
        );
        assert!(
            outline.table("pg_class").is_some(),
            "system objects when asked"
        );
        pg.close().await.unwrap();
    });
}

/// "Stop" on a PostgreSQL session that requires SSL: the cancel request goes over TLS too.
#[test]
fn postgres_cancel_over_tls() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let mut p = pg_params();
        p.ssl = Some(SslParams {
            mode: SslMode::Require,
            ..Default::default()
        });
        let mut pg = connect(Arc::new(PostgresDriver::default()), p)
            .await
            .unwrap();
        let cancel = pg.cancel_handle().unwrap();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            cancel().await
        });
        let started = std::time::Instant::now();
        let e = pg
            .relational()
            .unwrap()
            .execute("SELECT pg_sleep(10)")
            .await
            .unwrap_err();
        canceller.await.unwrap().unwrap();
        assert_eq!(e.code.as_deref(), Some("57014"), "{e}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        pg.close().await.unwrap();
    });
}

/// `USE` / `SET search_path` survive a transparent reconnect. Unqualified names must not
/// fall back to the connection's default database.
#[test]
fn reconnect_keeps_the_selected_database() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let driver: Arc<dyn Driver> = Arc::new(MySqlDriver::mysql());
        let mut session = connect(driver.clone(), mysql_params(13306)).await.unwrap();
        let mut bootstrap = connect(driver.clone(), mysql_params(13306)).await.unwrap();
        bootstrap
            .relational()
            .unwrap()
            .execute("CREATE DATABASE IF NOT EXISTS navidog_cr01")
            .await
            .unwrap();
        bootstrap.close().await.unwrap();
        session
            .relational()
            .unwrap()
            .execute("USE navidog_cr01")
            .await
            .unwrap();
        let id = session.session_id().unwrap();
        let mut killer = connect(driver, mysql_params(13306)).await.unwrap();
        killer
            .relational()
            .unwrap()
            .execute(&format!("KILL {id}"))
            .await
            .unwrap();
        killer.close().await.unwrap();
        let sets = session
            .relational()
            .unwrap()
            .execute_read("SELECT DATABASE()")
            .await
            .unwrap();
        assert_eq!(sets[0].rows[0][0].as_deref(), Some("navidog_cr01"));
        session
            .relational()
            .unwrap()
            .execute("DROP DATABASE navidog_cr01")
            .await
            .unwrap();
        session.close().await.unwrap();

        let mut session = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        let mut bootstrap = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        bootstrap
            .relational()
            .unwrap()
            .execute("CREATE SCHEMA IF NOT EXISTS navidog_cr01")
            .await
            .unwrap();
        bootstrap.close().await.unwrap();
        session
            .relational()
            .unwrap()
            .execute("SET search_path TO navidog_cr01, public")
            .await
            .unwrap();
        let pid = session.session_id().unwrap();
        let mut killer = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        killer
            .relational()
            .unwrap()
            .execute(&format!("SELECT pg_terminate_backend({pid})"))
            .await
            .unwrap();
        killer.close().await.unwrap();
        let sets = session
            .relational()
            .unwrap()
            .execute_read("SELECT current_schema()")
            .await
            .unwrap();
        assert_eq!(sets[0].rows[0][0].as_deref(), Some("navidog_cr01"));
        session
            .relational()
            .unwrap()
            .execute("DROP SCHEMA navidog_cr01")
            .await
            .unwrap();
        session.close().await.unwrap();
    });
}

/// Explicitly stateless reads recover once after an idle session is killed.
#[test]
fn reconnects_after_the_server_kills_the_session() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        let driver: Arc<dyn Driver> = Arc::new(MySqlDriver::mysql());
        let mut session = connect(driver.clone(), mysql_params(13306)).await.unwrap();
        let id = session.session_id().unwrap();
        let mut killer = connect(driver, mysql_params(13306)).await.unwrap();
        killer
            .relational()
            .unwrap()
            .execute(&format!("KILL {id}"))
            .await
            .unwrap();
        killer.close().await.unwrap();
        let sets = session
            .relational()
            .unwrap()
            .execute_read("SELECT 1")
            .await
            .unwrap();
        assert_eq!(sets[0].rows, [vec![Some("1".to_string())]]);
        assert_ne!(session.session_id(), Some(id));
        session.close().await.unwrap();

        let mut session = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        let pid = session.session_id().unwrap();
        let mut killer = connect(Arc::new(PostgresDriver::default()), pg_params())
            .await
            .unwrap();
        killer
            .relational()
            .unwrap()
            .execute(&format!("SELECT pg_terminate_backend({pid})"))
            .await
            .unwrap();
        killer.close().await.unwrap();
        let sets = session
            .relational()
            .unwrap()
            .execute_read("SELECT 1")
            .await
            .unwrap();
        assert_eq!(sets[0].rows, [vec![Some("1".to_string())]]);
        assert_ne!(session.session_id(), Some(pid));
        session.close().await.unwrap();
    });
}

/// An application write on its own session must not pin the metadata session, and recovering
/// that session must not submit the write again.
#[test]
fn a_separate_write_leaves_the_metadata_session_recoverable() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, mut params) in network_drivers() {
            if driver.info().id.as_ref() != "postgres" {
                params.database = Some("sakila".into());
            }
            let mut metadata = connect(driver.clone(), params.clone()).await.unwrap();
            let mut writer = connect(driver.clone(), params.clone()).await.unwrap();
            let dialect = metadata.relational().unwrap().dialect();
            let table = format!("navidog_meta_{}_{}", params.port, std::process::id());
            let quoted = dialect.quote_ident(&table);
            metadata
                .relational()
                .unwrap()
                .execute_read("SELECT 1")
                .await
                .unwrap();
            let id = metadata.session_id().unwrap();
            writer
                .relational()
                .unwrap()
                .execute(&format!(
                    "DROP TABLE IF EXISTS {quoted}; CREATE TABLE {quoted} (n INTEGER); INSERT INTO {quoted} VALUES (1)"
                ))
                .await
                .unwrap();
            kill_backend(writer.as_mut(), dialect, id).await;
            writer.close().await.unwrap();
            let sets = metadata
                .relational()
                .unwrap()
                .execute_read(&format!("SELECT COUNT(*) FROM {quoted}"))
                .await
                .unwrap();
            assert_eq!(sets[0].rows, [vec![Some("1".to_string())]], "{table}");
            assert_ne!(metadata.session_id(), Some(id), "{table}");
            metadata
                .relational()
                .unwrap()
                .execute(&format!("DROP TABLE {quoted}"))
                .await
                .unwrap();
            metadata.close().await.unwrap();
        }
    });
}

#[test]
fn ssh_tunnel_to_mysql_and_postgres() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        // Inside the tunnel, use the compose network's service names and container ports
        let mut my = mysql_params(3306);
        my.host = "mysql".into();
        my.ssh = Some(ssh());
        let v = test_connection(Arc::new(MySqlDriver::mysql()), my)
            .await
            .unwrap();
        assert!(v.starts_with('8'), "{v}");

        let mut pg = pg_params();
        pg.host = "postgres".into();
        pg.port = 5432;
        pg.ssh = Some(ssh());
        test_connection(Arc::new(PostgresDriver::default()), pg)
            .await
            .unwrap();
    });
}

fn network_drivers() -> Vec<(Arc<dyn Driver>, ConnectParams)> {
    vec![
        (Arc::new(MySqlDriver::mysql()), mysql_params(13306)),
        (Arc::new(MySqlDriver::mariadb()), mysql_params(13307)),
        (Arc::new(PostgresDriver::default()), pg_params()),
    ]
}

async fn kill_backend(observer: &mut dyn navidog_db::Session, dialect: Dialect, id: u64) {
    let sql = match dialect {
        Dialect::MySql => format!("KILL {id}"),
        Dialect::Postgres => format!("SELECT pg_terminate_backend({id})"),
        _ => unreachable!(),
    };
    observer.relational().unwrap().execute(&sql).await.unwrap();
}

async fn wait_for_backend_sleep(observer: &mut dyn navidog_db::Session, dialect: Dialect, id: u64) {
    let sql = match dialect {
        Dialect::MySql => format!(
            "SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE ID={id} AND STATE='User sleep'"
        ),
        Dialect::Postgres => {
            format!("SELECT COUNT(*) FROM pg_stat_activity WHERE pid={id} AND wait_event='PgSleep'")
        }
        _ => unreachable!(),
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let sets = observer.relational().unwrap().execute(&sql).await.unwrap();
            if sets[0].rows[0][0].as_deref() == Some("1") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("backend never reached the response-delay statement");
}

/// The first write has committed, but the caller loses the response while the script is still
/// running. Neither ordinary execution path may submit that script a second time.
#[test]
fn uncertain_writes_are_never_replayed() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, mut params) in network_drivers() {
            if driver.info().id.as_ref() != "postgres" {
                params.database = Some("sakila".into());
            }
            for statement_api in [false, true] {
                let mut observer = connect(driver.clone(), params.clone()).await.unwrap();
                let mut session = connect(driver.clone(), params.clone()).await.unwrap();
                let dialect = session.relational().unwrap().dialect();
                let table = format!(
                    "navidog_no_replay_{}_{}",
                    std::process::id(),
                    u8::from(statement_api)
                );
                let quoted = dialect.quote_ident(&table);
                observer
                    .relational()
                    .unwrap()
                    .execute(&format!(
                        "DROP TABLE IF EXISTS {quoted}; CREATE TABLE {quoted} (n INTEGER)"
                    ))
                    .await
                    .unwrap();
                let id = session.session_id().unwrap();
                let script = match dialect {
                    Dialect::MySql => format!("INSERT INTO {quoted} VALUES (1); SELECT SLEEP(30)"),
                    Dialect::Postgres => format!(
                        "BEGIN; INSERT INTO {quoted} VALUES (1); COMMIT; SELECT pg_sleep(30)"
                    ),
                    _ => unreachable!(),
                };
                let mut job = runtime::spawn(async move {
                    let rel = session.relational().unwrap();
                    let result = if statement_api {
                        rel.execute_statement(&script).await
                    } else {
                        rel.execute(&script).await
                    };
                    (session, result)
                });
                wait_for_backend_sleep(observer.as_mut(), dialect, id).await;
                kill_backend(observer.as_mut(), dialect, id).await;
                let finished =
                    tokio::time::timeout(std::time::Duration::from_secs(5), &mut job).await;
                let promptly = finished.is_ok();
                // Let a broken replay finish before removing its fixture; avoid detached writers.
                let (session, result) = match finished {
                    Ok(joined) => joined.unwrap(),
                    Err(_) => job.await.unwrap(),
                };
                let rows = observer
                    .relational()
                    .unwrap()
                    .execute(&format!("SELECT COUNT(*) FROM {quoted}"))
                    .await
                    .unwrap();
                observer
                    .relational()
                    .unwrap()
                    .execute(&format!("DROP TABLE {quoted}"))
                    .await
                    .unwrap();
                let same_session = session.session_id() == Some(id);
                let _ = session.close().await;
                observer.close().await.unwrap();
                assert!(promptly, "a disconnected write was replayed or stalled");
                let error = result.unwrap_err();
                assert!(error.is_session_lost(), "{error}");
                assert!(error.is_outcome_unknown(), "{error}");
                assert!(
                    same_session,
                    "a write silently moved to a different physical session"
                );
                assert_eq!(rows[0].rows[0][0].as_deref(), Some("1"));
            }
        }
    });
}

#[test]
fn transaction_loss_cannot_reconnect_through_metadata_or_ping() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, params) in network_drivers() {
            let mut session = connect(driver.clone(), params.clone()).await.unwrap();
            let mut observer = connect(driver, params).await.unwrap();
            let dialect = session.relational().unwrap().dialect();
            session
                .relational()
                .unwrap()
                .execute("BEGIN")
                .await
                .unwrap();
            let id = session.session_id().unwrap();
            kill_backend(observer.as_mut(), dialect, id).await;
            let read = session
                .relational()
                .unwrap()
                .execute_read("SELECT 1")
                .await
                .unwrap_err();
            assert!(read.is_session_lost(), "{read}");
            assert!(!read.is_outcome_unknown(), "{read}");
            assert!(session.ping().await.is_err());
            assert!(
                session
                    .relational()
                    .unwrap()
                    .execute("COMMIT")
                    .await
                    .is_err()
            );
            assert_eq!(session.session_id(), Some(id));
            assert!(session.cancel_handle().unwrap()().await.is_err());
            let _ = session.close().await;
            observer.close().await.unwrap();
        }
    });
}

#[test]
fn captured_cancel_handle_follows_safe_reconnect() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        for (driver, params) in network_drivers() {
            let mut session = connect(driver.clone(), params.clone()).await.unwrap();
            let mut observer = connect(driver, params).await.unwrap();
            let dialect = session.relational().unwrap().dialect();
            let old_id = session.session_id().unwrap();
            let cancel = session.cancel_handle().unwrap();
            kill_backend(observer.as_mut(), dialect, old_id).await;
            session
                .relational()
                .unwrap()
                .execute_read("SELECT 1")
                .await
                .unwrap();
            let id = session.session_id().unwrap();
            assert_ne!(id, old_id);
            let sql = match dialect {
                Dialect::MySql => "SELECT SLEEP(30)",
                Dialect::Postgres => "SELECT pg_sleep(30)",
                _ => unreachable!(),
            };
            let mut job = runtime::spawn(async move {
                let result = session.relational().unwrap().execute(sql).await;
                (session, result)
            });
            wait_for_backend_sleep(observer.as_mut(), dialect, id).await;
            let cancel_clock = std::time::Instant::now();
            let cancellation =
                tokio::time::timeout(std::time::Duration::from_secs(5), cancel()).await;
            let finished = tokio::time::timeout(std::time::Duration::from_secs(5), &mut job).await;
            let promptly =
                finished.is_ok() && cancel_clock.elapsed() < std::time::Duration::from_secs(5);
            let (mut session, result) = match finished {
                Ok(joined) => joined.unwrap(),
                Err(_) => {
                    kill_backend(observer.as_mut(), dialect, id).await;
                    job.await.unwrap()
                }
            };
            let still_usable = session
                .relational()
                .unwrap()
                .execute("SELECT 1")
                .await
                .is_ok();
            let _ = session.close().await;
            observer.close().await.unwrap();
            assert!(cancellation.is_ok(), "{cancellation:?}");
            assert!(
                promptly,
                "the old cancellation handle did not reach the new backend"
            );
            match (dialect, result) {
                (Dialect::MySql, Ok(sets)) => {
                    assert_eq!(
                        sets[0].rows,
                        [vec![Some("1".into())]],
                        "SLEEP must be interrupted"
                    );
                }
                (Dialect::MySql, Err(error)) => {
                    assert_eq!(error.code.as_deref(), Some("1317"), "{error}");
                    assert!(!error.disconnected);
                }
                (Dialect::Postgres, Err(error)) => {
                    assert_eq!(error.code.as_deref(), Some("57014"), "{error}");
                    assert!(!error.disconnected);
                }
                (_, other) => panic!("unexpected cancellation result: {other:?}"),
            }
            assert!(still_usable);
            assert!(
                cancel().await.is_err(),
                "closed sessions must invalidate old handles"
            );
        }
    });
}
