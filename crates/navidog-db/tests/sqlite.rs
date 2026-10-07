use std::sync::Arc;

use navidog_db::dialect::Dialect;
use navidog_db::{ConnectParams, Driver, connect, runtime, sqlite::SqliteDriver};

#[test]
fn sqlite_create_attach_execute_and_error_codes() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.db");
    let other = dir.path().join("other.db");

    runtime::runtime().block_on(async {
        // Create the attached database first
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

#[test]
fn sqlite_metadata_and_change_sets() {
    use navidog_core::object::{ObjectKind, Scope};
    use navidog_db::dialect::{ChangeSet, RowKey, RowUpdate, Written};

    let dir = tempfile::tempdir().unwrap();
    let params = ConnectParams {
        file: Some(dir.path().join("m.db")),
        create_new: true,
        ..Default::default()
    };
    runtime::runtime().block_on(async {
        let mut session = connect(Arc::new(SqliteDriver::default()), params)
            .await
            .unwrap();
        let rel = session.relational().unwrap();
        rel.execute(
            "CREATE TABLE actor(id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'x', note TEXT);\
             CREATE VIEW v_actor AS SELECT name FROM actor;\
             CREATE INDEX idx_name ON actor(name);\
             CREATE TRIGGER trg AFTER INSERT ON actor BEGIN SELECT 1; END",
        )
        .await
        .unwrap();
        let scope = Scope::new("c", "main", None);
        rel.execute(
            "CREATE TABLE role(id INTEGER PRIMARY KEY, actor_id INTEGER REFERENCES actor(id))",
        )
        .await
        .unwrap();
        let children = rel.table_children(&scope, "actor").await.unwrap();
        assert_eq!(
            (children.indexes, children.triggers),
            (vec!["idx_name".into()], vec!["trg".into()])
        );
        let role = rel.table_children(&scope, "role").await.unwrap();
        assert_eq!(role.foreign_keys, ["→ actor"]);
        // Duplicating rewrites the CREATE TABLE statement, keeping the primary key
        let ddl = rel
            .object_ddl(&scope, ObjectKind::Table, "role")
            .await
            .unwrap();
        for sql in Dialect::Sqlite
            .duplicate_table(&scope, "role", "role_copy1", true, Some(&ddl))
            .unwrap()
        {
            rel.execute(&sql).await.unwrap();
        }
        let cols = rel.table_columns(&scope, "role_copy1").await.unwrap();
        assert!(cols[0].primary_key, "{ddl}");
        rel.execute("DROP TABLE role_copy1; DROP TABLE role")
            .await
            .unwrap();
        assert_eq!(rel.list_schemas().await.unwrap(), Vec::<String>::new());
        assert_eq!(
            rel.object_kinds(),
            [
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::Index,
                ObjectKind::Trigger
            ]
        );
        let names =
            |v: Vec<navidog_db::ObjectInfo>| v.into_iter().map(|o| o.name).collect::<Vec<_>>();
        assert_eq!(
            names(rel.list_objects(&scope, ObjectKind::Table).await.unwrap()),
            ["actor"]
        );
        assert_eq!(
            names(rel.list_objects(&scope, ObjectKind::View).await.unwrap()),
            ["v_actor"]
        );
        assert_eq!(
            names(rel.list_objects(&scope, ObjectKind::Index).await.unwrap()),
            ["idx_name"]
        );
        assert_eq!(
            names(rel.list_objects(&scope, ObjectKind::Trigger).await.unwrap()),
            ["trg"]
        );
        let ddl = rel
            .object_ddl(&scope, ObjectKind::Table, "actor")
            .await
            .unwrap();
        assert!(ddl.starts_with("CREATE TABLE actor"), "{ddl}");

        let cols = rel.table_columns(&scope, "actor").await.unwrap();
        assert_eq!(
            cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["id", "name", "note"]
        );
        assert!(cols[0].primary_key && !cols[1].nullable && cols[2].nullable);
        assert_eq!(cols[1].default.as_deref(), Some("'x'"));

        // Run grid change sets against a real database
        let d = rel.dialect();
        let insert = ChangeSet {
            inserts: vec![
                vec![
                    ("id".into(), Written::text(Some("1".into()))),
                    ("name".into(), Written::text(Some("O'Brien".into()))),
                ],
                vec![
                    ("id".into(), Written::text(Some("2".into()))),
                    ("name".into(), Written::text(Some("b".into()))),
                    ("note".into(), Written::text(None)),
                ],
            ],
            ..Default::default()
        };
        for sql in d.render_changes(&scope, "actor", &insert, true) {
            rel.execute(&sql).await.unwrap();
        }
        let update = ChangeSet {
            updates: vec![RowUpdate {
                key: RowKey(vec![("id".into(), Some("1".into()))]),
                values: vec![("note".into(), Written::text(Some("hi".into())))],
            }],
            deletes: vec![RowKey(vec![("id".into(), Some("2".into()))])],
            ..Default::default()
        };
        for sql in d.render_changes(&scope, "actor", &update, true) {
            rel.execute(&sql).await.unwrap();
        }
        let page = d.select_page(&scope, "actor", "", "id", Some(10), 0);
        let rows = rel.execute(&page).await.unwrap().remove(0).rows;
        assert_eq!(
            rows,
            vec![vec![
                Some("1".into()),
                Some("O'Brien".into()),
                Some("hi".into())
            ]]
        );
        let count = rel
            .execute(&d.count(&scope, "actor", ""))
            .await
            .unwrap()
            .remove(0)
            .rows;
        assert_eq!(count, vec![vec![Some("1".into())]]);
        session.close().await.unwrap();
    });
}
