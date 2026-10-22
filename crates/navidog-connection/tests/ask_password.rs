//! Saving the password can fail without failing the connection that was just typed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, IntoElement, Render, Styled as _, TestAppContext, Window, WindowBounds,
    WindowOptions, div, size,
};
use navidog_core::connection::Connection;
use navidog_core::crypto::Key;
use navidog_core::paths::config_dir;
use navidog_core::store::ConnectionStore;

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

#[gpui_kit::test]
fn a_failed_save_still_connects_once(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-ask-password-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this file is its own test binary, and the directory is unique to this process.
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_connection::init(cx);
        cx.set_reduce_motion(true);
        ConnectionStore::global(cx).update(cx, |store, cx| {
            store.set_key(Key::random());
            let mut connection = Connection::new("mysql");
            connection.name = "ask".into();
            connection.general.save_password = false;
            let id = connection.id.clone();
            store.upsert(connection, &[], cx).unwrap();
            id
        })
    });
    let path = config_dir().join("connections.json");
    let before = std::fs::read(&path).unwrap();
    std::fs::create_dir(path.with_extension("tmp")).unwrap();

    let hits = Rc::new(Cell::new(0usize));
    let password = Rc::new(RefCell::new(None));
    let (handle, _) = cx.update(|cx| {
        // raw-px: test window size
        let test_size = size(gpui_kit::px(800.), gpui_kit::px(600.));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: Default::default(),
                size: test_size,
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Host)).unwrap()
    });
    let seen = hits.clone();
    let saved = password.clone();
    cx.update_window(handle, |_, window, cx| {
        navidog_connection::resolve(id.clone(), window, cx, move |secrets, _, _| {
            seen.set(seen.get() + 1);
            *saved.borrow_mut() = secrets.password;
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("s3cret", cx);
        window.click("save", cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(hits.get(), 1);
    assert_eq!(password.borrow().as_deref(), Some("s3cret"));
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
    })
    .unwrap();
    cx.update(|cx| {
        let conn = ConnectionStore::global(cx)
            .read(cx)
            .get(&id)
            .unwrap()
            .clone();
        assert!(!conn.general.save_password);
        assert!(conn.general.password.is_none());
    });
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
