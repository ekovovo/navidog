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
