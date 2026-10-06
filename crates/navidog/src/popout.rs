//! Editor tabs popped out into windows of their own (20-objects.md §5). Closing such a window
//! puts the editor back into the main window, so open sessions and pending edits survive.

use std::sync::Arc;

use gpui_kit::component::dock::{BasePanelView, PanelHandle};
use gpui_kit::component::{TitleBar, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window, div,
};
use navidog_ui::palette;

use crate::main_window::MainWindow;
use crate::state::AppState;

struct PoppedOut {
    panel: Arc<dyn BasePanelView>,
}

impl Render for PoppedOut {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = PanelHandle::of(&self.panel).map(|h| h.title(window, cx));
        v_flex()
            .size_full()
            .bg(palette(cx).window_bg)
            .child(TitleBar::new().child(div().flex_1().flex().justify_center().children(title)))
            .child(div().flex_1().min_h_0().child(self.panel.view()))
    }
}

/// Opens `panel` (already detached from the main dock) in a new window and returns its handle.
pub(crate) fn open(
    panel: Arc<dyn BasePanelView>,
    main: WeakEntity<MainWindow>,
    main_window: AnyWindowHandle,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let mut options = crate::main_window_options(&AppState::default(), cx);
    options.window_bounds = None;
    let (handle, _) = gpui_kit::open_window(options, cx, move |window, cx| {
        let back = panel.clone();
        window.on_window_should_close(cx, move |_, cx| {
            let panel = back.clone();
            let main = main.clone();
            cx.defer(move |cx| {
                let _ = main_window.update(cx, |_, window, cx| {
                    let _ = main.update(cx, |this, cx| this.restore_tab(panel, window, cx));
                });
            });
            true
        });
        cx.new(|_| PoppedOut { panel })
    })
    .ok()?;
    Some(handle)
}
