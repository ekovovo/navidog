//! Executing SQL through the query toolbar updates an already open table and view definition.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Role, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::Sessions;
use navidog_db::{ConnectParams, Secrets, connect, runtime, sqlite::SqliteDriver};
use navidog_grid::TableViewer;
use navidog_navigator::DdlPanel;
use navidog_query::{QueryEditor, Target};
use navidog_ui::tokens;

fn wait(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fixture(file: &Path) {
    runtime::runtime().block_on(async {
        let mut session = connect(
            Arc::new(SqliteDriver::default()),
            ConnectParams {
                file: Some(file.into()),
                create_new: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        session
            .relational()
            .unwrap()
            .execute(
                "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT); \
                 INSERT INTO items VALUES (1, 'before'); \
                 CREATE VIEW shown AS SELECT id FROM items",
            )
            .await
            .unwrap();
        session.close().await.unwrap();
    });
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            // raw-px: headless window with enough room for the query toolbar and grid
            size(px(1000.), px(700.)),
        ))),
        ..Default::default()
    }
}

fn copy_table(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    let found = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = ElementId::NamedInteger("row".into(), 0);
            if window.try_find(row.clone()).is_none() {
                return false;
            }
            window.click_at(
                row,
                gpui_kit::point(
                    tokens::grid::ROW_NUMBER_WIDTH.to_pixels(window.rem_size()) / 2.,
                    window.rem_size() / 2.,
                ),
                cx,
            );
            true
        })
        .unwrap();
    if !found {
        return None;
    }
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(navidog_grid::grid::SelectAll), cx);
        window.dispatch_action(Box::new(navidog_grid::grid::Copy), cx);
        cx.read_from_clipboard().and_then(|item| item.text())
    })
    .unwrap()
}

#[gpui_kit::test]
fn query_toolbar_refreshes_open_tables_and_view_definitions(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("navidog-query-refresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("fixture.db");
    fixture(&file);
    // SAFETY: this integration-test binary contains one test and owns its temporary settings.
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    cx.update(|cx| {
        navidog::init(cx);
        cx.set_reduce_motion(true);
        Settings::update(cx, |settings| settings.auto_begin_transaction = false);
    });
    let id = cx.update(|cx| {
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut connection = Connection::new("sqlite");
            connection.general.file = Some(file.to_string_lossy().into_owned());
            let id = connection.id.clone();
            store.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |sessions, cx| {
            sessions.open(id.clone(), Secrets::default(), cx);
        });
    });
    wait(cx, "the connection", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let scope = Scope::new(id.clone(), "main", None);
    let viewer: AnyWindowHandle = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            TableViewer::new(
                ObjectRef {
                    scope: scope.clone(),
                    kind: ObjectKind::Table,
                    name: "items".into(),
                },
                window,
                cx,
            )
        })
        .unwrap()
        .0
    });
    wait(cx, "the original table data", |cx| {
        copy_table(cx, viewer).is_some_and(|text| text.contains("before"))
    });
    let ddl: AnyWindowHandle = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            DdlPanel::new(
                ObjectRef {
                    scope: scope.clone(),
                    kind: ObjectKind::View,
                    name: "shown".into(),
                },
                window,
                cx,
            )
        })
        .unwrap()
        .0
    });
    let ddl_value = |cx: &mut TestAppContext| {
        cx.update_window(ddl, |_, window, cx| {
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
    wait(cx, "the original view definition", |cx| {
        ddl_value(cx).contains("SELECT id")
    });
    let query: AnyWindowHandle = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            QueryEditor::from_sql(
                Target { connection: Some(id.clone()), database: Some("main".into()), schema: None },
                "UPDATE items SET name = 'after'; DROP VIEW shown; CREATE VIEW shown AS SELECT name FROM items".into(),
                window,
                cx,
            )
        })
        .unwrap()
        .0
    });
    cx.update_window(query, |_, window, cx| {
        window.render_frame(cx);
        window.click("query-run", cx);
    })
    .unwrap();
    wait(
        cx,
        "table data to refresh without a Refresh command",
        |cx| {
            copy_table(cx, viewer)
                .is_some_and(|text| text.contains("after") && !text.contains("before"))
        },
    );
    wait(cx, "the already open view definition to refresh", |cx| {
        ddl_value(cx).contains("SELECT name")
    });
    cx.update(|cx| Sessions::global(cx).update(cx, |sessions, cx| sessions.close(&id, cx)));
}
