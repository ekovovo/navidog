//! Main window: title bar, main toolbar, dock and status bar (10-shell.md §2–4, §8).

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::dock::{
    BasePanelView, DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin,
    PanelId, PanelInfo, panel_handle,
};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div,
};
use navidog_app_state::actions::{
    DesignField, DesignTable, InsertText, NewQuery, NewQueryText, NewTable, OpenConnection,
    OpenConsole, OpenDefinition, OpenExternalQuery, OpenHistoryLog, OpenObject, OpenTool, RunQuery,
    ToggleFocusMode, ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden,
    ToggleShowTableProfile,
};
use navidog_app_state::favorites::Favorites;
use navidog_app_state::registry::CommandRegistry;
use navidog_app_state::settings::Settings;
use navidog_app_state::store::ConnectionStore;
use navidog_app_state::workspace::{Selection, Workspace};
use navidog_model::object::{ObjectKind, ObjectRef};
use navidog_navigator::{InfoPanel, NavigatorPanel, ObjectsPanel};
use navidog_session::sessions::{Sessions, SessionsEvent};
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
    /// Left-dock width saved when maximizing had to shrink it to fit the
    /// right dock at its minimum width; restored when maximizing ends.
    info_restore_left: Option<gpui_kit::Pixels>,
    /// Viewport width the maximized layout was last fitted to, so render
    /// only refits when the window actually resized.
    info_maximized_viewport: Option<gpui_kit::Pixels>,
    /// The fixed objects tab; editors open in its tab group
    objects_tab: PanelId,
    /// Editors popped out into their own windows, with the object each shows
    popped:
        std::collections::HashMap<PanelId, (gpui_kit::AnyWindowHandle, Option<ObjectRef>, bool)>,
    /// Popped editors closed with their connection. Closing the window must not put them back.
    closing_popped: std::collections::HashSet<PanelId>,
    /// Objects to open once their connection is open (favorites, recent, URIs, startup screen)
    pending: Vec<ObjectRef>,
    /// What focus mode hid, restored when it ends
    focus_mode: Option<FocusRestore>,
    _subscriptions: Vec<Subscription>,
}

struct FocusRestore {
    navigator: bool,
    info: bool,
    fullscreen: bool,
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
        // One owner: the window holds the live state and the global is its
        // published copy for menu callbacks. Every save republishes it.
        cx.set_global(state.clone());
        let (dock, skin) = DockSkin::dock_area("main-dock", Some(DOCK_VERSION), window, cx);
        skin.set_close_button_visible(true, cx);
        let info = InfoPanel::shared(window, cx);
        NavigatorPanel::shared(window, cx);
        let objects_tab = PanelId::from(ObjectsPanel::shared(window, cx).entity_id());

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
                    // The objects tab is not a separate dock pin. Put it back at the
                    // front after a drag. Deferred so this does not re-enter the dock update.
                    cx.defer_in(window, |this, window, cx| this.pin_objects_tab(window, cx));
                }
            }),
            cx.observe(&Workspace::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
            // The status bar's object count changes when a catalog load finishes.
            cx.observe(
                &navidog_navigator::catalog::Catalog::global(cx),
                |_, _, cx| cx.notify(),
            ),
            // The Favorites and Open Recent menus list these
            cx.observe(&Favorites::global(cx), |_, _, cx| {
                crate::menus::set_menus(cx)
            }),
            cx.observe(&navidog_navigator::snippet_form_entity(cx), |_, _, cx| {
                cx.notify()
            }),
            cx.subscribe_in(
                &Sessions::global(cx),
                window,
                |this, _, e: &SessionsEvent, window, cx| match e {
                    SessionsEvent::Opened(id) => {
                        let (ready, rest) = std::mem::take(&mut this.pending)
                            .into_iter()
                            .partition(|o| &o.scope.connection == id);
                        this.pending = rest;
                        for o in ready {
                            this.open_selection(Selection::Object(o), window, cx);
                        }
                    }
                    SessionsEvent::Closed(id) => this.close_connection_editors(id, window, cx),
                    // Editors stay through a reopen and reconnect when it opens
                    SessionsEvent::Reopening(_) | SessionsEvent::Failed(_, _) => {}
                },
            ),
            cx.observe_window_appearance(window, |_, window, cx| {
                if navidog_app_state::settings::Settings::global(cx)
                    .appearance
                    .is_dark()
                    .is_none()
                {
                    crate::apply_theme(Some(window), cx);
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
            info_restore_left: None,
            info_maximized_viewport: None,
            objects_tab,
            popped: Default::default(),
            closing_popped: Default::default(),
            pending: Vec::new(),
            focus_mode: None,
            _subscriptions: subs,
        }
    }

    /// History log (F-QRY-012): one tab, activated if it is open already.
    fn open_history_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for group in navidog_ui::tabs::tab_groups(cx) {
            let found = group
                .read(cx)
                .panels()
                .iter()
                .find(|p| p.panel_name(cx) == navidog_query::HISTORY_PANEL)
                .map(|p| p.panel_id(cx));
            if let Some(id) = found {
                self.dock.update(cx, |a, cx| a.select_panel(id, window, cx));
                return;
            }
        }
        let log = navidog_query::HistoryLog::new(window, cx);
        self.add_editor(panel_handle(log), window, cx);
    }

    /// Auto recovery (F-APP-016): offers editors left unsaved by a crash.
    pub fn check_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let main = cx.entity().downgrade();
        navidog_query::check_recovery(window, cx, move |editor, window, cx| {
            let _ = main.update(cx, |this, cx| {
                this.add_editor(panel_handle(editor), window, cx)
            });
        });
    }

    /// Startup screen (F-APP-012): reopens the configured tabs, connecting as needed.
    pub fn open_startup_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let objects: Vec<ObjectRef> = Settings::global(cx)
            .startup_tabs
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        for o in objects {
            self.open_selection(Selection::Object(o), window, cx);
        }
    }

    /// The object an editor panel shows (its persisted panel state).
    fn panel_object(view: &Arc<dyn BasePanelView>, cx: &App) -> Option<ObjectRef> {
        match view.dump(cx).info {
            PanelInfo::Panel(v) => serde_json::from_value(v.clone())
                .ok()
                .or_else(|| serde_json::from_value(v.get("object")?.clone()).ok())
                .or_else(|| {
                    // Unsaved designers still belong to their connection.
                    if view.panel_name(cx) != "TableDesigner" {
                        return None;
                    }
                    Some(ObjectRef {
                        scope: serde_json::from_value(v.get("scope")?.clone()).ok()?,
                        kind: ObjectKind::Table,
                        name: String::new(),
                    })
                }),
            _ => None,
        }
    }

    fn tab_object(&self, panel: PanelId, cx: &App) -> Option<ObjectRef> {
        self.dock
            .read(cx)
            .panel(panel)
            .and_then(|v| Self::panel_object(v, cx))
    }

    /// The tab already showing `object` in the objects tab's group.
    // ponytail: editors dragged into another tab group are not found and open again
    fn find_tab(&self, object: &ObjectRef, designer: bool, cx: &App) -> Option<PanelId> {
        let group = navidog_ui::tabs::group_of(self.objects_tab, cx)?;
        let group = group.read(cx);
        group
            .panels()
            .iter()
            .find(|p| {
                Self::panel_object(p, cx).as_ref() == Some(object)
                    && (p.panel_name(cx) == "TableDesigner") == designer
            })
            .map(|p| p.panel_id(cx))
    }

    /// Keeps the objects tab at the front of its group. gpui's tab bar scrolls every
    /// tab together, so this does not freeze it while the strip scrolls.
    fn pin_objects_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = navidog_ui::tabs::group_of(self.objects_tab, cx) else {
            return;
        };
        let ix = group
            .read(cx)
            .panels()
            .iter()
            .position(|p| p.panel_id(cx) == self.objects_tab);
        let Some(ix) = ix else {
            return;
        };
        if ix == 0 {
            return;
        }
        let ctx = group.read(cx).context(cx);
        let Some(drag) = ctx.drag_panel(ix, cx) else {
            return;
        };
        ctx.drop_panel(drag, Some(0), false, window, cx);
    }

    /// Closes every editor of `id`, including ones popped into their own window.
    fn close_connection_editors(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        for group in navidog_ui::tabs::tab_groups(cx) {
            let victims: Vec<PanelId> = group
                .read(cx)
                .panels()
                .iter()
                .filter_map(|panel| {
                    let object = Self::panel_object(panel, cx)?;
                    // Saved queries are documents and stay open (30-query-editor.md §2)
                    (object.scope.connection == id && object.kind != ObjectKind::Query)
                        .then(|| panel.panel_id(cx))
                })
                .collect();
            let ctx = group.read(cx).context(cx);
            for panel in victims {
                ctx.close(panel, window, cx);
            }
        }
        let popped: Vec<_> = self
            .popped
            .iter()
            .filter(|(_, (_, object, _))| {
                object
                    .as_ref()
                    .is_some_and(|o| o.scope.connection == id && o.kind != ObjectKind::Query)
            })
            .map(|(panel, (handle, _, _))| (*panel, *handle))
            .collect();
        for (panel, handle) in popped {
            self.popped.remove(&panel);
            self.closing_popped.insert(panel);
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// Selects `sel` and, for an object, opens it; when its connection is closed, opens the
    /// connection first and the object once it is open.
    pub(crate) fn open_selection(
        &mut self,
        sel: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = sel.connection_id().cloned() else {
            return;
        };
        Workspace::global(cx).update(cx, |w, cx| w.select(sel.clone(), cx));
        let open = Sessions::global(cx).read(cx).state(&conn).is_open();
        match sel {
            Selection::Object(o) if open => self.open_object(&o, window, cx),
            Selection::Object(o) => {
                self.pending.push(o);
                window.dispatch_action(Box::new(OpenConnection), cx);
            }
            _ if !open => window.dispatch_action(Box::new(OpenConnection), cx),
            _ => {}
        }
    }

    fn active_tab_object(&self, cx: &App) -> Option<ObjectRef> {
        navidog_ui::tabs::active(cx).and_then(|p| self.tab_object(p, cx))
    }

    /// Focus mode (F-APP-008): only the editors stay, in full screen.
    fn toggle_focus_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let docks = [DockPlacement::Left, DockPlacement::Right];
        match self.focus_mode.take() {
            Some(r) => {
                for (placement, open) in docks.into_iter().zip([r.navigator, r.info]) {
                    if open && !self.dock.read(cx).is_dock_open(placement) {
                        self.toggle_dock(placement, window, cx);
                    }
                }
                if !r.fullscreen && window.is_fullscreen() {
                    window.toggle_fullscreen();
                }
            }
            None => {
                let dock = self.dock.read(cx);
                let r = FocusRestore {
                    navigator: dock.is_dock_open(DockPlacement::Left),
                    info: dock.is_dock_open(DockPlacement::Right),
                    fullscreen: window.is_fullscreen(),
                };
                for placement in docks {
                    if self.dock.read(cx).is_dock_open(placement) {
                        self.toggle_dock(placement, window, cx);
                    }
                }
                if !r.fullscreen {
                    window.toggle_fullscreen();
                }
                self.focus_mode = Some(r);
            }
        }
        cx.notify();
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
        cx.set_global(self.state.clone());
    }

    fn toggle(
        &mut self,
        f: impl FnOnce(&mut AppState),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.state);
        navidog_navigator::set_view_options(self.state.view, cx);
        crate::menus::set_menus(cx);
        self.save_state(window, cx);
        cx.notify();
    }

    /// Toggles a dock open or closed. When the left dock toggles while the
    /// info pane is maximized (`info_restore` is set), the maximized right
    /// dock is resized to the new left-dock width: reopening the navigator
    /// without this would leave left + right wider than the viewport and
    /// push the right dock's edge -- and its maximize button -- off-screen.
    fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock
            .update(cx, |a, cx| a.toggle_dock(placement, window, cx));
        if placement == DockPlacement::Left && self.info_restore.is_some() {
            self.resize_maximized_info(window, cx);
        }
    }

    /// Current width of the left dock: its measured width when open, the
    /// navigator default as a fallback, and zero when closed. Do not assume
    /// the navigator is still at its default width: the user may have
    /// dragged it wider, or a restored layout may have saved a wider size.
    fn left_dock_width(&self, rem: gpui_kit::Pixels, cx: &mut Context<Self>) -> gpui_kit::Pixels {
        let dock = self.dock.read(cx);
        if dock.is_dock_open(DockPlacement::Left) {
            dock.dock_size(DockPlacement::Left)
                .unwrap_or_else(|| tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem))
        } else {
            gpui_kit::px(0.) // raw-px: a closed left dock has zero width
        }
    }

    /// Resizes the maximized right dock to the current left-dock width,
    /// leaving `info_restore` (the pre-maximize right-dock size) untouched.
    /// When the left dock was previously shrunk to fit and now fits at its
    /// original width again (for example the window was widened), the
    /// original width is restored first. When the maximized width would
    /// fall below the dock minimum, the left dock is shrunk instead and
    /// its original width is saved for when maximizing ends.
    fn resize_maximized_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rem = window.rem_size();
        let viewport = window.viewport_size().width;
        let left_open = self.dock.read(cx).is_dock_open(DockPlacement::Left);
        let mut left = self.left_dock_width(rem, cx);
        if let Some(original) =
            widen_restored_left_dock(viewport, self.info_restore_left, rem, left_open)
        {
            left = original;
            self.info_restore_left = None;
            // The dock itself must be resized, not just the local width:
            // fit_maximized_layout returns no left change for a fitting
            // dock, so without this call the shrunk width would persist
            // and the saved original would already be discarded.
            self.dock.update(cx, |a, cx| {
                a.set_dock_size(DockPlacement::Left, original, window, cx)
            });
        }
        let (new_left, right) = fit_maximized_layout(viewport, left, rem, left_open);
        if let Some(new_left) = new_left {
            if self.info_restore_left.is_none() {
                self.info_restore_left = Some(left);
            }
            self.dock.update(cx, |a, cx| {
                a.set_dock_size(DockPlacement::Left, new_left, window, cx)
            });
        }
        self.dock.update(cx, |a, cx| {
            a.set_dock_size(DockPlacement::Right, right, window, cx)
        });
        self.info_maximized_viewport = Some(viewport);
    }

    /// Info pane maximize/restore: widens the right dock to the window width minus the navigator; clicking again restores it.
    /// Restoring also returns a left dock shrunk to fit while maximized to
    /// its original width.
    fn toggle_info_maximized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restore = self.info_restore.take();
        match restore {
            Some(prev) => {
                self.dock.update(cx, |a, cx| {
                    a.set_dock_size(DockPlacement::Right, prev, window, cx)
                });
                if let Some(original) = self.info_restore_left.take() {
                    self.dock.update(cx, |a, cx| {
                        a.set_dock_size(DockPlacement::Left, original, window, cx)
                    });
                }
                self.info_maximized_viewport = None;
            }
            None => {
                self.info_restore = self.dock.read(cx).dock_size(DockPlacement::Right);
                self.info_restore_left = None;
                // Using the default navigator width here made left + right
                // exceed the viewport, pushing the right dock's edge -- and
                // with it the maximize button -- off-screen.
                self.resize_maximized_info(window, cx);
            }
        }
    }

    fn reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::default_layout(&self.dock, window, cx);
        self.save_state(window, cx);
    }

    /// Opens an object in the object pane, switching to its tab (or its own window) if it is already open.
    fn open_object(&mut self, object: &ObjectRef, window: &mut Window, cx: &mut Context<Self>) {
        self.open_editor(object, false, window, cx);
    }

    fn open_editor(
        &mut self,
        object: &ObjectRef,
        designer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        Favorites::global(cx).update(cx, |f, cx| f.opened(object.clone(), cx));
        if let Some(id) = self.find_tab(object, designer, cx) {
            self.dock.update(cx, |a, cx| a.select_panel(id, window, cx));
            return;
        }
        if let Some((w, _, _)) = self
            .popped
            .values()
            .find(|(_, o, design)| o.as_ref() == Some(object) && *design == designer)
        {
            let _ = w.update(cx, |_, window, _| window.activate_window());
            return;
        }
        let panel = if designer {
            panel_handle(navidog_designer::TableDesigner::open(
                object.clone(),
                window,
                cx,
            ))
        } else {
            crate::editors::open(object.clone(), window, cx)
        };
        self.add_editor(panel, window, cx);
    }

    /// Adds an editor to the object pane, or its own window when the preferences ask for that.
    fn add_editor(
        &mut self,
        panel: Arc<dyn BasePanelView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let object = Self::panel_object(&view, cx);
        let designer = view.panel_name(cx) == "TableDesigner";
        let main = cx.entity().downgrade();
        if let Some(w) = crate::popout::open(view, main, window.window_handle(), cx) {
            self.popped.insert(id, (w, object, designer));
        }
    }

    /// Puts a popped-out editor back into the object pane (its window is closing).
    pub(crate) fn restore_tab(
        &mut self,
        view: Arc<dyn BasePanelView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.panel_id(cx);
        if self.closing_popped.remove(&id) {
            return;
        }
        self.popped.remove(&id);
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
                let item = ToolbarItem::new(*id, lucide(icon), t!(*label).to_string())
                    .disabled(action.is_none());
                let item = match tokens::icon_color(icon, cx) {
                    Some(color) => item.icon_color(color),
                    None => item,
                };
                item.when_some(action, |item, make| {
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

/// The open connection's database, for tools opened from the menu rather than a tree node.
fn tool_scope(cx: &App) -> Option<navidog_model::object::Scope> {
    let selection = Workspace::global(cx).read(cx).selection().clone();
    if let Some(scope) = selection.scope() {
        return Some(scope);
    }
    let id = selection.connection_id()?.clone();
    let (file, saved) = {
        let store = ConnectionStore::global(cx).read(cx);
        let connection = store.get(&id)?;
        (
            connection.general.file.is_some(),
            connection.general.database.clone().unwrap_or_default(),
        )
    };
    let database = if saved.is_empty() && file {
        "main".into()
    } else {
        saved
    };
    Some(navidog_model::object::Scope::new(id, database, None))
}

/// Width the right dock gets when the info pane is maximized: whatever the
/// viewport leaves after the left dock, minus one row-height sliver of the
/// centre pane. The caller supplies the left dock's actual width (zero when
/// it is closed), never the navigator's default width: a left dock dragged
/// wider than the default would otherwise make left + right exceed the
/// viewport and push the right dock's edge -- and its maximize button --
/// off-screen. The result is clamped to zero: a viewport narrower than the
/// left dock plus the sliver must not produce a negative dock size.
fn info_maximized_size(
    viewport_width: gpui_kit::Pixels,
    left_dock_width: gpui_kit::Pixels,
    rem_size: gpui_kit::Pixels,
) -> gpui_kit::Pixels {
    let size = viewport_width - left_dock_width - tokens::list::ROW_HEIGHT.to_pixels(rem_size);
    // raw-px: clamping a maximized dock size to a nonnegative width
    if size < gpui_kit::px(0.) {
        gpui_kit::px(0.) // raw-px: clamped maximized width
    } else {
        size
    }
}

/// Left and right dock widths for a maximized info pane that actually fit
/// the viewport, as (left change, right width). `set_dock_size` floors any
/// dock at `PANEL_MIN_SIZE` (100px), so a maximized width clamped to zero
/// would still render at 100px and overflow when the left dock is wide:
/// when the maximized width would fall below that minimum, the left dock
/// is shrunk so the right dock reaches the minimum, but never below the
/// minimum itself. A closed left dock is never resized. If even both
/// docks at their minimum plus the centre sliver exceed the viewport, no
/// layout can fit, and both minimums are returned as the least-overflow
/// result.
/// Saved left-dock width to restore when the window widened enough for
/// it to fit at its original width again: `Some(original)` only when the
/// left dock is open and the maximized right dock would still reach the
/// dock minimum with the original left width. `None` otherwise (closed
/// dock, nothing saved, or still too narrow), so the caller must not
/// resize the left dock and must keep the saved width for later.
fn widen_restored_left_dock(
    viewport_width: gpui_kit::Pixels,
    saved_left_dock_width: Option<gpui_kit::Pixels>,
    rem_size: gpui_kit::Pixels,
    left_dock_open: bool,
) -> Option<gpui_kit::Pixels> {
    if !left_dock_open {
        return None;
    }
    saved_left_dock_width.filter(|&original| {
        info_maximized_size(viewport_width, original, rem_size) >= gpui_kit::base::PANEL_MIN_SIZE
    })
}

fn fit_maximized_layout(
    viewport_width: gpui_kit::Pixels,
    left_dock_width: gpui_kit::Pixels,
    rem_size: gpui_kit::Pixels,
    left_dock_open: bool,
) -> (Option<gpui_kit::Pixels>, gpui_kit::Pixels) {
    let min = gpui_kit::base::PANEL_MIN_SIZE;
    let right = info_maximized_size(viewport_width, left_dock_width, rem_size);
    if right >= min || !left_dock_open {
        (None, right.max(min))
    } else {
        let shrunk_left =
            (viewport_width - min - tokens::list::ROW_HEIGHT.to_pixels(rem_size)).max(min);
        (Some(shrunk_left), min)
    }
}

impl gpui_kit::Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI has no window-resize callback to hook, but render runs when
        // the window resizes. While maximized, refit the docks to the new
        // viewport width; the memo keeps this to one refit per resize so
        // the dock minimum floor cannot make it run every frame.
        if self.info_restore.is_some() {
            let viewport = window.viewport_size().width;
            if self.info_maximized_viewport != Some(viewport) {
                self.resize_maximized_info(window, cx);
            }
        }
        let p = palette(cx);
        let status = navidog_navigator::status_text(cx);
        let root = v_flex()
            .id("main-window")
            .relative()
            .key_context("MainWindow")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.window_bg)
            .text_color(p.label)
            .on_action(cx.listener(|this, a: &OpenObject, window, cx| {
                this.open_object(&a.object, window, cx)
            }))
            .on_action(cx.listener(|this, a: &DesignTable, window, cx| {
                if let Some(object) = a.object.as_ref().filter(|o| o.kind == ObjectKind::Table) {
                    this.open_editor(object, true, window, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &DesignField, window, cx| {
                let editor = navidog_designer::TableDesigner::open(a.object.clone(), window, cx);
                editor.update(cx, |designer, _| {
                    designer.select_column(a.column.to_string())
                });
                this.add_editor(panel_handle(editor), window, cx);
            }))
            .on_action(cx.listener(|this, a: &OpenDefinition, window, cx| {
                let Some(editor) = navidog_designer::ObjectDesigner::blank(
                    a.scope.clone(),
                    &a.kind,
                    a.name.as_ref().map(|name| name.to_string()),
                    window,
                    cx,
                ) else {
                    return;
                };
                this.add_editor(panel_handle(editor), window, cx);
            }))
            .on_action(cx.listener(|this, a: &OpenTool, window, cx| {
                let Some(scope) = a.scope.clone().or_else(|| tool_scope(cx)) else {
                    return;
                };
                let Some(editor) = navidog_designer::ToolPanel::open(
                    scope,
                    &a.tool,
                    a.target.as_ref().map(|name| name.to_string()),
                    window,
                    cx,
                ) else {
                    return;
                };
                this.add_editor(panel_handle(editor), window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewTable, window, cx| {
                let scope = Workspace::global(cx).read(cx).selection().scope();
                if let Some(scope) = scope.filter(|scope| {
                    Sessions::global(cx)
                        .read(cx)
                        .state(&scope.connection)
                        .is_open()
                }) {
                    let editor = navidog_designer::TableDesigner::new(scope, window, cx);
                    this.add_editor(panel_handle(editor), window, cx);
                } else {
                    use gpui_kit::component::WindowExt as _;
                    window.push_notification(rust_i18n::t!("app.choose_database").to_string(), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NewQuery, window, cx| {
                let editor = navidog_query::QueryEditor::untitled(window, cx);
                this.add_editor(panel_handle(editor), window, cx);
            }))
            .on_action(cx.listener(|this, a: &RunQuery, window, cx| {
                this.open_object(&a.object, window, cx);
                if let Some(editor) = navidog_query::editor_for(&a.object, cx) {
                    editor.update(cx, |e, cx| e.run_all(window, cx));
                }
            }))
            .on_action(cx.listener(|this, a: &NewQueryText, window, cx| {
                let editor = navidog_query::QueryEditor::from_sql(
                    navidog_query::Target {
                        connection: Some(a.connection.to_string()),
                        database: Some(a.database.to_string()),
                        schema: a.schema.as_ref().map(|s| s.to_string()),
                    },
                    a.sql.to_string(),
                    window,
                    cx,
                );
                this.add_editor(panel_handle(editor.clone()), window, cx);
                editor.update(cx, |e, cx| e.run_all(window, cx));
            }))
            .on_action(
                cx.listener(|this, _: &OpenHistoryLog, window, cx| {
                    this.open_history_log(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenConsole, window, cx| {
                let selection = Workspace::global(cx).read(cx).selection().clone();
                let Some(conn) = selection.connection_id().cloned() else {
                    return;
                };
                if !Sessions::global(cx).read(cx).state(&conn).is_open() {
                    window.dispatch_action(
                        Box::new(navidog_app_state::actions::OpenConnectionId {
                            id: conn.clone().into(),
                        }),
                        cx,
                    );
                }
                let db = selection.scope().map(|s| s.database);
                let console = navidog_query::Console::new(conn, db, window, cx);
                this.add_editor(panel_handle(console), window, cx);
            }))
            .on_action(cx.listener(|_, _: &OpenExternalQuery, window, cx| {
                let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: None,
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Ok(Some(paths))) = picked.await else {
                        return;
                    };
                    let _ = this.update_in(cx, |this, window, cx| {
                        for path in paths {
                            let editor =
                                navidog_query::QueryEditor::open_external(path, window, cx);
                            this.add_editor(panel_handle(editor), window, cx);
                        }
                    });
                })
                .detach();
            }))
            .on_action(|a: &InsertText, window, cx| {
                if let Some(editor) = navidog_query::active_editor(cx) {
                    editor.update(cx, |e, cx| e.insert_text(&a.text, a.snippet, window, cx));
                }
            })
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
            .on_action(cx.listener(|_, _: &ToggleShowHidden, _, cx| {
                navidog_app_state::settings::Settings::update(cx, |s| {
                    s.show_system_objects = !s.show_system_objects
                });
                crate::menus::set_menus(cx);
                navidog_navigator::ObjectsPanel::drop_hidden_selections(cx);
                navidog_navigator::ObjectsPanel::refresh_count(cx);
                cx.refresh_windows();
            }))
            .on_action(cx.listener(|this, _: &ToggleHideObjectGroups, window, cx| {
                this.toggle(
                    |s| s.view.hide_object_groups = !s.view.hide_object_groups,
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ToggleShowTableProfile, window, cx| {
                this.toggle(
                    |s| s.view.show_table_profile = !s.view.show_table_profile,
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
            )
            .on_action(cx.listener(|this, _: &ToggleFocusMode, window, cx| {
                this.toggle_focus_mode(window, cx)
            }))
            // Favorites (F-APP-011)
            .on_action(
                cx.listener(|this, a: &navidog_ui::tabs::AddToFavorites, window, cx| {
                    if let Some(o) = this.tab_object(a.panel, cx) {
                        crate::opening::add_favorite(o, window, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &AddActiveToFavorites, window, cx| {
                if let Some(o) = this.active_tab_object(cx) {
                    crate::opening::add_favorite(o, window, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &AddFavoriteWithId, _, cx| {
                if let Some(o) = this.active_tab_object(cx) {
                    let name = o.name.clone();
                    Favorites::global(cx).update(cx, |f, cx| f.add(o, name, Some(a.id), cx));
                }
            }))
            .on_action(cx.listener(|this, a: &OpenFavoriteId, window, cx| {
                let o = Favorites::global(cx)
                    .read(cx)
                    .with_id(a.id)
                    .map(|f| f.object.clone());
                if let Some(o) = o {
                    this.open_selection(Selection::Object(o), window, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &OpenFavorite, window, cx| {
                let o = Favorites::global(cx)
                    .read(cx)
                    .favorites()
                    .get(a.ix)
                    .map(|f| f.object.clone());
                if let Some(o) = o {
                    this.open_selection(Selection::Object(o), window, cx);
                }
            }))
            .on_action(|a: &RemoveFavorite, _, cx| {
                Favorites::global(cx).update(cx, |f, cx| f.remove(a.ix, cx))
            })
            .on_action(|_: &ClearFavorites, _, cx| {
                Favorites::global(cx).update(cx, |f, cx| f.clear(cx))
            })
            // Recent objects (F-APP-013)
            .on_action(cx.listener(|this, a: &OpenRecent, window, cx| {
                let o = Favorites::global(cx).read(cx).recent().get(a.ix).cloned();
                if let Some(o) = o {
                    this.open_selection(Selection::Object(o), window, cx);
                }
            }))
            .on_action(|_: &ClearRecent, _, cx| {
                Favorites::global(cx).update(cx, |f, cx| f.clear_recent(cx))
            })
            // Startup screen (F-APP-012)
            .on_action(
                cx.listener(|this, a: &navidog_ui::tabs::AddToStartup, _, cx| {
                    let Some(o) = this
                        .tab_object(a.panel, cx)
                        .and_then(|o| serde_json::to_value(o).ok())
                    else {
                        return;
                    };
                    Settings::update(cx, |s| {
                        if !s.startup_tabs.contains(&o) {
                            s.startup_tabs.push(o);
                        }
                    });
                }),
            )
            // navidog URIs (F-APP-014)
            .on_action(cx.listener(|_, _: &OpenUri, window, cx| {
                let main = cx.entity().downgrade();
                crate::opening::open_uri(window, cx, move |sel, window, cx| {
                    let _ = main.update(cx, |this, cx| this.open_selection(sel, window, cx));
                });
            }));
        let root = navidog_navigator::bind_actions(navidog_connection::bind_actions(root));
        let _ = window;
        root.child(
            TitleBar::new().child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_color(p.label)
                    .child("Navidog"),
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
        .child(navidog_navigator::snippet_form(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sliver(rem_size: gpui_kit::Pixels) -> gpui_kit::Pixels {
        tokens::list::ROW_HEIGHT.to_pixels(rem_size)
    }

    #[test]
    fn maximized_right_dock_fits_the_viewport_for_any_left_dock_width() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let viewport = gpui_kit::px(1240.); // raw-px: test viewport width
        // The invariant that keeps the maximize button on-screen: left dock
        // + maximized right dock + centre sliver == viewport width.
        for left in [
            gpui_kit::px(0.), // raw-px: closed left dock in the test
            tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem_size),
            gpui_kit::px(400.), // raw-px: widened left dock in the test
        ] {
            let right = info_maximized_size(viewport, left, rem_size);
            assert_eq!(
                left + right + sliver(rem_size),
                viewport,
                "left dock {left:?} must not push the right dock off-screen"
            );
            assert!(
                right < viewport,
                "maximized right dock must leave room for the left dock and centre"
            );
        }
    }

    #[test]
    fn maximized_size_never_goes_negative_on_a_narrow_viewport() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        // Viewport narrower than the left dock plus the centre sliver: the
        // unclamped formula would give a negative width, which is not a
        // valid dock size to pass to set_dock_size.
        let viewport = gpui_kit::px(200.); // raw-px: test viewport width
        let left = gpui_kit::px(400.); // raw-px: widened left dock in the test
        assert_eq!(
            info_maximized_size(viewport, left, rem_size),
            gpui_kit::px(0.), // raw-px: clamped maximized width in the test
            "a maximized dock size must be clamped to zero, not negative"
        );
    }

    #[test]
    fn reopening_the_left_dock_recomputes_the_maximized_right_dock() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let viewport = gpui_kit::px(1240.); // raw-px: test viewport width
        // The sequence toggle_dock now runs when the navigator reopens
        // while maximized: closed left dock -> reopened at the default
        // width. The maximized right dock must shrink by exactly the
        // reopened dock's width, or left + right would exceed the viewport.
        let closed = gpui_kit::px(0.); // raw-px: closed left dock in the test
        let reopened = tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem_size);
        let maximized_closed = info_maximized_size(viewport, closed, rem_size);
        let maximized_reopened = info_maximized_size(viewport, reopened, rem_size);
        assert!(
            (f32::from(maximized_closed - maximized_reopened) - f32::from(reopened)).abs() < 0.01,
            "reopening the navigator must shrink the maximized right dock by the left dock's width"
        );
        assert_eq!(
            reopened + maximized_reopened + sliver(rem_size),
            viewport,
            "reopened left dock + maximized right dock + centre sliver must fit the viewport"
        );
    }

    #[test]
    fn maximized_layout_shrinks_a_wide_left_dock_to_the_dock_minimum() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let min = gpui_kit::base::PANEL_MIN_SIZE;
        // 800px navigator reopened in an 800px window: the plain maximized
        // width would be negative, and set_dock_size would floor the right
        // dock at 100px anyway, giving 900px of docks in an 800px window.
        // The left dock must shrink so the right dock reaches its minimum.
        let (new_left, right) =
            fit_maximized_layout(gpui_kit::px(800.), gpui_kit::px(800.), rem_size, true); // raw-px: test widths
        // 800 - 100 minimum - one row-height sliver (22pt, not 22px).
        // raw-px: test widths in the assertion below
        assert_eq!(new_left, Some(gpui_kit::px(800.) - min - sliver(rem_size)));
        assert_eq!(right, min);
        assert_eq!(
            new_left.unwrap() + right + sliver(rem_size),
            gpui_kit::px(800.), // raw-px: test viewport width
            "shrunk left dock + minimum right dock + sliver must fit the viewport"
        );
    }

    #[test]
    fn maximized_layout_keeps_a_fitting_left_dock_unchanged() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let (new_left, right) =
            fit_maximized_layout(gpui_kit::px(1240.), gpui_kit::px(400.), rem_size, true); // raw-px: test widths
        assert_eq!(new_left, None, "a left dock that fits must not be shrunk");
        assert_eq!(
            right,
            // raw-px: test widths in the assertion below
            gpui_kit::px(1240.) - gpui_kit::px(400.) - sliver(rem_size)
        );
    }

    #[test]
    fn maximized_layout_never_shrinks_a_closed_left_dock() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let (new_left, right) =
            fit_maximized_layout(gpui_kit::px(110.), gpui_kit::px(0.), rem_size, false); // raw-px: test widths
        assert_eq!(new_left, None, "a closed left dock must not be resized");
        assert_eq!(right, gpui_kit::base::PANEL_MIN_SIZE);
    }

    #[test]
    fn widened_window_restores_a_shrunk_left_dock_to_its_original_width() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let original = gpui_kit::px(800.); // raw-px: widened left dock in the test
        // A window widened from 800px to 1240px now fits the original
        // 800px left dock with a full-size maximized right dock.
        assert_eq!(
            widen_restored_left_dock(
                gpui_kit::px(1240.), // raw-px: test viewport width
                Some(original),
                rem_size,
                true
            ),
            Some(original),
            "a saved left dock that fits again must be restored, not left shrunk"
        );
        // The same saved width in the original 800px window still does
        // not fit, so it must stay saved (and shrunk) for a later widen.
        assert_eq!(
            widen_restored_left_dock(
                gpui_kit::px(800.), // raw-px: test viewport width
                Some(original),
                rem_size,
                true
            ),
            None,
            "a saved left dock that still does not fit must not be restored yet"
        );
        // A closed left dock is never restored, and nothing saved means
        // nothing to restore.
        assert_eq!(
            widen_restored_left_dock(
                gpui_kit::px(1240.), // raw-px: test viewport width
                Some(original),
                rem_size,
                false
            ),
            None,
            "a closed left dock must not be restored"
        );
        assert_eq!(
            widen_restored_left_dock(
                gpui_kit::px(1240.), // raw-px: test viewport width
                None,
                rem_size,
                true
            ),
            None,
            "with nothing saved there is nothing to restore"
        );
    }

    #[test]
    fn widening_the_left_dock_shrinks_the_maximized_right_dock_by_the_same_amount() {
        let rem_size = gpui_kit::px(16.); // raw-px: test rem size
        let viewport = gpui_kit::px(1240.); // raw-px: test viewport width
        let default_left = tokens::pane::NAVIGATOR_WIDTH.to_pixels(rem_size);
        let wide_left = gpui_kit::px(400.); // raw-px: widened left dock in the test
        let default_right = info_maximized_size(viewport, default_left, rem_size);
        let wide_right = info_maximized_size(viewport, wide_left, rem_size);
        // A left dock dragged wider must shrink the maximized right dock by
        // the same amount; the old code ignored the actual left width and
        // always subtracted the 240px default, so this failed by ~105px.
        // Compare with a 0.01px tolerance: the token conversion goes through
        // f32, so the two differences can differ by a rounding epsilon.
        let shrink = default_right - wide_right;
        let widen = wide_left - default_left;
        assert!(
            (f32::from(shrink) - f32::from(widen)).abs() < 0.01,
            "maximized width must follow the actual left dock width, got shrink {shrink:?} vs widen {widen:?}"
        );
    }
}
