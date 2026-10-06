//! 主窗口：标题栏、主工具栏、Dock、状态栏（10-shell.md 第 2 ~ 4、8 节）。

use std::rc::Rc;

use gpui_kit::component::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, panel_handle,
};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div,
};
use navidog_connection::{InfoPanel, NavigatorPanel, ObjectsPanel};
use navidog_core::registry::CommandRegistry;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::Workspace;
use navidog_ui::assets::lucide;
use navidog_ui::skin::ToolbarItem;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::actions::*;
use crate::state::{AppState, Frame};

/// Dock 布局版本；默认布局结构变化时递增，旧布局将被丢弃。
const DOCK_VERSION: usize = 1;

pub struct MainWindow {
    focus_handle: FocusHandle,
    dock: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    state: AppState,
    info_restore: Option<gpui_kit::Pixels>,
    _subscriptions: Vec<Subscription>,
}

/// 主工具栏条目：(id, Lucide 图标, i18n 键, 命令注册表中的命令 ID)。
/// 命令未注册的条目显示为禁用（对应功能在后续里程碑实现）。
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
        navidog_connection::set_view_options(state.filter_on_top, state.hide_groups, cx);

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
        navidog_connection::set_view_options(self.state.filter_on_top, self.state.hide_groups, cx);
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

    /// 信息窗格“最大化/还原”：把右侧 dock 扩到窗口宽度减去导航窗格，再次点击还原。
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
        let status = navidog_connection::status_text(cx);
        let root = v_flex()
            .id("main-window")
            .key_context("MainWindow")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.window_bg)
            .text_color(p.label)
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
                this.toggle(|s| s.hide_groups = !s.hide_groups, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleFilterOnTop, window, cx| {
                this.toggle(|s| s.filter_on_top = !s.filter_on_top, window, cx)
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
            .on_action(|_: &CloseTab, _, _| {
                // M1 只有固定的“对象”选项卡，不可关闭；编辑器选项卡在 M2 加入
            });
        let root = navidog_connection::bind_actions(root);
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
