//! 依赖 docker-compose 环境的驱动测试。运行方式：
//! `docker compose up -d --wait && NAVIDOG_LIVE=1 cargo test -p navidog-db --test live`
//! 端口和账号与仓库根目录的 docker-compose.yml 一致。

use std::sync::Arc;

use navidog_core::connection::SslMode;
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
        // 自签名证书且未提供根证书时必须失败
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
        assert_eq!(rel.list_schemas().await.unwrap(), ["inventory", "public"]);
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

/// 表查看器生成的 SQL：分页、计数、修改集合；修改在事务中执行后回滚，不改动种子数据。
#[test]
fn table_viewer_sql_on_mysql_and_postgres() {
    use navidog_core::object::Scope;
    use navidog_db::dialect::{ChangeSet, RowKey, RowUpdate};
    if !live() {
        return;
    }
    let v = |c: &str, s: &str| (c.to_string(), Some(s.to_string()));
    let changes = ChangeSet {
        updates: vec![RowUpdate {
            key: RowKey(vec![v("actor_id", "1")]),
            values: vec![v("first_name", "O'Hara\\")],
        }],
        inserts: vec![vec![v("first_name", "NEW"), v("last_name", "ROW")]],
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

#[test]
fn ssh_tunnel_to_mysql_and_postgres() {
    if !live() {
        return;
    }
    runtime::runtime().block_on(async {
        // 隧道内访问 compose 网络中的服务名与容器端口
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
