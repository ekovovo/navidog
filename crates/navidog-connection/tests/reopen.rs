//! Explicit recovery asks before replacing a session and emits Reopening instead of Closed.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement, Render,
    Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use navidog_core::actions::{OpenConnectionId, ReopenConnectionId};
use navidog_core::connection::Connection;
use navidog_core::store::ConnectionStore;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent};

struct Host {
    focus: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        navidog_connection::bind_actions(div().id("host").track_focus(&self.focus).size_full())
    }
}

fn wait_open(cx: &mut TestAppContext, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let state = cx.update(|cx| Sessions::global(cx).read(cx).state(id).clone());
        match state {
            ConnState::Open { .. } => return,
            ConnState::Failed(error) => panic!("opening fixture connection failed: {error}"),
            _ if Instant::now() > deadline => panic!("opening fixture connection timed out"),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[gpui_kit::test]
fn reopening_requires_confirmation_and_preserves_the_editor_lifecycle(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("navidog-reopen-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this binary contains one test; no other thread reads the environment yet.
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    let id = cx.update(|cx| {
        gpui_kit::init(cx);
        navidog_core::init(cx);
        navidog_ui::theme::init(cx);
        navidog_db::init(cx);
        navidog_connection::init(cx);
        cx.set_reduce_motion(true);
        let mut connection = Connection::new("sqlite");
        connection.name = "Recovery fixture".into();
        connection.general.file = Some(home.join("fixture.db").to_string_lossy().into_owned());
        connection.general.create_new = true;
        let id = connection.id.clone();
        ConnectionStore::global(cx)
            .update(cx, |store, cx| store.upsert(connection, &[], cx))
            .unwrap();
        id
    });
    let (handle, host) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    // raw-px: headless test window size
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| Host {
                    focus: cx.focus_handle(),
                })
            },
        )
        .unwrap()
    });
    let reopenings = Rc::new(Cell::new(0));
    let closings = Rc::new(Cell::new(0));
    let reopened = reopenings.clone();
    let closed = closings.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&Sessions::global(cx), move |_, event, _| match event {
            SessionsEvent::Reopening(_) => reopened.set(reopened.get() + 1),
            SessionsEvent::Closed(_) => closed.set(closed.get() + 1),
            _ => {}
        })
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
        Box::new(OpenConnectionId {
            id: id.clone().into(),
        }),
    );
    wait_open(cx, &id);
    for (index, (locale, reopen_label)) in [("en", "Reopen"), ("zh-CN", "重新打开")]
        .into_iter()
        .enumerate()
    {
        rust_i18n::set_locale(locale);
        dispatch(cx, Box::new(ReopenConnectionId::new(id.clone())));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            assert_eq!(window.find("ok").label(), Some(reopen_label));
            window.click("cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(reopenings.get(), index, "cancel does not close or reopen");
        assert_eq!(closings.get(), 0);
        assert!(cx.update(|cx| Sessions::global(cx).read(cx).state(&id).is_open()));
        dispatch(cx, Box::new(ReopenConnectionId::new(id.clone())));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();
        wait_open(cx, &id);
        assert_eq!(reopenings.get(), index + 1, "confirm reopens exactly once");
        assert_eq!(
            closings.get(),
            0,
            "editors do not receive a real close event"
        );
    }
    cx.update(|cx| Sessions::global(cx).update(cx, |sessions, cx| sessions.close(&id, cx)));
    let _ = std::fs::remove_dir_all(&home);
}
