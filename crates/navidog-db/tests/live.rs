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
                .execute(&d.select_page(&scope, "actor", "actor_id > 1", &order, Some(2), 0))
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
                .execute(&d.select_page(&scope, "actor", "actor_id = 1", "", None, 0))
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
