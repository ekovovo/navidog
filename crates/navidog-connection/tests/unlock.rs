//! Headless UI test of the unlock dialog: one Enter in the master password field is one attempt.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement, Render,
    Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, size,
};
use navidog_core::actions::OpenConnection;
use navidog_core::connection::{Connection, SecretField};
use navidog_core::store::{ConnectionStore, ConnectionsFile};
use navidog_core::workspace::{Selection, Workspace};

struct Host {
    focus: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_connection::bind_actions(div().id("host").track_focus(&self.focus).size_full())
    }
}

#[gpui_kit::test]
fn one_enter_is_one_unlock_attempt(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-unlock-test-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // A master-password file with one saved password, so opening asks to unlock first
    let (crypto, key) = navidog_core::crypto::new_master("right").unwrap();
    let mut conn = Connection::new("mysql");
    conn.general.save_password = true;
    ConnectionsFile::set_secret(&mut conn, SecretField::Password, Some("pw"), &key);
    let id = conn.id.clone();
    let file = ConnectionsFile {
        crypto,
        connections: vec![conn],
        ..ConnectionsFile::default()
    };
    std::fs::write(
        home.join("connections.json"),
        serde_json::to_vec(&file).unwrap(),
    )
    .unwrap();
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_connection::init(cx);
        assert!(ConnectionStore::global(cx).read(cx).is_locked());
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Connection(id), cx));
    });
    let (handle, host) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(600.));
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
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.focus(&host.read(cx).focus.clone(), cx);
        window.dispatch_action(Box::new(OpenConnection), cx);
    })
    .unwrap();
    cx.run_until_parked();

    // The unlock dialog focuses its field; a wrong password and one Enter
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("wrong", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let failures = cx.update(|cx| ConnectionStore::global(cx).read(cx).unlock_failures);
    assert_eq!(failures, 1);
    let _ = std::fs::remove_dir_all(&home);
}
