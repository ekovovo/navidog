use std::sync::Arc;

use navidog_db::{ConnectParams, Driver, connect, runtime, sqlite::SqliteDriver};

#[test]
fn sqlite_create_attach_execute_and_error_codes() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.db");
    let other = dir.path().join("other.db");

    runtime::runtime().block_on(async {
        // 附加库先建出来
        let mk = ConnectParams { file: Some(other.clone()), create_new: true, ..Default::default() };
        let s = connect(Arc::new(SqliteDriver::default()), mk).await.unwrap();
        s.close().await.unwrap();

        let driver: Arc<dyn Driver> = Arc::new(SqliteDriver::default());
        let missing = ConnectParams { file: Some(dir.path().join("nope.db")), ..Default::default() };
        assert!(connect(driver.clone(), missing).await.is_err());

        let params = ConnectParams {
            file: Some(main.clone()),
            create_new: true,
            attached: vec![("other".into(), other.clone())],
            initial_queries: "PRAGMA foreign_keys = ON".into(),
            ..Default::default()
        };
        let mut session = connect(driver, params).await.unwrap();
        assert!(session.server_version().starts_with('3'));
        session.ping().await.unwrap();
        assert!(session.cancel_handle().is_some());

        let rel = session.relational().unwrap();
        assert_eq!(rel.list_databases().await.unwrap(), ["main", "other"]);
        let sets = rel
            .execute("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t(name) VALUES ('a'), (NULL); SELECT id, name FROM t")
            .await
            .unwrap();
        assert_eq!(sets.len(), 3);
        assert_eq!(sets[1].affected_rows, Some(2));
        assert_eq!(sets[2].columns[1].type_name, "TEXT");
        assert_eq!(sets[2].rows, vec![vec![Some("1".into()), Some("a".into())], vec![Some("2".into()), None]]);

        let e = rel.execute("SELECT * FROM missing").await.unwrap_err();
        assert!(e.code.is_some(), "保留 SQLite 错误码: {e}");
        session.close().await.unwrap();
    });
    assert!(main.exists());
}
