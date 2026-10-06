//! Headless UI test of connection management: create a SQLite connection with real clicks and
//! typing and open it, then add and switch profiles and use the Manage Connections window.

use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement, Render,
    Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, size,
};
use navidog_core::actions::{
    ManageConnections, NewConnectionOf, NewProfile, OpenConnection, SwitchProfile,
};
use navidog_core::crypto::Key;
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::{ConnState, Sessions};

struct Host {
    focus: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_connection::bind_actions(div().id("host").track_focus(&self.focus).size_full())
    }
}

/// Waits for the driver, which runs on the tokio runtime, to report the connection open.
fn wait_open(cx: &mut TestAppContext, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let state = cx.update(|cx| Sessions::global(cx).read(cx).state(id).clone());
        match state {
            ConnState::Open { databases, .. } => {
                assert_eq!(databases, ["main"]);
                return;
            }
            ConnState::Failed(e) => panic!("opening the connection failed: {e}"),
            _ if Instant::now() > deadline => panic!("opening the connection timed out: {state:?}"),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[gpui_kit::test]
fn creates_and_opens_sqlite_connection(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-ui-test-{}", std::process::id()));
    let db_file = home.join("app.db");
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_connection::init(cx);
        // Keep away from the keychain
        ConnectionStore::global(cx).update(cx, |s, _| s.set_key(Key::random()));
    });
    let (handle, host) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(1000.), gpui_kit::px(700.));
        let bounds = gpui_kit::Bounds {
            origin: Default::default(),
            size: test_size,
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| Host {
                focus: cx.focus_handle(),
            })
        })
        .unwrap()
    });
    let dispatch = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.focus(&host.read(cx).focus.clone(), cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    dispatch(
        cx,
        Box::new(NewConnectionOf {
            driver: "sqlite".into(),
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Pick "new SQLite 3" and fill in the file path and connection name;
        // RadioGroup uses the index as the child id
        window
            .within("sqlite-type")
            .click(gpui_kit::ElementId::Integer(1), cx);
        window.click("file-browse-input", cx);
        window.input(&db_file.to_string_lossy(), cx);
        window.click("conn-name", cx);
        window.press("cmd-a", cx);
        window.input("测试库", cx);
        window.click("ok", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let id = cx.update(|cx| {
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        assert_eq!(store.connections().len(), 1);
        let c = &store.connections()[0];
        assert_eq!((c.name.as_str(), c.driver.as_str()), ("测试库", "sqlite"));
        assert_eq!(
            c.general.file.as_deref(),
            Some(db_file.to_string_lossy().as_ref())
        );
        assert!(c.general.create_new);
        c.id.clone()
    });
    // The form closed after saving; the new connection is selected, so opening acts on it
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("conn-name").is_none());
    })
    .unwrap();
    dispatch(cx, Box::new(OpenConnection));
    cx.executor().allow_parking();
    wait_open(cx, &id);
    assert!(db_file.exists(), "new SQLite 3 creates the file");
    // connections.json on disk has the saved settings
    let on_disk = std::fs::read_to_string(home.join("connections.json")).unwrap();
    assert!(on_disk.contains("测试库"));

    // A new profile starts as a copy of the active settings, becomes active and reconnects
    dispatch(cx, Box::new(NewProfile));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("profile-name", cx);
        window.input("只读", cx);
        window.click("ok", cx);
        window.render_frame(cx);
        // The form opens on the new profile; leave it as is
        window.click("cancel", cx);
    })
    .unwrap();
    wait_open(cx, &id);
    let profile = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let c = ConnectionStore::global(cx)
                .read(cx)
                .get(&id)
                .cloned()
                .unwrap();
            (c.profile, c.profiles.len())
        })
    };
    assert_eq!(profile(cx), ("只读".into(), 1));
    dispatch(cx, Box::new(SwitchProfile { name: "".into() }));
    wait_open(cx, &id);
    assert_eq!(profile(cx), ("".into(), 1));

    // Manage Connections: the search filters rows, the star button stars the selected connection
    dispatch(cx, Box::new(ManageConnections));
    let manage = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| w.window_id() != handle.window_id())
            .unwrap()
    });
    cx.update_window(manage, |_, window, cx| {
        window.render_frame(cx);
        window.click("manage-search", cx);
        window.input("zzz", cx);
        window.render_frame(cx);
        assert!(
            window.try_find(("manage-conn", 0usize)).is_none(),
            "no connection matches"
        );
        window.press("cmd-a", cx);
        window.input("测试", cx);
        window.render_frame(cx);
        window.click(("manage-conn", 0usize), cx);
        window.render_frame(cx);
        window.click("manage-star", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| {
        ConnectionStore::global(cx)
            .read(cx)
            .get(&id)
            .unwrap()
            .starred
    }));
    let _ = std::fs::remove_dir_all(home);
}
