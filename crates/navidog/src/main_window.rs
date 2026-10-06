//! Main window: title bar, main toolbar, dock and status bar (10-shell.md §2–4, §8).

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::dock::{
    BasePanelView, DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin,
    PanelId, panel_handle,
};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div,
};
use navidog_core::actions::{
    OpenObject, ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden,
};
use navidog_core::object::ObjectRef;
use navidog_core::registry::CommandRegistry;
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::Workspace;
use navidog_navigator::{InfoPanel, NavigatorPanel, ObjectsPanel};
use navidog_ui::assets::lucide;
use navidog_ui::skin::ToolbarItem;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::actions::*;
use crate::state::{AppState, Frame};

/// Dock layout version; bump it when the default layout structure changes so old layouts are discarded.
const DOCK_VERSION: usize = 1;

pub struct MainWindow {
    focus_handle: FocusHandle,
    dock: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    state: AppState,
    info_restore: Option<gpui_kit::Pixels>,
    /// Open editor tabs, so an object is not opened twice
    open_tabs: std::collections::HashMap<ObjectRef, PanelId>,
    /// Editors popped out into their own windows
    popped: std::collections::HashMap<PanelId, gpui_kit::AnyWindowHandle>,
    _subscriptions: Vec<Subscription>,
}

/// Main toolbar entries: (id, Lucide icon, i18n key, command id in the command registry).
/// Entries whose command is not registered are shown disabled (implemented in later milestones).
const TOOLBAR: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "tb-connection",
        "plug",
        "toolbar.connection",
        Some("connection.new"),
    ),
    (
        "tb-new-query",
        "square-terminal",
        "toolbar.new_query",
        Some("query.new"),
    ),
    ("|", "", "", None),
    ("tb-table", "table", "toolbar.table", Some("objects.tables")),
    ("tb-view", "eye", "toolbar.view", Some("objects.views")),
    (
        "tb-function",
        "square-function",
        "toolbar.function",
        Some("objects.functions"),
    ),
    ("tb-user", "user", "toolbar.user", Some("objects.users")),
    (
        "tb-others",
        "shapes",
        "toolbar.others",
        Some("objects.others"),
    ),
    (
        "tb-query",
        "file-code",
        "toolbar.query",
        Some("objects.queries"),
    ),
    (
        "tb-backup",
        "database-backup",
        "toolbar.backup",
        Some("objects.backups"),
    ),
    (
        "tb-automation",
        "calendar-clock",
        "toolbar.automation",
        Some("objects.automation"),
    ),
    (
        "tb-model",
        "workflow",
        "toolbar.model",
        Some("objects.models"),
    ),
    ("tb-bi", "chart-pie", "toolbar.bi", Some("objects.bi")),
];

impl MainWindow {
    pub fn new(state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (dock, skin) = DockSkin::dock_area("main-dock", Some(DOCK_VERSION), window, cx);
        skin.set_close_button_visible(true, cx);
        let info = InfoPanel::shared(window, cx);
        NavigatorPanel::shared(window, cx);
        ObjectsPanel::shared(window, cx);

        let restored = state
            .dock
            .clone()
            .and_then(|v| serde_json::from_value::<DockAreaState>(v).ok())
            .filter(|s| s.version == Some(DOCK_VERSION))
            .is_some_and(|s| dock.update(cx, |a, cx| a.load(s, window, cx)).is_ok());
        if !restored {
            Self::default_layout(&dock, window, cx);
        }

        let weak = cx.entity().downgrade();
        info.update(cx, |p, _| {
            p.set_on_toggle_maximize(move |window, cx| {
                let _ = weak.update(cx, |this, cx| this.toggle_info_maximized(window, cx));
            })
        });
        navidog_navigator::set_view_options(state.view, cx);

        let subs = vec![
            cx.subscribe_in(&dock, window, |this, _, e: &DockEvent, window, cx| {
                if matches!(e, DockEvent::LayoutChanged) {
                    this.save_state(window, cx);
                }
            }),
            cx.observe(&Workspace::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| {
                if navidog_core::settings::Settings::global(cx)
                    .appearance
                    .is_dark()
                    .is_none()
                {
                    navidog_ui::theme::apply_appearance(None, Some(window), cx);
                }
            }),
        ];
        window.on_window_should_close(cx, {
            let weak = cx.entity().downgrade();
            move |window, cx| {
                let _ = weak.update(cx, |this, cx| this.save_state(window, cx));
                true
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            dock,
            _skin: skin,
            state,
            info_restore: None,
            open_tabs: Default::default(),
            popped: Default::default(),
            _subscriptions: subs,
        }
    }

    fn default_layout(dock: &Entity<DockArea>, window: &mut Window, cx: &mut App) {
        let navigator = NavigatorPanel::shared(window, cx);
        let objects = ObjectsPanel::shared(window, cx);
        let info = InfoPanel::shared(window, cx);
        let rem = window.rem_size();
        dock.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(objects), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(panel_handle(navigator), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs().panel_view(panel_handle(info), cx),
                window,
                cx,
            );
            area.set_dock_size(
                DockPlacement::Left,
                tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem),
                window,
                cx,
            );
            area.set_dock_size(
                DockPlacement::Right,
                tokens::pane::INFO_WIDTH.to_pixels(rem),
                window,
                cx,
            );
        });
    }

    pub fn save_state(&mut self, window: &mut Window, cx: &mut App) {
        self.state.window = Some(Frame::from_bounds(window.window_bounds()));
        self.state.dock = serde_json::to_value(self.dock.read(cx).dump(cx)).ok();
        self.state.save();
    }

    fn toggle(
        &mut self,
        f: impl FnOnce(&mut AppState),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.state);
        navidog_navigator::set_view_options(self.state.view, cx);
        cx.set_global(self.state.clone());
        crate::menus::set_menus(cx);
        self.save_state(window, cx);
        cx.notify();
    }

    fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock
            .update(cx, |a, cx| a.toggle_dock(placement, window, cx));
    }

    /// Info pane maximize/restore: widens the right dock to the window width minus the navigator; clicking again restores it.
    fn toggle_info_maximized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rem = window.rem_size();
        let restore = self.info_restore.take();
        let size = match restore {
            Some(prev) => prev,
            None => {
                self.info_restore = self.dock.read(cx).dock_size(DockPlacement::Right);
                let total = window.viewport_size().width;
                total
                    - tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem)
                    - tokens::list::ROW_HEIGHT.to_pixels(rem)
            }
        };
        self.dock.update(cx, |a, cx| {
            a.set_dock_size(DockPlacement::Right, size, window, cx)
        });
    }

    fn reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::default_layout(&self.dock, window, cx);
        self.save_state(window, cx);
    }

    /// Opens an object in the object pane, switching to its tab (or its own window) if it is already open.
    fn open_object(&mut self, object: &ObjectRef, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.open_tabs.get(object).copied() {
            if self.dock.read(cx).panel(id).is_some() {
                self.dock.update(cx, |a, cx| a.select_panel(id, window, cx));
                return;
            }
            if let Some(w) = self.popped.get(&id) {
                let _ = w.update(cx, |_, window, _| window.activate_window());
                return;
            }
        }
        let panel = crate::editors::open(object.clone(), window, cx);
        let id = panel.panel_id(cx);
        self.open_tabs.insert(object.clone(), id);
        if Settings::global(cx).open_in_new_window {
            self.popped_out(panel, window, cx);
            return;
        }
        self.dock.update(cx, |a, cx| {
            a.add_panel_view(panel, DockPlacement::Center, None, window, cx)
        });
    }

    fn pop_out(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.dock.read(cx).panel(panel).cloned() else {
            return;
        };
        if !view.closable(cx) {
            return;
        }
        navidog_ui::tabs::detach(panel, cx);
        self.popped_out(view, window, cx);
    }

    fn popped_out(
        &mut self,
        view: Arc<dyn BasePanelView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.panel_id(cx);
        let main = cx.entity().downgrade();
        if let Some(w) = crate::popout::open(view, main, window.window_handle(), cx) {
            self.popped.insert(id, w);
        }
    }

    /// Puts a popped-out editor back into the object pane (its window is closing).
    pub(crate) fn restore_tab(
        &mut self,
        view: Arc<dyn BasePanelView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.popped.remove(&view.panel_id(cx));
        self.dock.update(cx, |a, cx| {
            a.add_panel_view(view, DockPlacement::Center, None, window, cx)
        });
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let registry = cx.global::<CommandRegistry>();
        let items: Vec<_> = TOOLBAR
            .iter()
            .map(|(id, icon, label, command)| {
                if *id == "|" {
                    return div()
                        .w(tokens::pane::DIVIDER.rems())
                        .h(tokens::toolbar::ITEM_ICON.rems())
                        .bg(p.separator)
                        .mx_1()
                        .into_any_element();
                }
                let action = command
                    .and_then(|c| registry.get(c))
                    .filter(|info| info.is_enabled(cx))
                    .map(|info| info.action);
                ToolbarItem::new(*id, lucide(icon), t!(*label).to_string())
                    .disabled(action.is_none())
                    .when_some(action, |item, make| {
                        item.on_click(move |_, window, cx| window.dispatch_action(make(), cx))
                    })
                    .into_any_element()
            })
            .collect();
        h_flex()
            .h(tokens::toolbar::HEIGHT.rems())
            .px(tokens::toolbar::INSET_X.rems())
            .gap(tokens::toolbar::ITEM_GAP.rems())
            .bg(p.window_bg)
            .border_b_1()
            .border_color(p.separator)
            .children(items)
    }
}

impl gpui_kit::Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let status = navidog_navigator::status_text(cx);
        let root = v_flex()
            .id("main-window")
            .key_context("MainWindow")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.window_bg)
            .text_color(p.label)
            .on_action(cx.listener(|this, a: &OpenObject, window, cx| {
                this.open_object(&a.object, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleNavigator, window, cx| {
                this.toggle_dock(DockPlacement::Left, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleInfoPane, window, cx| {
                this.toggle_dock(DockPlacement::Right, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleToolbar, window, cx| {
                this.toggle(|s| s.show_toolbar = !s.show_toolbar, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleStatusBar, window, cx| {
                this.toggle(|s| s.show_status_bar = !s.show_status_bar, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHideGroups, window, cx| {
                this.toggle(|s| s.view.hide_groups = !s.view.hide_groups, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleFilterOnTop, window, cx| {
                this.toggle(|s| s.view.filter_on_top = !s.view.filter_on_top, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleOnlyActive, window, cx| {
                this.toggle(|s| s.view.only_active = !s.view.only_active, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleShowHidden, window, cx| {
                this.toggle(|s| s.view.show_hidden = !s.view.show_hidden, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleHideObjectGroups, window, cx| {
                this.toggle(
                    |s| s.view.hide_object_groups = !s.view.hide_object_groups,
                    window,
                    cx,
                )
            }))
            .on_action(
                cx.listener(|this, _: &ResetLayout, window, cx| this.reset_layout(window, cx)),
            )
            .on_action(|_: &ToggleFullScreen, window, _| window.toggle_fullscreen())
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                this.save_state(window, cx);
                window.remove_window();
            }))
            .on_action(
                cx.listener(|this, a: &navidog_ui::tabs::PopOutTab, window, cx| {
                    this.pop_out(a.panel, window, cx)
                }),
            );
        let root = navidog_navigator::bind_actions(navidog_connection::bind_actions(root));
        let _ = window;
        root.child(
            TitleBar::new().child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_color(p.label)
                    .child("navidog"),
            ),
        )
        .when(self.state.show_toolbar, |r| {
            r.child(self.render_toolbar(cx))
        })
        .child(div().flex_1().min_h_0().child(self.dock.clone()))
        .when(self.state.show_status_bar, |r| {
            r.child(
                StatusBar::new()
                    .h(tokens::statusbar::HEIGHT.rems())
                    .pl(tokens::statusbar::TEXT_INSET_X.rems())
                    .text_size(tokens::font::SIZE_SMALL.rems())
                    .text_color(p.label_secondary)
                    .left(status),
            )
        })
    }
}
