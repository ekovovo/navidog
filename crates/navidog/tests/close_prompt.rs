//! Headless test of closing a table viewer tab with an uncommitted edit: ⌘W asks first and keeps
//! the tab, and "Commit" writes the edit and then closes it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Focusable as _, TestAppContext};
use navidog::main_window::MainWindow;
use navidog::state::AppState;
use navidog_core::actions::OpenObject;
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::Sessions;
use navidog_db::sqlite::SqliteDriver;
use navidog_db::{ConnectParams, Secrets, connect, runtime};
use navidog_ui::tabs::{self, CloseTab};
use navidog_ui::tokens;

fn wait(cx: &mut TestAppContext, what: &str, done: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sql(file: &std::path::Path, statements: &str) -> Vec<Vec<Option<String>>> {
    runtime::runtime().block_on(async {
        let params = ConnectParams {
            file: Some(file.to_path_buf()),
            create_new: !file.exists(),
            ..Default::default()
        };
        let mut s = connect(Arc::new(SqliteDriver::default()), params)
            .await
            .unwrap();
        let sets = s.relational().unwrap().execute(statements).await.unwrap();
        s.close().await.unwrap();
        sets.into_iter()
            .last()
            .map(|set| set.rows)
            .unwrap_or_default()
    })
}

fn tab_count(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| {
        tabs::group_of(tabs::active(cx).unwrap(), cx)
            .unwrap()
            .read(cx)
            .panels()
            .len()
    })
}

fn has(cx: &mut TestAppContext, handle: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn closing_a_tab_with_an_edit_asks_and_commits(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-close-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("close.db");
    sql(
        &file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t(name) VALUES ('a')",
    );
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    cx.update(navidog::init);
    let id = cx.update(|cx| {
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "close".into();
            c.general.file = Some(file.to_string_lossy().into_owned());
            let id = c.id.clone();
            s.upsert(c, &[], cx).unwrap();
            id
        })
    });
    let (handle, main) = cx.update(|cx| {
        let mut options = navidog::main_window_options(&AppState::default(), cx);
        options.show = false;
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| MainWindow::new(AppState::default(), window, cx))
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
    let table = ObjectRef {
        scope: Scope::new(&id, "main", None),
        kind: ObjectKind::Table,
        name: "t".into(),
    };
    let dispatch = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.focus(&main.focus_handle(cx), cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    dispatch(cx, Box::new(OpenObject { object: table }));
    wait(cx, "the table viewer's rows", |cx| {
        has(cx, handle, ElementId::NamedInteger("row".into(), 0))
    });
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    let tabs_before = tab_count(cx);

    // Edit the name of row 1 and stay on the row, so nothing commits yet
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rem = window.rem_size();
        let col = tokens::grid::COLUMN_WIDTH.to_pixels(rem);
        let x = tokens::grid::ROW_NUMBER_WIDTH.to_pixels(rem) + col + col / 2.;
        window.click_at(
            ElementId::NamedInteger("row".into(), 0),
            gpui_kit::point(x, rem / 2.),
            cx,
        );
        window.press("enter", cx);
        window.render_frame(cx);
        window.press("cmd-a", cx);
        window.input("z", cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();

    // ⌘W asks instead of closing
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(CloseTab), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        tab_count(cx),
        tabs_before,
        "the tab stays while the prompt is open"
    );
    assert!(has(cx, handle, "close-commit"), "the close prompt is shown");
    assert_eq!(
        sql(&file, "SELECT name FROM t WHERE id = 1"),
        [vec![Some("a".to_string())]]
    );

    // "Commit" writes the edit, then the tab closes
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("close-commit", cx);
    })
    .unwrap();
    wait(cx, "the commit and the tab to close", |cx| {
        tab_count(cx) == tabs_before - 1
    });
    assert_eq!(
        sql(&file, "SELECT name FROM t WHERE id = 1"),
        [vec![Some("z".to_string())]]
    );
    let _ = std::fs::remove_dir_all(&home);
}
