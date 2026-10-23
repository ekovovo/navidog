use crate::panel::TableDesigner;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, WindowOptions,
};
use navidog_core::object::Scope;

fn isolate_home(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let home = std::env::temp_dir().join(format!("navidog-designer-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: every test in this process holds LOCK while accessing the config directory.
    unsafe {
        std::env::set_var("NAVIDOG_HOME", &home);
    }
    (guard, home)
}

struct Host(Entity<TableDesigner>);
impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div().size_full().child(self.0.clone())
    }
}

#[gpui_kit::test]
fn edits_fields_previews_sql_and_restores_drafts(cx: &mut TestAppContext) {
    let (_guard, _) = isolate_home("draft");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let window = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let p = TableDesigner::new(Scope::new("test", "main", None), window, cx);
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (window.0, panel.unwrap())
    });
    assert_ne!(rust_i18n::t!("designer.save").to_string(), "designer.save");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("designer-table-name", cx);
        window.input("people", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-add", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-field-name", cx);
        // Input uses Command on macOS and Control on Linux/Windows.
        window.press("secondary-a", cx);
        window.input("display_name", cx);
        window.click(("designer-flag", 12usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-field-default", cx);
        window.input("'guest'", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        let row = w.find(("row", 1usize)).bounds();
        let col = w.find(("col-header", 1usize)).bounds();
        w.click_at(
            ("row", 1usize),
            gpui_kit::point(col.center().x - row.left(), row.size.height / 2.),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-field-type", cx);
        w.press("secondary-a", cx);
        w.input("VARCHAR", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        let row = w.find(("row", 1usize)).bounds();
        let col = w.find(("col-header", 2usize)).bounds();
        w.click_at(
            ("row", 1usize),
            gpui_kit::point(col.center().x - row.left(), row.size.height / 2.),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-field-length", cx);
        w.input("80; invalid", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-save", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let p = panel.read(cx);
        assert!(
            p.preview.is_none(),
            "length={:?} definition={:?} errors={:?} page={:?}",
            p.properties[1].read(cx).value(),
            p.draft.columns[1].definition,
            p.invalid_inputs,
            p.page == super::Page::Preview
        );
        assert!(!panel.read(cx).invalid_inputs.is_empty());
    });
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-field-length", cx);
        w.press("secondary-a", cx);
        w.input("80", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (page, name) in [
        ("indexes", "idx_people"),
        ("foreign_keys", "fk_people"),
        ("checks", "chk_people"),
    ] {
        cx.update_window(handle, |_, w, cx| {
            w.click(
                gpui_kit::SharedString::from(format!("designer-page-{page}")),
                cx,
            )
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, w, cx| w.click("designer-add", cx))
            .unwrap();
        cx.run_until_parked();
        if page == "indexes" {
            cx.update_window(handle, |_, w, cx| w.click("designer-entry-name", cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, w, cx| w.click("designer-choices-12", cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, w, cx| {
                let input = panel.read(cx).entry_inputs[0].read(cx);
                assert!(
                    gpui_kit::Focusable::focus_handle(input, cx).is_focused(w),
                    "disabled method picker must not open its menu or steal input focus"
                );
            })
            .unwrap();
        }
        if page == "foreign_keys" {
            cx.update_window(handle, |_, w, cx| {
                w.render_frame(cx);
                assert!(w.try_find("designer-entry-reference_schema").is_none());
                assert_eq!(
                    panel.read(cx).references.error.as_deref(),
                    Some(rust_i18n::t!("designer.disconnected").as_ref())
                );
            })
            .unwrap();
        }
        cx.update_window(handle, |_, w, cx| {
            w.click("designer-entry-name", cx);
            w.press("secondary-a", cx);
            w.input(name, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.draft.indexes.len(), 1);
        assert!(p.draft.indexes[0].contains("idx_people"));
        assert!(p.draft.constraints.iter().any(|s| s.contains("fk_people")));
        assert!(p.draft.constraints.iter().any(|s| s.contains("chk_people")));
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        use gpui_kit::component::dock::BasePanel as _;
        let dumped = panel.dump(cx);
        let gpui_kit::component::dock::PanelInfo::Panel(value) = dumped.info else {
            panic!("saved draft")
        };
        let saved: super::Saved = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(saved.draft.columns.len(), 2);
        assert_eq!(value["draft"]["name"], "people");
        assert_eq!(value["draft"]["columns"][1]["name"], "display_name");
        assert_eq!(
            value["draft"]["columns"][1]["definition"],
            "VARCHAR(80) NOT NULL DEFAULT 'guest'"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("designer-save").is_none());
        assert!(window.try_find("designer-add").is_some());
        assert_ne!(window.find("designer-execute").disabled(), Some(true));
        use gpui_kit::component::dock::{BasePanel as _, PanelInfo};
        let PanelInfo::Panel(value) = panel.read(cx).dump(cx).info else {
            panic!("draft")
        };
        let restored = TableDesigner::restore(serde_json::from_value(value).unwrap(), window, cx);
        assert_eq!(restored.read(cx).draft.columns[1].name, "display_name");
        assert_eq!(
            restored.read(cx).definition.read(cx).value(),
            "INTEGER NOT NULL"
        );
        assert!(restored.read(cx).dirty());
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-page-fields", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-field-default", cx);
        window.press("cmd-d", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let added = cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.draft.columns.len(), 3);
        p.selected.unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-field-name", cx);
        window.press("cmd-up", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(panel.read(cx).draft.columns[1].id, added));
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-delete", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(panel.read(cx).draft.columns.len(), 2));
}

#[gpui_kit::test]
fn saves_only_after_preview_and_keeps_failed_draft(cx: &mut TestAppContext) {
    let (_guard, home) = isolate_home("save");
    use navidog_core::{connection::Connection, crypto::Key, store::ConnectionStore};
    use navidog_db::{ConnectParams, Secrets, sessions::Sessions};
    use std::sync::Arc;
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_reduce_motion(true));
    let file = home.join(format!(
        "{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sql = |query: &str| {
        navidog_db::runtime::runtime().block_on(async {
            let mut s = navidog_db::connect(
                Arc::new(navidog_db::sqlite::SqliteDriver::default()),
                ConnectParams {
                    file: Some(file.clone()),
                    create_new: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let result = s.relational().unwrap().execute(query).await;
            s.close().await.unwrap();
            result.unwrap()
        })
    };
    sql("SELECT 1");
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut connection = Connection::new("sqlite");
            connection.name = "designer-test".into();
            connection.general.file = Some(file.to_string_lossy().into_owned());
            let id = connection.id.clone();
            store.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&mut TestAppContext) -> bool| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(cx) {
            assert!(std::time::Instant::now() < end);
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    wait(cx, &|cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let p = TableDesigner::new(Scope::new(id, "main", None), window, cx);
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (handle.0, panel.unwrap())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("designer-table-name", cx);
        window.input("people", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-page-options", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-option-strict", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(panel.read(cx).draft.options, "STRICT"));
    cx.update_window(handle, |_, w, cx| w.click("designer-page-fields", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-auto-increment", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        sql("SELECT name FROM sqlite_schema WHERE name='people'")[0]
            .rows
            .is_empty()
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-execute", cx);
    })
    .unwrap();
    cx.run_until_parked();
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).busy));
    cx.update(|cx| {
        assert!(panel.read(cx).baseline.is_some());
        assert!(!panel.read(cx).dirty());
    });
    sql("INSERT INTO people(id) VALUES(1)");
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-page-fields", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-add", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.click(("designer-flag", 12usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = panel.read(cx);
        let selected = panel.selected.unwrap();
        assert_eq!(
            panel
                .draft
                .columns
                .iter()
                .find(|c| c.id == selected)
                .unwrap()
                .definition,
            "TEXT NOT NULL"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("designer-save", cx);
    })
    .unwrap();
    wait(cx, &|cx| {
        cx.update(|cx| {
            let p = panel.read(cx);
            !p.busy && p.preview.is_some()
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("designer-save").is_none());
        window.click("designer-execute", cx);
    })
    .unwrap();
    cx.run_until_parked();
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).busy));
    cx.update(|cx| {
        let p = panel.read(cx);
        assert!(p.error.is_some());
        assert!(p.dirty());
        assert_eq!(p.draft.columns.len(), 2);
        assert_eq!(p.baseline.as_ref().unwrap().design.columns.len(), 1);
    });
    assert_eq!(
        sql("SELECT count(*) FROM people")[0].rows[0][0].as_deref(),
        Some("1")
    );
}

#[gpui_kit::test]
fn cascades_real_reference_metadata_and_preserves_drafts(cx: &mut TestAppContext) {
    use crate::panel::ChooseValue;
    use navidog_core::{connection::Connection, crypto::Key, store::ConnectionStore};
    use navidog_db::{ConnectParams, Secrets, sessions::Sessions};
    use std::sync::Arc;
    let (_guard, home) = isolate_home("references");
    cx.executor().allow_parking();
    let file = home.join(format!(
        "references-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    navidog_db::runtime::runtime().block_on(async {
        let mut session = navidog_db::connect(Arc::new(navidog_db::sqlite::SqliteDriver::default()), ConnectParams { file: Some(file.clone()), create_new: true, ..Default::default() }).await.unwrap();
        session.relational().unwrap().execute("CREATE TABLE parent (a INTEGER, b INTEGER, PRIMARY KEY(a,b)); CREATE TABLE other (other_id INTEGER PRIMARY KEY)").await.unwrap();
        session.close().await.unwrap();
    });
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut connection = Connection::new("sqlite");
            connection.name = "reference-test".into();
            connection.general.file = Some(file.to_string_lossy().into_owned());
            let id = connection.id.clone();
            store.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&mut TestAppContext) -> bool| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(cx) {
            assert!(std::time::Instant::now() < end);
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    wait(cx, &|cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            let p = TableDesigner::new(Scope::new(id, "main", None), w, cx);
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (handle.0, panel.unwrap())
    });
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        w.click("designer-table-name", cx);
        w.input("child", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-add", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-page-foreign_keys", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-add", cx))
        .unwrap();
    cx.run_until_parked();
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).references.loading));
    cx.update(|cx| {
        let p = panel.read(cx);
        assert!(p.references.error.is_none(), "{:?}", p.references.error);
        assert_eq!(p.references.tables, vec!["other", "parent"]);
        assert!(p.reference_values(5, cx).contains(&"child".into()));
    });
    // Dispatch the same semantic actions emitted by native menu items.
    for table in ["other", "parent"] {
        cx.update_window(handle, |_, w, cx| {
            w.render_frame(cx);
            let focus = panel.read(cx).focus.clone();
            w.focus(&focus, cx);
            w.dispatch_action(
                Box::new(ChooseValue {
                    target: 15,
                    value: table.into(),
                }),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    }
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).references.loading));
    cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.reference_values(6, cx), vec!["a", "b"]);
        assert_eq!(p.entry_inputs[6].read(cx).value(), "");
        assert!(!p.invalid_inputs.is_empty());
    });
    cx.update_window(handle, |_, w, cx| w.click("designer-save", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(panel.read(cx).preview.is_none()));
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        let focus = panel.read(cx).focus.clone();
        w.focus(&focus, cx);
        w.dispatch_action(
            Box::new(ChooseValue {
                target: 16,
                value: "a".into(),
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let p = panel.read(cx);
        assert!(p.invalid_inputs.is_empty());
        assert!(
            p.draft
                .constraints
                .iter()
                .any(|s| s.contains("REFERENCES \"parent\" (\"a\")"))
        );
    });
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-entry-fields", cx);
        w.press("secondary-a", cx);
        w.input("\"id\", \"field_2\"", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(!panel.read(cx).invalid_inputs.is_empty()));
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        let focus = panel.read(cx).focus.clone();
        w.focus(&focus, cx);
        w.dispatch_action(
            Box::new(ChooseValue {
                target: 16,
                value: "b".into(),
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let p = panel.read(cx);
        assert!(p.invalid_inputs.is_empty());
        assert!(
            p.draft
                .constraints
                .iter()
                .any(|s| s.contains("REFERENCES \"parent\" (\"a\", \"b\")"))
        );
    });
    let before = cx.update(|cx| panel.read(cx).draft.clone());
    cx.update_window(handle, |_, w, cx| w.click("designer-reference-refresh", cx))
        .unwrap();
    cx.run_until_parked();
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).references.loading));
    cx.update(|cx| assert_eq!(panel.read(cx).draft, before));
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-entry-reference_table", cx);
        w.press("secondary-a", cx);
        w.input("child", cx);
    })
    .unwrap();
    cx.run_until_parked();
    wait(cx, &|cx| cx.update(|cx| !panel.read(cx).references.loading));
    cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.reference_values(6, cx), vec!["id", "field_2"]);
        assert_eq!(p.entry_inputs[6].read(cx).value(), "");
    });
}

#[gpui_kit::test]
fn option_forms_use_saved_dialect_and_guard_invalid_previews(cx: &mut TestAppContext) {
    use navidog_core::{connection::Connection, crypto::Key, store::ConnectionStore};
    let (_guard, _home) = isolate_home("option-forms");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |store, _| store.set_key(Key::random()));
    });
    for (driver, key, value) in [
        ("mysql", "engine", "InnoDB"),
        ("postgres", "fillfactor", "80"),
    ] {
        let id = cx.update(|cx| {
            ConnectionStore::global(cx).update(cx, |store, cx| {
                let c = Connection::new(driver);
                let id = c.id.clone();
                store.upsert(c, &[], cx).unwrap();
                id
            })
        });
        // No live session: restored drafts must still use the saved connection's dialect.
        let (handle, panel) = cx.update(|cx| {
            let mut panel = None;
            let handle = gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                let p = TableDesigner::new(Scope::new(id, "test", Some("public".into())), w, cx);
                panel = Some(p.clone());
                cx.new(|_| Host(p))
            })
            .unwrap();
            (handle.0, panel.unwrap())
        });
        cx.update_window(handle, |_, w, cx| {
            w.render_frame(cx);
            w.click("designer-table-name", cx);
            w.input("option_test", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, w, cx| w.click("designer-page-options", cx))
            .unwrap();
        cx.run_until_parked();
        let input = gpui_kit::SharedString::from(format!("designer-option-{key}"));
        cx.update_window(handle, |_, w, cx| {
            w.click(input.clone(), cx);
            w.input(if driver == "postgres" { "101" } else { value }, cx);
        })
        .unwrap();
        cx.run_until_parked();
        if driver == "postgres" {
            cx.update_window(handle, |_, w, cx| w.click("designer-save", cx))
                .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                assert!(panel.read(cx).preview.is_none());
                assert!(!panel.read(cx).invalid_inputs.is_empty());
            });
            cx.update_window(handle, |_, w, cx| {
                w.click(input.clone(), cx);
                w.press("secondary-a", cx);
                w.input(value, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, w, cx| w.click("designer-option-unlogged", cx))
                .unwrap();
            cx.run_until_parked();
        }
        cx.update_window(handle, |_, w, cx| w.click("designer-save", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let p = panel.read(cx);
            assert!(p.invalid_inputs.is_empty());
            let sql = &p.preview.as_ref().unwrap()[0];
            assert!(sql.contains(if driver == "mysql" {
                "ENGINE=InnoDB"
            } else {
                "CREATE UNLOGGED TABLE"
            }));
            if driver == "postgres" {
                assert!(sql.contains("fillfactor=80"));
            }
        });
        cx.update_window(handle, |_, w, _| w.remove_window())
            .unwrap();
    }
}

#[gpui_kit::test]
fn postgres_comments_edit_in_the_grid_preview_and_restore(cx: &mut TestAppContext) {
    use gpui_kit::component::dock::{BasePanel as _, PanelInfo};
    use navidog_core::{connection::Connection, crypto::Key, store::ConnectionStore};
    use navidog_db::table_design::TableDesign;
    let (_guard, _home) = isolate_home("pg-comments");
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let c = Connection::new("postgres");
            let id = c.id.clone();
            store.upsert(c, &[], cx).unwrap();
            id
        })
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let handle = gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            let mut draft = TableDesign::new();
            draft.name = "commented".into();
            draft.indexes = vec![
                "COMMENT ON TABLE public.commented IS '旧表注释'".into(),
                "COMMENT ON COLUMN public.commented.id IS '旧字段注释'".into(),
            ];
            let p = TableDesigner::restore(
                super::Saved {
                    object: None,
                    scope: Scope::new(id, "test", Some("public".into())),
                    draft,
                    baseline: None,
                    property_height: None,
                },
                w,
                cx,
            );
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (handle.0, panel.unwrap())
    });
    cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.draft.comment, "旧表注释");
        assert_eq!(p.draft.columns[0].comment, "旧字段注释");
        assert!(p.draft.indexes.is_empty());
    });
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        let row = w.find(("row", 0usize)).bounds();
        let col = w.find(("col-header", 7usize)).bounds();
        w.click_at(
            ("row", 0usize),
            gpui_kit::point(
                col.left() - row.left() + col.size.width / 4.,
                row.size.height / 2.,
            ),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-field-comment", cx);
        w.press("secondary-a", cx);
        w.input("字段 O'Brien \\ path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-page-options", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-option-table_comment", cx);
        w.press("secondary-a", cx);
        w.input("表 O'Brien", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| w.click("designer-save", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        let p = panel.read(cx);
        let preview = p.preview.as_ref().unwrap();
        assert!(
            preview
                .iter()
                .any(|s| s.contains("COMMENT ON COLUMN") && s.contains("字段 O''Brien \\\\ path")),
            "{preview:?}"
        );
        assert!(
            preview
                .iter()
                .any(|s| s.contains("COMMENT ON TABLE") && s.contains("表 O''Brien"))
        );
        let PanelInfo::Panel(value) = p.dump(cx).info else {
            panic!("saved draft")
        };
        let restored = TableDesigner::restore(serde_json::from_value(value).unwrap(), w, cx);
        assert_eq!(
            restored.read(cx).draft.columns[0].comment,
            "字段 O'Brien \\ path"
        );
        assert_eq!(restored.read(cx).draft.comment, "表 O'Brien");
        assert!(restored.read(cx).preview.is_none());
    })
    .unwrap();
    cx.update_window(handle, |_, w, cx| w.click("designer-page-fields", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.click("designer-field-default", cx);
        w.press("cmd-d", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let p = panel.read(cx);
        assert_eq!(p.draft.columns.len(), 2);
        assert_eq!(p.draft.columns[1].comment, "字段 O'Brien \\ path");
        assert!(p.draft.columns[1].original.is_none());
    });
}

#[gpui_kit::test]
fn reload_rebuilds_entry_forms_and_clears_validation(cx: &mut TestAppContext) {
    use super::{ChooseValue, Page};
    use gpui_kit::component::dock::BasePanel as _;
    use navidog_db::table_design::TableSnapshot;
    let (_guard, _) = isolate_home("review-reload");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let window = gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            let p = TableDesigner::new(Scope::new("test", "main", None), w, cx);
            assert!(!p.read(cx).dirty(), "a pristine new table has no edits");
            assert!(p.read(cx).closable(cx));
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (window.0, panel.unwrap())
    });
    cx.update_window(handle, |_, w, cx| {
        panel.update(cx, |p, cx| {
            p.draft.name = "t".into();
            p.draft.constraints = vec![
                "CONSTRAINT z CHECK(id > 0)".into(),
                "CONSTRAINT a CHECK(id < 100)".into(),
            ];
            p.sync_all(w, cx);
            p.switch_page(Page::Checks, w, cx);
            p.entry_selected = Some(1);
            p.active_cell = Some(0);
            p.invalid_inputs.insert(100, "stale error".into());
            let mut design = p.draft.clone();
            design.constraints.reverse();
            p.accept_snapshot(
                serde_json::from_value::<TableSnapshot>(
                    serde_json::json!({"design": design, "ddl": "", "triggers": []}),
                )
                .unwrap(),
                w,
                cx,
            );
            assert!(!p.dirty());
            assert!(p.active_cell.is_none());
            assert_eq!(p.entry_selected, Some(0));
            assert_eq!(p.entry_inputs[0].read(cx).value(), "a");
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        w.render_frame(cx);
        w.click("designer-entry-name", cx);
        w.press("secondary-a", cx);
        w.input("renamed_a", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, w, cx| {
        panel.update(cx, |p, cx| {
            assert!(p.draft.constraints[0].contains("renamed_a"));
            assert!(p.draft.constraints[0].contains("id < 100"));
            assert!(p.draft.constraints[1].contains("id > 0"));
            p.switch_page(Page::Indexes, w, cx);
            p.draft.constraints = vec!["PRIMARY KEY (id)".into()];
            p.sync_all(w, cx);
            // Existing unquoted identifiers and picker-produced identifiers denote the same field.
            p.choose_value(
                &ChooseValue {
                    target: 11,
                    value: "id".into(),
                },
                w,
                cx,
            );
            assert_eq!(p.entry_inputs[1].read(cx).value(), "");
            p.choose_value(
                &ChooseValue {
                    target: 11,
                    value: "id".into(),
                },
                w,
                cx,
            );
            assert_eq!(p.entry_inputs[1].read(cx).value(), "\"id\"");
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn reload_while_previewing_replaces_stale_sql(cx: &mut TestAppContext) {
    use super::Page;
    use navidog_db::table_design::{DesignColumn, TableDesign, TableSnapshot};
    let (_guard, _) = isolate_home("preview-reload");
    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_ui::tabs::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
    });
    let (handle, panel) = cx.update(|cx| {
        let mut panel = None;
        let window = gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            let p = TableDesigner::new(Scope::new("test", "main", None), w, cx);
            panel = Some(p.clone());
            cx.new(|_| Host(p))
        })
        .unwrap();
        (window.0, panel.unwrap())
    });
    cx.update_window(handle, |_, w, cx| {
        panel.update(cx, |p, cx| {
            let mut design = TableDesign::new();
            design.name = "people".into();
            let saved = serde_json::from_value::<TableSnapshot>(serde_json::json!({
                "design": design,
                "ddl": "",
                "triggers": []
            }))
            .unwrap();
            p.draft = design;
            p.baseline = Some(saved.clone());
            p.sync_all(w, cx);
            p.draft.columns.push(DesignColumn::new(9, "extra", "TEXT"));
            p.show_preview(w, cx);
            let sql = p.sql.read(cx).value().to_string();
            assert!(
                sql.contains("extra"),
                "preview should show the pending change, got {sql}"
            );
            assert!(p.page == Page::Preview);
            p.accept_snapshot(saved, w, cx);
            assert!(p.page == Page::Preview);
            let sql = p.sql.read(cx).value().to_string();
            assert_eq!(
                sql,
                format!("-- {}", rust_i18n::t!("designer.no_changes")),
                "discarding changes on the preview page must refresh the editor in place"
            );
        })
    })
    .unwrap();
}
