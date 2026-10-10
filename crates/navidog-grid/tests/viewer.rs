//! Headless UI test of the table viewer on a real SQLite table: paging with an uncommitted edit
//! commits it first, and "auto begin transaction" still loads the first page.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::Sessions;
use navidog_db::sqlite::SqliteDriver;
use navidog_db::{ConnectParams, Secrets, connect, runtime};
use navidog_grid::TableViewer;
use navidog_ui::tokens;

struct Host {
    viewer: Entity<TableViewer>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.viewer.clone())
    }
}

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

fn open_viewer(cx: &mut TestAppContext, object: ObjectRef) -> AnyWindowHandle {
    cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(900.), gpui_kit::px(500.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let viewer = TableViewer::new(object, window, cx);
            cx.new(|_| Host { viewer })
        })
        .unwrap()
        .0
    })
}

fn has_rows(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(ElementId::NamedInteger("row".into(), 0))
            .is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn paging_commits_first_and_auto_transaction_still_loads(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-viewer-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("viewer.db");
    sql(
        &file,
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO t(name) VALUES ('a'), ('b'), ('c')",
    );
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_grid::init(cx);
        Settings::update(cx, |s| {
            s.limit_records = true;
            s.records_per_page = 2;
            s.auto_begin_transaction = false;
        });
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "viewer".into();
            c.general.file = Some(file.to_string_lossy().into_owned());
            let id = c.id.clone();
            s.upsert(c, &[], cx).unwrap();
            id
        })
    });
    cx.executor().allow_parking();
    cx.update(|cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(id.clone(), Secrets::default(), cx))
    });
    wait(cx, "the connection to open", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });
    let table = ObjectRef {
        scope: Scope {
            connection: id.clone(),
            database: "main".into(),
            schema: None,
        },
        kind: ObjectKind::Table,
        name: "t".into(),
    };

    // Edit row 1's name, then go to the next page without leaving the row
    let first = open_viewer(cx, table.clone());
    wait(cx, "the first page", |cx| has_rows(cx, first));
    // Let the row count arrive so the next-page button is enabled
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(20));
    }
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        let rem = window.rem_size();
        let col = tokens::grid::COLUMN_WIDTH.to_pixels(rem);
        // The name column: past the marker column and id
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
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        window.click("page-next", cx);
    })
    .unwrap();
    wait(cx, "the edit to be committed before paging", |_| {
        sql(&file, "SELECT name FROM t WHERE id = 1") == [vec![Some("z".to_string())]]
    });

    // Opening a table with "auto begin transaction" sends BEGIN right after the first SELECT
    cx.update(|cx| Settings::update(cx, |s| s.auto_begin_transaction = true));
    let second = open_viewer(cx, table);
    wait(cx, "the first page with auto begin transaction", |cx| {
        has_rows(cx, second)
    });
    let _ = std::fs::remove_dir_all(&home);
}
