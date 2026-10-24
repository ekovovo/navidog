//! Database-change events reload only existing metadata and update an already open DDL tab.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render, Role,
    Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::Workspace;
use navidog_db::sessions::Sessions;
use navidog_db::{ConnectParams, Secrets, connect, runtime, sqlite::SqliteDriver};
use navidog_navigator::DdlPanel;
use navidog_navigator::catalog::{Catalog, Load};

struct Host {
    ddl: Entity<DdlPanel>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.ddl.clone())
    }
}

fn wait(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn execute(file: &Path, sql: &str, create_new: bool) {
    runtime::runtime().block_on(async {
        let mut session = connect(
            Arc::new(SqliteDriver::default()),
            ConnectParams {
                file: Some(file.into()),
                create_new,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        session.relational().unwrap().execute(sql).await.unwrap();
        session.close().await.unwrap();
    });
}

fn ready(catalog: &Catalog, scope: &Scope, view: &ObjectRef) -> bool {
    matches!(
        catalog.schemas(&scope.connection, &scope.database),
        Some(Load::Ready(_))
    ) && matches!(
        catalog.objects(scope, ObjectKind::Table),
        Some(Load::Ready(_))
    ) && matches!(
        catalog.objects(scope, ObjectKind::View),
        Some(Load::Ready(_))
    ) && matches!(catalog.columns(scope, "items"), Some(Load::Ready(_)))
        && matches!(catalog.table_children(scope, "items"), Some(Load::Ready(_)))
        && matches!(catalog.ddl(view), Some(Load::Ready(_)))
}

#[gpui_kit::test]
fn database_changes_reload_cached_metadata_and_open_ddl_without_touching_other_connections(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let home = tempfile::tempdir().unwrap();
    let first_file = home.path().join("first.db");
    let other_file = home.path().join("other.db");
    for file in [&first_file, &other_file] {
        execute(
            file,
            "CREATE TABLE items(id INTEGER PRIMARY KEY); CREATE VIEW shown AS SELECT id FROM items",
            true,
        );
    }
    // SAFETY: this integration-test binary has one test and owns its environment until exit.
    unsafe { std::env::set_var("NAVIDOG_HOME", home.path()) };
    let (first, other) = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_navigator::init(cx);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut add = |name: &str, file: &Path| {
                let mut connection = Connection::new("sqlite");
                connection.name = name.into();
                connection.general.file = Some(file.to_string_lossy().into_owned());
                let id = connection.id.clone();
                store.upsert(connection, &[], cx).unwrap();
                id
            };
            (add("first", &first_file), add("other", &other_file))
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.open(first.clone(), Secrets::default(), cx);
            sessions.open(other.clone(), Secrets::default(), cx);
        });
    });
    wait(cx, "both connections", |cx| {
        cx.update(|cx| {
            let sessions = Sessions::global(cx);
            sessions.read(cx).state(&first).is_open() && sessions.read(cx).state(&other).is_open()
        })
    });
    let scope = Scope::new(&first, "main", None);
    let other_scope = Scope::new(&other, "main", None);
    let view = |scope: &Scope| ObjectRef {
        scope: scope.clone(),
        kind: ObjectKind::View,
        name: "shown".into(),
    };
    let first_view = view(&scope);
    let other_view = view(&other_scope);
    let (handle, _host) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    // raw-px: fixed headless test viewport, independent of product geometry.
                    size(px(800.), px(500.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let ddl = DdlPanel::new(first_view.clone(), window, cx);
                cx.new(|_| Host { ddl })
            },
        )
        .unwrap()
    });
    cx.update(|cx| {
        Catalog::global(cx).update(cx, |catalog, cx| {
            for scope in [&scope, &other_scope] {
                catalog.ensure_schemas(&scope.connection, &scope.database, false, cx);
                catalog.ensure_objects(scope, ObjectKind::Table, false, cx);
                catalog.ensure_objects(scope, ObjectKind::View, false, cx);
                catalog.ensure_columns(scope, "items", false, cx);
                catalog.ensure_table_children(scope, "items", false, cx);
                catalog.ensure_ddl(&view(scope), false, cx);
            }
        });
    });
    wait(cx, "initial metadata", |cx| {
        cx.update(|cx| {
            let catalog = Catalog::global(cx);
            ready(catalog.read(cx), &scope, &first_view)
                && ready(catalog.read(cx), &other_scope, &other_view)
        })
    });
    let before_other = cx.update(|cx| {
        let catalog = Catalog::global(cx);
        let catalog = catalog.read(cx);
        (
            catalog.columns(&other_scope, "items").cloned(),
            catalog.ddl(&other_view).cloned(),
        )
    });
    let editor_value = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let editors = window.find_all_by_role(Role::MultilineTextInput);
            assert_eq!(editors.len(), 1, "the real DDL editor is mounted");
            assert!(editors[0].visible(), "the DDL editor is visible");
            editors[0]
                .value()
                .expect("the DDL editor exposes its actual document value")
                .to_string()
        })
        .unwrap()
    };
    wait(cx, "initial DDL editor", |cx| {
        editor_value(cx).contains("SELECT id FROM items")
    });

    execute(
        &first_file,
        "ALTER TABLE items ADD COLUMN name TEXT; \
         CREATE INDEX items_name ON items(name); \
         CREATE TABLE added(id INTEGER); \
         DROP VIEW shown; CREATE VIEW shown AS SELECT name FROM items; \
         CREATE VIEW added_view AS SELECT id FROM added",
        false,
    );
    execute(
        &other_file,
        "ALTER TABLE items ADD COLUMN untouched TEXT",
        false,
    );
    cx.update(|cx| {
        Workspace::global(cx).update(cx, |workspace, cx| {
            workspace.database_changed(first.clone(), cx)
        });
    });
    wait(cx, "refreshed metadata", |cx| {
        cx.update(|cx| {
            let catalog = Catalog::global(cx);
            ready(catalog.read(cx), &scope, &first_view)
                && matches!(
                    catalog.read(cx).columns(&scope, "items"),
                    Some(Load::Ready(columns)) if columns.iter().any(|column| column.name == "name")
                )
        })
    });
    wait(cx, "updated DDL editor", |cx| {
        editor_value(cx).contains("SELECT name FROM items")
    });
    cx.update(|cx| {
        let catalog = Catalog::global(cx);
        let catalog = catalog.read(cx);
        assert!(matches!(
            catalog.objects(&scope, ObjectKind::Table),
            Some(Load::Ready(objects)) if objects.iter().any(|object| object.name == "added")
        ));
        assert!(matches!(
            catalog.objects(&scope, ObjectKind::View),
            Some(Load::Ready(objects)) if objects.iter().any(|object| object.name == "added_view")
        ));
        assert!(matches!(
            catalog.table_children(&scope, "items"),
            Some(Load::Ready(children)) if children.indexes.iter().any(|name| name == "items_name")
        ));
        assert_eq!(
            (
                catalog.columns(&other_scope, "items").cloned(),
                catalog.ddl(&other_view).cloned()
            ),
            before_other,
            "a different connection's cached entries stay untouched"
        );
        assert!(catalog.columns(&scope, "added").is_none());
        assert!(catalog.table_children(&scope, "added").is_none());
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.close(&first, cx);
            sessions.close(&other, cx);
        });
    });
}
