//! Headless test of the app-menu actions that need a window (F-APP-018). The menu bar dispatches
//! an action while the window it belongs to is out of its slot, so a handler that updates the same
//! window finds nothing to update and the click does nothing.

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use navidog::actions::About;
use navidog::main_window::MainWindow;
use navidog::state::AppState;

#[gpui_kit::test]
fn about_shows_its_dialog_when_dispatched_from_a_window_update(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-app-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
    cx.update(|cx| {
        navidog::init(cx);
        navidog::register_app_actions(cx);
    });
    let (handle, view) = cx.update(|cx| {
        let mut options = navidog::main_window_options(&AppState::default(), cx);
        options.show = false;
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| MainWindow::new(AppState::default(), window, cx))
        })
        .unwrap()
    });
    cx.executor().allow_parking();

    // What the menu bar does: dispatch inside the window's own update.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.focus(&view.focus_handle(cx), cx);
        window.dispatch_action(Box::new(About), cx);
    })
    .unwrap();
    cx.run_until_parked();

    let shown = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.has_active_dialog(cx)
        })
        .unwrap();
    assert!(shown, "About opened no dialog");
    let _ = std::fs::remove_dir_all(&home);
}
