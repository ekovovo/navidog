use super::*;
use std::sync::Arc;
fn scope() -> Scope {
    Scope::new("test", "main", Some("public".into()))
}
#[test]
fn quoted_definitions_round_trip_and_change_by_identity() {
    let sql = "CREATE TABLE `t` (`id` int NOT NULL AUTO_INCREMENT, `a``b` enum('x,y','z') DEFAULT 'x,y', PRIMARY KEY (`id`), KEY `ix` (`a``b`)) ENGINE=InnoDB";
    let design = super::parse::create(sql, "t", Dialect::MySql).unwrap();
    assert_eq!(design.columns[1].name, "a`b");
    assert_eq!(design.constraints.len(), 2);
    assert!(
        design
            .create_sql(Dialect::MySql, &scope(), "t")
            .unwrap()
            .contains("enum('x,y','z') DEFAULT 'x,y'")
    );
    let before = TableSnapshot {
        design: design.clone(),
        ddl: sql.into(),
        triggers: Vec::new(),
    };
    let mut after = design;
    after.columns[1].name = "renamed".into();
    let plan = plan(Dialect::MySql, &scope(), Some(&before), &after).unwrap();
    assert_eq!(plan.len(), 1);
    assert!(plan[0].contains("CHANGE COLUMN `a``b` `renamed` enum('x,y','z') DEFAULT 'x,y'"));
}
#[test]
fn invalid_fragments_and_unsupported_attributes_are_rejected() {
    let mut draft = TableDesign::new();
    draft.name = "t".into();
    draft.columns[0].definition = "int); DROP TABLE x; --".into();
    assert!(plan(Dialect::MySql, &scope(), None, &draft).is_err());
    assert!(super::plan::index_name("CREATE TABLE x (id int)").is_err());
    assert!(super::parse::fragment("TEXT DEFAULT 'semi;hash#-- $$/*'").is_ok());
    assert!(super::parse::fragment("TEXT DEFAULT 1 -- hidden suffix").is_err());
}
#[test]
fn sqlite_backup_name_uses_the_date_then_a_suffix() {
    let name = super::plan::backup_name;
    assert_eq!(name("test", "20261024", &[]).unwrap(), "_test_old_20261024");
    let taken = ["_test_old_20261024".into()];
    assert_eq!(
        name("test", "20261024", &taken).unwrap(),
        "_test_old_20261024_1"
    );
    let taken = ["_test_old_20261024".into(), "_TEST_old_20261024_1".into()];
    assert_eq!(
        name("test", "20261024", &taken).unwrap(),
        "_test_old_20261024_2"
    );
}

#[test]
fn sqlite_rebuild_renames_the_old_table_and_leaves_it() {
    let sql = "CREATE TABLE \"test\" (\"id\" INTEGER NOT NULL, \"name\" TEXT, \"detail\" TEXT, PRIMARY KEY (\"id\"))";
    let design = super::parse::create(sql, "test", Dialect::Sqlite).unwrap();
    let before = TableSnapshot {
        design: design.clone(),
        ddl: sql.into(),
        triggers: vec!["CREATE TRIGGER audit_t AFTER INSERT ON test BEGIN SELECT 1; END".into()],
    };
    let mut added = design.clone();
    added.columns.push(DesignColumn::new(9, "add_col", "TEXT"));
    added.indexes.push("CREATE INDEX ix_t ON test(name)".into());
    let mut indexed = before.clone();
    indexed
        .design
        .indexes
        .push("CREATE INDEX ix_t ON test(name)".into());
    let statements = super::plan::plan_on(
        Dialect::Sqlite,
        &scope(),
        Some(&indexed),
        &added,
        &[],
        "20261024",
    )
    .unwrap();
    assert!(statements.iter().all(|sql| !sql.contains("DROP TABLE")));
    assert_eq!(
        statements[0],
        "ALTER TABLE \"main\".\"test\" RENAME TO \"_test_old_20261024\""
    );
    assert!(statements[1].starts_with("CREATE TABLE \"main\".\"test\""));
    assert!(statements[1].contains("\"add_col\" TEXT"));
    assert_eq!(
        statements[2],
        "INSERT INTO \"main\".\"test\" (\"id\", \"name\", \"detail\") SELECT \"id\", \"name\", \"detail\" FROM \"main\".\"_test_old_20261024\""
    );
    assert_eq!(statements[3], "DROP INDEX \"main\".\"ix_t\"");
    assert_eq!(statements[4], "DROP TRIGGER \"main\".\"audit_t\"");
    assert_eq!(statements[5], "CREATE INDEX ix_t ON test(name)");
    assert!(statements[6].starts_with("CREATE TRIGGER audit_t"));
    let mut dropped = design;
    dropped.columns.retain(|c| c.name != "detail");
    let statements = super::plan::plan_on(
        Dialect::Sqlite,
        &scope(),
        Some(&before),
        &dropped,
        &["_test_old_20261024".into()],
        "20261024",
    )
    .unwrap();
    assert_eq!(
        statements[0],
        "ALTER TABLE \"main\".\"test\" RENAME TO \"_test_old_20261024_1\""
    );
    assert!(!statements[1].contains("detail"));
    assert_eq!(
        statements[2],
        "INSERT INTO \"main\".\"test\" (\"id\", \"name\") SELECT \"id\", \"name\" FROM \"main\".\"_test_old_20261024_1\""
    );
    assert!(statements.iter().all(|sql| !sql.contains("DROP TABLE")));
}

#[test]
fn sqlite_rebuild_preserves_rows_indexes_triggers_and_rolls_back() {
    crate::runtime::runtime().block_on(async {
        let mut session=crate::connect(Arc::new(crate::sqlite::SqliteDriver::default()),crate::ConnectParams {file:Some(":memory:".into()),create_new:true,..Default::default()}).await.unwrap();
        let r=session.relational().unwrap(); let scope=Scope::new("test","main",None);
        r.execute("PRAGMA foreign_keys=ON; CREATE TABLE p(id INTEGER PRIMARY KEY); INSERT INTO p VALUES(1); CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, value TEXT NOT NULL, parent INTEGER REFERENCES p(id), CHECK(length(value)>0)); CREATE INDEX ix_t ON t(value); CREATE TABLE audit(v TEXT); CREATE TRIGGER audit_t AFTER INSERT ON t BEGIN INSERT INTO audit VALUES(new.value); END; INSERT INTO t(value,parent) VALUES('before',1)").await.unwrap();
        let old=load(r,&scope,"t").await.unwrap();assert_eq!(old.triggers.len(),1);assert_eq!(old.design.indexes.len(),1);
        let mut draft=old.design.clone();draft.columns.push(DesignColumn::new(99,"extra","TEXT DEFAULT 'new'"));
        let saved=apply(r,&scope,Some(&old),&draft).await.unwrap();
        let rows=r.execute("SELECT id,value,parent,extra FROM t").await.unwrap();assert_eq!(rows[0].rows[0],vec![Some("1".into()),Some("before".into()),Some("1".into()),Some("new".into())]);
        assert_eq!(saved.triggers.len(),1);assert_eq!(saved.design.indexes.len(),1);
        let tables=r.execute("SELECT name FROM sqlite_schema WHERE type='table'").await.unwrap();
        assert_eq!(tables[0].rows.iter().filter(|row| row[0].as_deref().is_some_and(|n| n.starts_with("_t_old_"))).count(),1);
        r.execute("INSERT INTO t(value,parent) VALUES('after',1)").await.unwrap();
        assert_eq!(r.execute("SELECT count(*) FROM audit").await.unwrap()[0].rows[0][0].as_deref(),Some("2"));
        let mut bad=saved.design.clone();bad.columns[1].definition="TEXT NOT NULL CHECK(value='invalid')".into();
        assert!(apply(r,&scope,Some(&saved),&bad).await.is_err());
        assert_eq!(load(r,&scope,"t").await.unwrap(),saved);
        assert_eq!(r.execute("PRAGMA foreign_keys").await.unwrap()[0].rows[0][0].as_deref(),Some("1"));
        let mut drop_dependency=saved.design.clone();
        drop_dependency.columns.retain(|c|c.name!="value");
        assert!(apply(r,&scope,Some(&saved),&drop_dependency).await.unwrap_err().message.contains("dependent schema"));
        assert_eq!(load(r,&scope,"t").await.unwrap(),saved);
        let mut invalid_foreign_key=saved.design.clone();
        invalid_foreign_key.constraints.push("FOREIGN KEY(id) REFERENCES p(id)".into());
        assert!(apply(r,&scope,Some(&saved),&invalid_foreign_key).await.unwrap_err().message.contains("Foreign key check"));
        assert_eq!(load(r,&scope,"t").await.unwrap(),saved);
        r.execute("ALTER TABLE t ADD COLUMN external TEXT").await.unwrap();
        assert!(apply(r,&scope,Some(&saved),&draft).await.unwrap_err().message.contains("outside"));
        session.close().await.unwrap();
    });
}

#[test]
fn sqlite_rebuild_keeps_inbound_references_views_sequence_and_rowid() {
    crate::runtime::runtime().block_on(async {
        let mut session=crate::connect(Arc::new(crate::sqlite::SqliteDriver::default()),crate::ConnectParams {file:Some(":memory:".into()),create_new:true,..Default::default()}).await.unwrap();
        let r=session.relational().unwrap(); let scope=Scope::new("test","main",None);
        r.execute("PRAGMA foreign_keys=ON; CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT,v TEXT); INSERT INTO t VALUES(1,'one'); UPDATE sqlite_sequence SET seq=100 WHERE name='t'; CREATE TABLE child(id INTEGER REFERENCES t(id)); INSERT INTO child VALUES(1); CREATE VIEW visible AS SELECT v FROM t").await.unwrap();
        let old=load(r,&scope,"t").await.unwrap();let mut draft=old.design.clone();draft.columns.push(DesignColumn::new(9,"new_field","TEXT"));
        let saved=apply(r,&scope,Some(&old),&draft).await.unwrap();
        assert_eq!(r.execute("SELECT * FROM visible").await.unwrap()[0].rows[0][0].as_deref(),Some("one"));
        assert_eq!(r.execute("SELECT id FROM child").await.unwrap()[0].rows[0][0].as_deref(),Some("1"));
        r.execute("INSERT INTO t(v) VALUES('two')").await.unwrap();
        assert_eq!(r.execute("SELECT max(id) FROM t").await.unwrap()[0].rows[0][0].as_deref(),Some("101"));
        let mut renamed=saved.design.clone();renamed.columns[1].name="renamed".into();
        apply(r,&scope,Some(&saved),&renamed).await.unwrap();
        assert_eq!(r.execute("SELECT * FROM visible").await.unwrap()[0].rows[0][0].as_deref(),Some("one"));
        r.execute("CREATE TABLE sparse(v TEXT); INSERT INTO sparse(rowid,v) VALUES(40,'forty')").await.unwrap();
        let old=load(r,&scope,"sparse").await.unwrap();let mut draft=old.design.clone();draft.columns.push(DesignColumn::new(9,"extra","TEXT"));
        apply(r,&scope,Some(&old),&draft).await.unwrap();
        assert_eq!(r.execute("SELECT rowid FROM sparse").await.unwrap()[0].rows[0][0].as_deref(),Some("40"));
        r.execute("CREATE TABLE generated_table(v INTEGER, doubled INTEGER GENERATED ALWAYS AS(v*2) STORED); INSERT INTO generated_table(v) VALUES(4)").await.unwrap();
        let original=load(r,&scope,"generated_table").await.unwrap();
        let mut draft=original.design.clone();draft.indexes.push("CREATE INDEX generated_idx ON generated_table(doubled)".into());
        let statements=plan(Dialect::Sqlite,&scope,Some(&original),&draft).unwrap();assert_eq!(statements.len(),1);
        apply(r,&scope,Some(&original),&draft).await.unwrap();
        assert_eq!(r.execute("SELECT doubled FROM generated_table").await.unwrap()[0].rows[0][0].as_deref(),Some("8"));
        session.close().await.unwrap();
    });
}

#[test]
fn live_table_design_create_alter_rename_and_constraints() {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    crate::runtime::runtime().block_on(async {
        for (driver, port, db, user) in [
            (
                Arc::new(crate::mysql::MySqlDriver::mysql()) as Arc<dyn crate::Driver>,
                13306,
                "sakila",
                "root",
            ),
            (
                Arc::new(crate::mysql::MySqlDriver::mariadb()) as Arc<dyn crate::Driver>,
                13307,
                "sakila",
                "root",
            ),
            (
                Arc::new(crate::postgres::PostgresDriver::default()) as Arc<dyn crate::Driver>,
                15432,
                "pagila",
                "postgres",
            ),
        ] {
            let mut session = crate::connect(
                driver,
                crate::ConnectParams {
                    host: "127.0.0.1".into(),
                    port,
                    database: Some(db.into()),
                    user: user.into(),
                    password: Some("navidog".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let r = session.relational().unwrap();
            let d = r.dialect();
            let scope = Scope::new("test", db, Some("public".into()));
            let name = format!("navidog_design_{}", std::process::id());
            let table = d.qualified(&scope, &name);
            r.execute(&format!("DROP TABLE IF EXISTS {table}"))
                .await
                .unwrap();
            let mut draft = TableDesign::new();
            draft.name = name.clone();
            draft.columns.push(DesignColumn::new(
                2,
                "value",
                "VARCHAR(80) NOT NULL DEFAULT 'before'",
            ));
            let mut check = DesignEntry::new(EntryKind::Check);
            check.name = "positive_id".into();
            check.expression = "id > 0".into();
            draft
                .constraints
                .push(check.sql(d, &scope, &name, false).unwrap());
            draft
                .columns
                .push(DesignColumn::new(3, "parent_id", "INTEGER"));
            let mut fk = DesignEntry::new(EntryKind::ForeignKey);
            fk.name = format!("fk_design_{}_{port}", std::process::id());
            fk.fields = d.quote_ident("parent_id");
            fk.reference_table = name.clone();
            fk.reference_fields = d.quote_ident("id");
            fk.on_delete = "SET NULL".into();
            draft
                .constraints
                .push(fk.sql(d, &scope, &name, false).unwrap());
            let mut index = DesignEntry::new(EntryKind::Index);
            index.name = format!("ix_design_{}_{port}", std::process::id());
            index.fields = d.quote_ident("value");
            draft
                .indexes
                .push(index.sql(d, &scope, &name, true).unwrap());
            let saved = apply(r, &scope, None, &draft).await.unwrap();
            r.execute(&format!("INSERT INTO {table} (id) VALUES(1)"))
                .await
                .unwrap();
            if d == Dialect::MySql {
                let name = format!("ix_standalone_{}", std::process::id());
                r.execute(&format!(
                    "CREATE INDEX {} ON {table} (value)",
                    d.quote_ident(&name)
                ))
                .await
                .unwrap();
                let mut before = load(r, &scope, &draft.name).await.unwrap();
                // MySQL's loader canonicalizes indexes inline; exercise the supported Advanced representation.
                before.design.constraints.retain(|s| !s.contains(&name));
                before.design.indexes.push(format!(
                    "CREATE INDEX {} ON {table} (value)",
                    d.quote_ident(&name)
                ));
                let mut after = before.design.clone();
                after.indexes.clear();
                for sql in plan(d, &scope, Some(&before), &after).unwrap() {
                    r.execute(&sql).await.unwrap();
                }
                assert!(
                    !load(r, &scope, &draft.name)
                        .await
                        .unwrap()
                        .ddl
                        .contains(&name)
                );
            }
            let mut changed = saved.design.clone();
            changed.columns[1].name = "renamed".into();
            changed.columns[1].definition = edit_column(
                &changed.columns[1].definition,
                ColumnProperty::Length,
                "100",
                d,
            )
            .unwrap();
            changed.columns[1].definition = edit_column(
                &changed.columns[1].definition,
                ColumnProperty::Default,
                "'after'",
                d,
            )
            .unwrap();
            changed
                .columns
                .push(DesignColumn::new(99, "extra", "INTEGER DEFAULT 7"));
            let changed = apply(r, &scope, Some(&saved), &changed).await.unwrap();
            let result = r
                .execute(&format!("SELECT renamed,extra FROM {table}"))
                .await
                .unwrap();
            assert_eq!(
                result[0].rows[0],
                vec![Some("before".into()), Some("7".into())],
                "{port}"
            );
            let mut bad = changed.design.clone();
            bad.columns[0].definition = "INTEGER NOT NULL DEFAULT 'invalid_number'".into();
            assert!(apply(r, &scope, Some(&changed), &bad).await.is_err());
            assert_eq!(
                r.execute(&format!("SELECT renamed FROM {table}"))
                    .await
                    .unwrap()[0]
                    .rows[0][0]
                    .as_deref(),
                Some("before")
            );
            r.execute(&format!("DROP TABLE {table}")).await.unwrap();
            session.close().await.unwrap();
        }
    });
}

#[test]
fn foreign_key_requires_matching_nonempty_field_lists() {
    let scope = Scope::new("test", "main", None);
    let mut e = DesignEntry::new(EntryKind::ForeignKey);
    e.fields = "\"local,a\", b".into();
    e.reference_table = "parent".into();
    e.reference_fields = "a".into();
    assert!(e.sql(Dialect::Sqlite, &scope, "child", false).is_err());
    e.reference_fields = "a, b".into();
    assert!(e.sql(Dialect::Sqlite, &scope, "child", false).is_ok());
    e.reference_table.clear();
    assert!(e.sql(Dialect::Sqlite, &scope, "child", false).is_err());
}

#[test]
fn sqlite_strict_rebuild_preserves_data_and_rejects_rowid_conversion() {
    crate::runtime::runtime().block_on(async {
        let mut session = crate::connect(
            Arc::new(crate::sqlite::SqliteDriver::default()),
            crate::ConnectParams {
                file: Some(":memory:".into()),
                create_new: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let r = session.relational().unwrap();
        let scope = Scope::new("test", "main", None);
        r.execute(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO t VALUES(8,'eight')",
        )
        .await
        .unwrap();
        let old = load(r, &scope, "t").await.unwrap();
        let mut next = old.design.clone();
        next.options = edit_table_option(&next.options, "strict", "true", Dialect::Sqlite).unwrap();
        let strict = apply(r, &scope, Some(&old), &next).await.unwrap();
        assert_eq!(
            table_option(&strict.design.options, "strict", Dialect::Sqlite).unwrap(),
            "true"
        );
        assert_eq!(
            r.execute("SELECT id,v FROM t").await.unwrap()[0].rows[0][0].as_deref(),
            Some("8")
        );
        let mut next = strict.design.clone();
        next.options =
            edit_table_option(&next.options, "without_rowid", "true", Dialect::Sqlite).unwrap();
        assert!(
            plan(Dialect::Sqlite, &scope, Some(&strict), &next)
                .unwrap_err()
                .message
                .contains("row identity")
        );
        r.execute("CREATE TABLE bad(v INTEGER); INSERT INTO bad VALUES('text')")
            .await
            .unwrap();
        let bad = load(r, &scope, "bad").await.unwrap();
        let mut next = bad.design.clone();
        next.options = "STRICT".into();
        assert!(apply(r, &scope, Some(&bad), &next).await.is_err());
        assert_eq!(load(r, &scope, "bad").await.unwrap(), bad);
        let mut fresh = TableDesign::new();
        fresh.name = "fresh".into();
        fresh.options = "WITHOUT ROWID, STRICT".into();
        assert!(apply(r, &scope, None, &fresh).await.is_ok());
    });
}

#[test]
fn live_table_options_and_dialect_attributes() {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    crate::runtime::runtime().block_on(async {
        for (driver, port, db, user) in [
            (
                Arc::new(crate::mysql::MySqlDriver::mysql()) as Arc<dyn crate::Driver>,
                13306,
                "sakila",
                "root",
            ),
            (
                Arc::new(crate::mysql::MySqlDriver::mariadb()) as Arc<dyn crate::Driver>,
                13307,
                "sakila",
                "root",
            ),
            (
                Arc::new(crate::postgres::PostgresDriver::default()) as Arc<dyn crate::Driver>,
                15432,
                "pagila",
                "postgres",
            ),
        ] {
            let mut session = crate::connect(
                driver,
                crate::ConnectParams {
                    host: "127.0.0.1".into(),
                    port,
                    database: Some(db.into()),
                    user: user.into(),
                    password: Some("navidog".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let r = session.relational().unwrap();
            let d = r.dialect();
            let scope = Scope::new("test", db, Some("public".into()));
            let name = format!("navidog_options_{}", std::process::id());
            let table = d.qualified(&scope, &name);
            let parent_name = format!("{name}_parent");
            let parent = d.qualified(&scope, &parent_name);
            let role = format!("navidog_options_role_{}", std::process::id());
            r.execute(&format!("DROP TABLE IF EXISTS {table}"))
                .await
                .unwrap();
            let mut draft = TableDesign::new();
            draft.name = name;
            if d == Dialect::MySql {
                draft.columns[0].definition = "INTEGER NOT NULL AUTO_INCREMENT".into();
                for (key, value) in [
                    ("engine", "InnoDB"),
                    ("charset", "utf8mb4"),
                    ("collation", "utf8mb4_bin"),
                    ("row_format", "DYNAMIC"),
                    ("auto_increment", "20"),
                    ("table_comment", "O'Brien"),
                ] {
                    draft.options = edit_table_option(&draft.options, key, value, d).unwrap();
                }
                draft.columns.push(DesignColumn::new(
                    2,
                    "stamp",
                    "TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP",
                ));
            } else {
                r.execute(&format!(
                    "DROP TABLE IF EXISTS {parent}; CREATE TABLE {parent}(id INTEGER)"
                ))
                .await
                .unwrap();
                draft.columns[0].definition =
                    "INTEGER GENERATED ALWAYS AS IDENTITY NOT NULL".into();
                r.execute(&format!("CREATE ROLE {}", d.quote_ident(&role)))
                    .await
                    .unwrap();
                draft.unlogged = true;
                draft.owner = user.into();
                for (key, value) in [
                    ("fillfactor", "70"),
                    ("autovacuum_enabled", "false"),
                    ("inherits", &*parent),
                ] {
                    draft.options = edit_table_option(&draft.options, key, value, d).unwrap();
                }
            }
            apply(r, &scope, None, &draft).await.unwrap();
            r.execute(&format!("INSERT INTO {table} DEFAULT VALUES").replace(
                " DEFAULT VALUES",
                if d == Dialect::MySql {
                    " () VALUES ()"
                } else {
                    " DEFAULT VALUES"
                },
            ))
            .await
            .unwrap();
            let old = load(r, &scope, &draft.name).await.unwrap();
            let mut next = old.design.clone();
            if d == Dialect::MySql {
                assert_eq!(
                    table_option(&next.options, "table_comment", d).unwrap(),
                    "O'Brien"
                );
                next.options =
                    edit_table_option(&next.options, "table_comment", "Changed", d).unwrap();
                next.options =
                    edit_table_option(&next.options, "stats_persistent", "0", d).unwrap();
                next.columns[0].definition = edit_column(
                    &next.columns[0].definition,
                    ColumnProperty::Zerofill,
                    "true",
                    d,
                )
                .unwrap();
                next.columns[1].definition =
                    edit_column(&next.columns[1].definition, ColumnProperty::OnUpdate, "", d)
                        .unwrap();
            } else {
                assert!(old.design.unlogged);
                assert_eq!(old.design.owner, "postgres");
                assert!(
                    !table_option(&next.options, "inherits", d)
                        .unwrap()
                        .is_empty()
                );
                next.unlogged = false;
                next.owner = role.clone();
                next.options = edit_table_option(&next.options, "fillfactor", "90", d).unwrap();
                next.columns[0].definition = edit_column(
                    &next.columns[0].definition,
                    ColumnProperty::Identity,
                    "BY DEFAULT",
                    d,
                )
                .unwrap();
            }
            let saved = apply(r, &scope, Some(&old), &next).await.unwrap();
            assert_eq!(
                r.execute(&format!("SELECT count(*) FROM {table}"))
                    .await
                    .unwrap()[0]
                    .rows[0][0]
                    .as_deref(),
                Some("1")
            );
            if d == Dialect::Postgres {
                assert!(!saved.design.unlogged);
                assert_eq!(saved.design.owner, role);
                assert_eq!(
                    column_properties(&saved.design.columns[0].definition).identity,
                    "BY DEFAULT"
                );
                assert_eq!(
                    table_option(&saved.design.options, "fillfactor", d).unwrap(),
                    "90"
                );
                let mut reset = saved.design.clone();
                reset.options = edit_table_option(&reset.options, "fillfactor", "", d).unwrap();
                reset.options = edit_table_option(&reset.options, "inherits", "", d).unwrap();
                reset.options =
                    edit_table_option(&reset.options, "tablespace", "pg_default", d).unwrap();
                let reset = apply(r, &scope, Some(&saved), &reset).await.unwrap();
                assert!(
                    table_option(&reset.design.options, "fillfactor", d)
                        .unwrap()
                        .is_empty()
                );
                let mut bad = reset.design.clone();
                bad.options = "WITH (fillfactor=1)".into();
                assert!(apply(r, &scope, Some(&reset), &bad).await.is_err());
                assert_eq!(load(r, &scope, &draft.name).await.unwrap(), reset);
                r.execute(&format!("ALTER TABLE {table} SET (fillfactor=85)"))
                    .await
                    .unwrap();
                assert!(
                    apply(r, &scope, Some(&reset), &reset.design)
                        .await
                        .unwrap_err()
                        .message
                        .contains("outside")
                );
            } else {
                assert_eq!(
                    table_option(&saved.design.options, "table_comment", d).unwrap(),
                    "Changed"
                );
            }
            r.execute(&format!("DROP TABLE {table}")).await.unwrap();
            if d == Dialect::Postgres {
                r.execute(&format!(
                    "DROP TABLE {parent}; DROP ROLE {}",
                    d.quote_ident(&role)
                ))
                .await
                .unwrap();
            }
            session.close().await.unwrap();
        }
    });
}

#[test]
fn live_postgres_comments_round_trip_rename_rollback_and_concurrency() {
    if std::env::var_os("NAVIDOG_LIVE").is_none() {
        return;
    }
    crate::runtime::runtime().block_on(async {
        let mut session = crate::connect(
            Arc::new(crate::postgres::PostgresDriver::default()),
            crate::ConnectParams {
                host: "127.0.0.1".into(),
                port: 15432,
                database: Some("pagila".into()),
                user: "postgres".into(),
                password: Some("navidog".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let r = session.relational().unwrap();
        let d = Dialect::Postgres;
        let scope = Scope::new("test", "pagila", Some("public".into()));
        let mut draft = TableDesign::new();
        draft.name = format!("navidog_comments_{}_表", std::process::id());
        draft.columns[0].name = "旧.字段\"".into();
        draft.constraints = vec![format!(
            "PRIMARY KEY ({})",
            d.quote_ident(&draft.columns[0].name)
        )];
        draft.comment = "旧表".into();
        draft.columns[0].comment = "旧字段".into();
        let table = d.qualified(&scope, &draft.name);
        r.execute(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .unwrap();
        let old = apply(r, &scope, None, &draft).await.unwrap();
        assert_eq!(old.design.comment, "旧表");
        assert_eq!(old.design.columns[0].comment, "旧字段");
        assert!(old.design.indexes.is_empty());
        let mut legacy = old.clone();
        legacy.design.comment.clear();
        legacy.design.columns[0].comment.clear();
        legacy
            .design
            .indexes
            .push(format!("COMMENT ON TABLE {table} IS '旧表'"));
        legacy.design.indexes.push(format!(
            "COMMENT ON COLUMN {table}.{} IS '旧字段'",
            d.quote_ident(&old.design.columns[0].name)
        ));
        assert_eq!(
            apply(r, &scope, Some(&legacy), &legacy.design)
                .await
                .unwrap(),
            old
        );
        r.execute(&format!("INSERT INTO {table} VALUES(1)"))
            .await
            .unwrap();
        let mut next = old.design.clone();
        next.columns[0].name = "新.字段\"".into();
        let text = "中文 O'Brien \\ server; -- text\nsecond line";
        next.columns[0].comment = text.into();
        next.comment = format!("表 {text}");
        r.execute("SET standard_conforming_strings=off")
            .await
            .unwrap();
        let saved = apply(r, &scope, Some(&old), &next).await.unwrap();
        assert_eq!(saved.design.columns[0].name, "新.字段\"");
        assert_eq!(saved.design.columns[0].comment, text);
        assert_eq!(saved.design.comment, format!("表 {text}"));
        assert!(
            plan(d, &scope, Some(&saved), &saved.design)
                .unwrap()
                .is_empty()
        );
        let mut failing = saved.design.clone();
        failing.comment = "must roll back".into();
        failing.columns[0].comment = "also roll back".into();
        failing
            .columns
            .push(DesignColumn::new(9, "temporary_column", "INTEGER"));
        failing.owner = format!("navidog_missing_owner_{}", std::process::id());
        let failure_plan = plan(d, &scope, Some(&saved), &failing).unwrap();
        assert!(failure_plan[failure_plan.len() - 2].starts_with("COMMENT ON COLUMN"));
        assert!(failure_plan.last().unwrap().contains("OWNER TO"));
        assert!(apply(r, &scope, Some(&saved), &failing).await.is_err());
        assert_eq!(load(r, &scope, &draft.name).await.unwrap(), saved);
        assert_eq!(failing.comment, "must roll back");
        let mut clear = saved.design.clone();
        clear.comment.clear();
        clear.columns[0].comment.clear();
        let clear_sql = plan(d, &scope, Some(&saved), &clear).unwrap();
        assert_eq!(clear_sql.len(), 2);
        assert!(clear_sql.iter().all(|s| s.ends_with("IS NULL")));
        let cleared = apply(r, &scope, Some(&saved), &clear).await.unwrap();
        assert!(cleared.design.comment.is_empty());
        assert!(cleared.design.columns[0].comment.is_empty());
        r.execute(&format!(
            "COMMENT ON COLUMN {table}.{} IS 'external'",
            d.quote_ident(&cleared.design.columns[0].name)
        ))
        .await
        .unwrap();
        assert!(
            apply(r, &scope, Some(&cleared), &clear)
                .await
                .unwrap_err()
                .message
                .contains("outside")
        );
        let external = load(r, &scope, &draft.name).await.unwrap();
        r.execute(&format!("COMMENT ON TABLE {table} IS 'external table'"))
            .await
            .unwrap();
        assert!(
            apply(r, &scope, Some(&external), &external.design)
                .await
                .unwrap_err()
                .message
                .contains("outside")
        );
        assert_eq!(
            r.execute(&format!("SELECT count(*) FROM {table}"))
                .await
                .unwrap()[0]
                .rows[0][0]
                .as_deref(),
            Some("1")
        );
        r.execute(&format!("DROP TABLE {table}")).await.unwrap();
        session.close().await.unwrap();
    });
}

#[test]
fn mysql_standalone_index_removal_names_its_table() {
    let mut design = TableDesign::new();
    design.name = "t".into();
    design.columns[0].original = Some("id".into());
    design
        .indexes
        .push("CREATE INDEX `ix` ON `main`.`t` (`id`)".into());
    let old = TableSnapshot {
        design: design.clone(),
        ddl: String::new(),
        triggers: vec![],
    };
    design.indexes.clear();
    assert_eq!(
        plan(Dialect::MySql, &scope(), Some(&old), &design).unwrap(),
        vec!["ALTER TABLE `main`.`t` DROP INDEX `ix`"]
    );
}
