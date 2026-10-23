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
use navidog_core::actions::{
    DesignTable, InsertText, NewQuery, NewQueryText, NewTable, OpenConnection, OpenConsole,
    OpenExternalQuery, OpenHistoryLog, OpenObject, RunQuery, ToggleFocusMode,
    ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden, ToggleShowTableProfile,
};
use navidog_core::favorites::Favorites;
use navidog_core::object::{ObjectKind, ObjectRef};
use navidog_core::registry::CommandRegistry;
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::sessions::{Sessions, SessionsEvent};
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
            // The Favorites and Open Recent menus list these
            cx.observe(&Favorites::global(cx), |_, _, cx| {
                crate::menus::set_menus(cx)
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
                if navidog_core::settings::Settings::global(cx)
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
            .on_action(cx.listener(|this, a: &DesignTable, window, cx| {
                if let Some(object) = a.object.as_ref().filter(|o| o.kind == ObjectKind::Table) {
                    this.open_editor(object, true, window, cx);
                }
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
                        Box::new(navidog_core::actions::OpenConnectionId {
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
    }
}
