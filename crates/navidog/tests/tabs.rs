//! Headless test of the editor tabs in the main window: the close button, ⌘W, and the tab
//! context menu's "close others"; the objects tab can never be closed.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use navidog::main_window::MainWindow;
use navidog::state::AppState;
use navidog_core::actions::OpenObject;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_ui::tabs::{self, Close, CloseTab, CloseTabs, PopOutTab};

fn view(name: &str) -> ObjectRef {
    ObjectRef {
        scope: Scope::new("c", "main", None),
        kind: ObjectKind::View,
        name: name.into(),
    }
}

#[gpui_kit::test]
fn close_tabs(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-tabs-test-{}", std::process::id()));
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    cx.update(navidog::init);
    let (handle, main) = cx.update(|cx| {
        let mut options = navidog::main_window_options(&AppState::default(), cx);
        options.show = false;
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| MainWindow::new(AppState::default(), window, cx))
        })
        .unwrap()
    });
    let dispatch = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.focus(&main.focus_handle(cx), cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let tab_count = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            tabs::group_of(tabs::active(cx).unwrap(), cx)
                .unwrap()
                .read(cx)
                .panels()
                .len()
        })
    };
    for name in ["a", "b", "c", "d"] {
        dispatch(cx, Box::new(OpenObject { object: view(name) }));
    }
    assert_eq!(tab_count(cx), 5, "the objects tab plus four editors");

    // The close button on a tab: tab 1 is the first editor
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::ElementId::NamedInteger("close-tab".into(), 1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_count(cx), 4);

    // ⌘W closes the active tab (the last opened one)
    dispatch(cx, Box::new(CloseTab));
    assert_eq!(tab_count(cx), 3);

    // "Open in new window" moves the active editor out of the tab group into a window of its own
    let windows = cx.update(|cx| cx.windows().len());
    let active = cx.update(|cx| tabs::active(cx).unwrap());
    dispatch(cx, Box::new(PopOutTab { panel: active }));
    assert_eq!(tab_count(cx), 2);
    assert_eq!(cx.update(|cx| cx.windows().len()), windows + 1);
    dispatch(cx, Box::new(OpenObject { object: view("e") }));
    assert_eq!(tab_count(cx), 3);

    // "Close other tabs" on the active editor keeps it and the objects tab
    dispatch(
        cx,
        Box::new(CloseTabs {
            panel: None,
            which: Close::Others,
        }),
    );
    assert_eq!(tab_count(cx), 2);
    dispatch(
        cx,
        Box::new(CloseTabs {
            panel: None,
            which: Close::All,
        }),
    );
    assert_eq!(tab_count(cx), 1, "the objects tab is not closable");
    let _ = std::fs::remove_dir_all(home);
}
