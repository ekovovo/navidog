//! Headless UI test of navigator arrow keys on a real SQLite connection: once a table is expanded,
//! ↓ walks through the rows under it to the next table instead of stopping on the table row.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowBounds, WindowOptions,
    div, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::sessions::Sessions;
use navidog_db::sqlite::SqliteDriver;
use navidog_db::{ConnectParams, Secrets, connect, runtime};
use navidog_navigator::NavigatorPanel;
use navidog_navigator::catalog::{Catalog, Load};

struct Host {
    focus: FocusHandle,
    navigator: gpui_kit::Entity<NavigatorPanel>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_navigator::bind_actions(
            div()
                .id("host")
                .track_focus(&self.focus)
                .size_full()
                .child(self.navigator.clone()),
        )
    }
}

/// Runs the app and the tokio driver until `done`, or fails after a few seconds.
fn wait(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn arrow_keys_walk_through_the_rows_under_a_table(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-nav-keys-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("keys.db");
    runtime::runtime().block_on(async {
        let params = ConnectParams {
            file: Some(file.clone()),
            create_new: true,
            ..Default::default()
        };
        let mut s = connect(Arc::new(SqliteDriver::default()), params)
            .await
            .unwrap();
        s.relational()
            .unwrap()
            .execute(
                "CREATE TABLE t1(id INTEGER PRIMARY KEY, name TEXT); CREATE TABLE t2(id INTEGER)",
            )
            .await
            .unwrap();
        s.close().await.unwrap();
    });
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_navigator::init(cx);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "keys".into();
            c.general.file = Some(file.to_string_lossy().into_owned());
            let id = c.id.clone();
            s.upsert(c, &[], cx).unwrap();
            id
        })
    });
    let (handle, _host) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(400.), gpui_kit::px(900.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let navigator = NavigatorPanel::shared(window, cx);
            cx.new(|cx| Host {
                focus: cx.focus_handle(),
                navigator,
            })
        })
        .unwrap()
    });
    cx.executor().allow_parking();
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    wait(cx, "the connection to open", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });

    let scope = Scope {
        connection: id.clone(),
        database: "main".into(),
        schema: None,
    };
    let table = |name: &str| {
        Selection::Object(ObjectRef {
            scope: scope.clone(),
            kind: ObjectKind::Table,
            name: name.into(),
        })
    };
    let selection = |cx: &mut TestAppContext| {
        cx.update(|cx| Workspace::global(cx).read(cx).selection().clone())
    };
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // The connection row, then main → expand → Tables → expand → t1 → expand
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("c:{id}"), cx);
    })
    .unwrap();
    for key in ["down", "right", "down", "right"] {
        press(cx, key);
    }
    wait(cx, "the table list", |cx| {
        cx.update(|cx| {
            matches!(
                Catalog::global(cx)
                    .read(cx)
                    .objects(&scope, ObjectKind::Table),
                Some(Load::Ready(_))
            )
        })
    });
    press(cx, "down");
    assert_eq!(selection(cx), table("t1"));
    press(cx, "right");
    wait(cx, "t1's fields", |cx| {
        cx.update(|cx| {
            matches!(
                Catalog::global(cx).read(cx).columns(&scope, "t1"),
                Some(Load::Ready(_))
            )
        })
    });
    // Let the indexes, foreign keys and triggers under t1 arrive too
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }

    // The folders under t1 stand for t1; ↓ must get past them to t2
    let mut reached = false;
    for _ in 0..8 {
        press(cx, "down");
        if selection(cx) == table("t2") {
            reached = true;
            break;
        }
        assert_eq!(selection(cx), table("t1"), "rows under t1 select t1");
    }
    assert!(reached, "↓ walks past the rows under t1 to t2");
    let _ = std::fs::remove_dir_all(&home);
}
