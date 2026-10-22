//! Headless test of the info pane's query pages: they come and go with the query editor, a
//! snippet saves from the editor dialog, and identifiers list a real SQLite table.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, WindowOptions, div,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::snippets::Snippets;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{QueryContext, Workspace};
use navidog_db::sessions::Sessions;
use navidog_db::{ConnectParams, Secrets, connect, runtime, sqlite::SqliteDriver};

use crate::info::{InfoPanel, Page};

struct Host {
    info: Entity<InfoPanel>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::bind_actions(div().size_full().child(self.info.clone()))
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

#[gpui_kit::test]
fn query_pages_follow_the_editor(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-info-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("i.db");
    runtime::runtime().block_on(async {
        let p = ConnectParams {
            file: Some(file.clone()),
            create_new: true,
            ..Default::default()
        };
        let mut s = connect(Arc::new(SqliteDriver::default()), p).await.unwrap();
        s.relational()
            .unwrap()
            .execute("CREATE TABLE film(film_id INTEGER PRIMARY KEY, title TEXT)")
            .await
            .unwrap();
        s.close().await.unwrap();
    });
    // SAFETY: `lock` is held for this test, so no other test in this crate reads the variable.
    let _home = crate::test_env::lock();
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        crate::init(cx);
        // Keep dialog hit regions fixed between the frame sampled by click and its dispatch.
        cx.set_reduce_motion(true);
        ConnectionStore::global(cx).update(cx, |s, cx| {
            s.set_key(Key::random());
            let mut c = Connection::new("sqlite");
            c.name = "i".into();
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
    wait(cx, "the connection", |cx| {
        cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open())
    });

    let (handle, info) = cx.update(|cx| {
        let mut info = None;
        let h = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let i = InfoPanel::shared(window, cx);
            info = Some(i.clone());
            cx.new(|_| Host { info: i })
        })
        .unwrap();
        (h.0, info.unwrap())
    });
    let page = |cx: &mut TestAppContext| cx.update(|cx| info.read(cx).page);
    assert!(page(cx) == Page::General);

    // A query editor becomes active: the snippets page shows
    cx.update(|cx| {
        Workspace::global(cx).update(cx, |w, cx| {
            w.set_query_context(
                Some(QueryContext {
                    connection: Some(id.clone()),
                    database: Some("main".into()),
                    schema: None,
                }),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(page(cx) == Page::Snippets);

    // New snippet through the editor dialog
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("snippet-new").is_some(), "the + button");
        window.click("snippet-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("snippet-name").is_some(),
            "the editor dialog"
        );
        window.click("snippet-name", cx);
        window.input("films", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let name = window.find("snippet-name");
        assert_eq!(name.value(), Some("films"));
        assert_eq!(name.focused(), Some(true));
        window.click("snippet-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let saved = cx.update(|cx| {
        Snippets::global(cx)
            .read(cx)
            .all()
            .find(|s| s.name == "films")
            .cloned()
    });
    let saved = saved.expect("the snippet was saved");
    assert_eq!(
        saved.driver, "sqlite",
        "a new snippet is for the editor's database type"
    );

    // Identifiers list the table from the completion info
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("info-identifiers", cx);
    })
    .unwrap();
    let scope = navidog_core::object::Scope::new(id.clone(), "main", None);
    wait(cx, "the identifiers", |cx| {
        cx.update(|cx| {
            navidog_db::outline::OutlineCache::global(cx)
                .read(cx)
                .ready(&scope)
                .is_some_and(|o| o.table("film").is_some_and(|t| t.columns.len() == 2))
        })
    });
    assert!(page(cx) == Page::Identifiers);

    // The editor goes: back to the page shown before
    cx.update(|cx| Workspace::global(cx).update(cx, |w, cx| w.set_query_context(None, cx)));
    cx.run_until_parked();
    assert!(page(cx) == Page::General);
    let _ = std::fs::remove_dir_all(&home);
}
