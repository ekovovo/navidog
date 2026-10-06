//! Navigator pane (10-shell.md §6, 20-objects.md §2):
//! group > connection > database > (schema) > object category > object.

use std::collections::HashSet;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Global, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Subscription, WeakEntity, Window, div, img,
};
use navidog_core::actions::*;
use navidog_core::connection::Connection;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent};
use navidog_ui::assets::{driver_icon, lucide};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load};
use crate::selection::{self, Click};

const PANEL: &str = "NavigatorPanel";
const SEP: char = '\u{1f}';

struct ActiveNavigator(WeakEntity<NavigatorPanel>);
impl Global for ActiveNavigator {}

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(NavigatorPanel::shared(window, cx))
    });
}

/// Tree node. IDs are encoded as `prefix:field␟field…`; an empty schema means no schema level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    Group(String),
    Connection(String),
    Database(String, String),
    Schema(Scope),
    Category(Scope, ObjectKind),
    Object(ObjectRef),
    /// Placeholder child before loading, so the parent shows a disclosure arrow
    Placeholder(String),
}

fn scope_id(s: &Scope) -> String {
    format!(
        "{}{SEP}{}{SEP}{}",
        s.connection,
        s.database,
        s.schema.as_deref().unwrap_or("")
    )
}

fn parse_scope(parts: &[&str]) -> Option<Scope> {
    let [c, d, s] = parts else { return None };
    Some(Scope::new(*c, *d, (!s.is_empty()).then(|| s.to_string())))
}

impl Node {
    pub(crate) fn id(&self) -> SharedString {
        match self {
            Self::Group(g) => format!("g:{g}"),
            Self::Connection(c) => format!("c:{c}"),
            Self::Database(c, d) => format!("d:{c}{SEP}{d}"),
            Self::Schema(s) => format!("s:{}", scope_id(s)),
            Self::Category(s, k) => format!("k:{}{SEP}{}", scope_id(s), k.key()),
            Self::Object(o) => format!(
                "o:{}{SEP}{}{SEP}{}",
                scope_id(&o.scope),
                o.kind.key(),
                o.name
            ),
            Self::Placeholder(p) => format!("p:{p}"),
        }
        .into()
    }

    pub(crate) fn parse(id: &str) -> Option<Self> {
        let (kind, rest) = id.split_once(':')?;
        let parts: Vec<&str> = rest.split(SEP).collect();
        Some(match kind {
            "g" => Self::Group(rest.into()),
            "c" => Self::Connection(rest.into()),
            "d" => Self::Database(parts.first()?.to_string(), parts.get(1)?.to_string()),
            "s" => Self::Schema(parse_scope(&parts)?),
            "k" => Self::Category(
                parse_scope(parts.get(..3)?)?,
                ObjectKind::from_key(parts.get(3)?)?,
            ),
            "o" => Self::Object(ObjectRef {
                scope: parse_scope(parts.get(..3)?)?,
                kind: ObjectKind::from_key(parts.get(3)?)?,
                name: parts.get(4..)?.join(&SEP.to_string()),
            }),
            "p" => Self::Placeholder(rest.into()),
            _ => return None,
        })
    }

    fn selection(&self) -> Option<Selection> {
        Some(match self.clone() {
            Self::Group(g) => Selection::Group(g),
            Self::Connection(c) => Selection::Connection(c),
            Self::Database(c, d) => Selection::Database(c, d),
            Self::Schema(s) => Selection::Schema(s),
            Self::Category(s, k) => Selection::Category(s, k),
            Self::Object(o) => Selection::Object(o),
            Self::Placeholder(_) => return None,
        })
    }

    fn from_selection(s: &Selection) -> Option<Self> {
        Some(match s.clone() {
            Selection::Group(g) => Self::Group(g),
            Selection::Connection(c) => Self::Connection(c),
            Selection::Database(c, d) => Self::Database(c, d),
            Selection::Schema(s) => Self::Schema(s),
            Selection::Category(s, k) => Self::Category(s, k),
            Selection::Object(o) => Self::Object(o),
            Selection::None => return None,
        })
    }

    /// Ancestors from the root down to this node (exclusive), expanded to reveal a selection.
    fn ancestors(&self, dialect: Option<Dialect>) -> Vec<Node> {
        let scope_chain = |s: &Scope| {
            let mut v = vec![
                Node::Connection(s.connection.clone()),
                Node::Database(s.connection.clone(), s.database.clone()),
            ];
            if s.schema.is_some() && dialect.is_some_and(|d| d.has_schemas()) {
                v.push(Node::Schema(s.clone()));
            }
            v
        };
        match self {
            Self::Database(c, _) => vec![Node::Connection(c.clone())],
            Self::Schema(s) => {
                scope_chain(&Scope::new(s.connection.clone(), s.database.clone(), None))
            }
            Self::Category(s, _) => scope_chain(s),
            Self::Object(o) => {
                let mut v = scope_chain(&o.scope);
                v.push(Node::Category(o.scope.clone(), o.kind));
                v
            }
            _ => Vec::new(),
        }
    }
}

/// Connections being dragged onto a group (or onto the blank area, meaning top level).
#[derive(Clone)]
struct DraggedConnections {
    ids: Vec<String>,
    label: SharedString,
}

impl Render for DraggedConnections {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(p.window_bg)
            .border_1()
            .border_color(p.separator)
            .text_color(p.label)
            .child(self.label.clone())
    }
}

fn move_connections(ids: &[String], group: Option<String>, cx: &mut App) {
    ConnectionStore::global(cx).update(cx, |s, cx| {
        for id in ids {
            s.move_to_group(id, group.clone(), cx);
        }
    });
}

/// Group being renamed in place.
struct Rename {
    group: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct NavigatorPanel {
    focus_handle: FocusHandle,
    tree: Entity<TreeState>,
    filter: Entity<InputState>,
    expanded: HashSet<SharedString>,
    /// Ids of all selected nodes (the workspace's primary and secondary selections)
    marked: HashSet<SharedString>,
    /// Anchor of ⇧-click range selection
    anchor: Option<SharedString>,
    renaming: Option<Rename>,
    pub filter_on_top: bool,
    pub hide_groups: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for NavigatorPanel {}

impl Focusable for NavigatorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for NavigatorPanel {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for NavigatorPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        t!("nav.refresh").to_string()
    }
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

fn kind_label(kind: ObjectKind) -> String {
    t!(format!("kind.{}", kind.key())).to_string()
}

impl NavigatorPanel {
    /// The main window has a single navigator; the dock reuses this instance when restoring a layout.
    pub fn shared(window: &mut Window, cx: &mut App) -> Entity<Self> {
        match cx
            .try_global::<ActiveNavigator>()
            .and_then(|a| a.0.upgrade())
        {
            Some(panel) => panel,
            None => Self::new(window, cx),
        }
    }

    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let panel = cx.new(|cx: &mut Context<Self>| {
            let tree = cx.new(|cx| TreeState::new(cx));
            let filter =
                cx.new(|cx| InputState::new(window, cx).placeholder(t!("nav.search").to_string()));
            let subs = vec![
                cx.observe(&ConnectionStore::global(cx), |this, _, cx| this.rebuild(cx)),
                cx.observe(&Sessions::global(cx), |this, _, cx| this.rebuild(cx)),
                cx.observe(&Catalog::global(cx), |this, _, cx| this.rebuild(cx)),
                cx.subscribe(&Sessions::global(cx), |this, _, e: &SessionsEvent, cx| {
                    if let SessionsEvent::Opened(id) = e {
                        this.expanded.insert(Node::Connection(id.clone()).id());
                        this.rebuild(cx);
                    }
                }),
                cx.observe(&filter, |this, _, cx| this.rebuild(cx)),
                cx.subscribe(&tree, |this, _, e: &TreeEvent, cx| {
                    match e {
                        TreeEvent::Expanded(id) => {
                            this.expanded.insert(id.clone());
                            if let Some(node) = Node::parse(id) {
                                Self::ensure_children(&node, false, cx);
                            }
                        }
                        TreeEvent::Collapsed(id) => {
                            this.expanded.remove(id);
                        }
                    }
                    cx.notify();
                }),
                // Keyboard navigation moves the tree's own selection; mirror it unless it is
                // already the primary selection (which would drop the secondary ones)
                cx.observe(&tree, |_, tree, cx| {
                    let selection = tree
                        .read(cx)
                        .selected_item()
                        .and_then(|i| Node::parse(&i.id))
                        .and_then(|n| n.selection());
                    if let Some(sel) = selection
                        && Workspace::global(cx).read(cx).selection() != &sel
                    {
                        Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
                    }
                }),
                cx.subscribe(&Workspace::global(cx), |this, _, e: &WorkspaceEvent, cx| {
                    let WorkspaceEvent::SelectionChanged = e;
                    this.marked = Self::selected_ids(cx).into_iter().collect();
                    this.reveal_selection(cx);
                }),
            ];
            let mut this = Self {
                focus_handle: cx.focus_handle(),
                tree,
                filter,
                expanded: HashSet::new(),
                marked: HashSet::new(),
                anchor: None,
                renaming: None,
                filter_on_top: false,
                hide_groups: false,
                _subscriptions: subs,
            };
            this.rebuild(cx);
            this
        });
        cx.set_global(ActiveNavigator(panel.downgrade()));
        panel
    }

    /// Loads children on demand when a node expands.
    fn ensure_children(node: &Node, force: bool, cx: &mut App) {
        let catalog = Catalog::global(cx);
        match node {
            Node::Database(c, d) => {
                if Sessions::global(cx)
                    .read(cx)
                    .dialect(c)
                    .is_some_and(|d| d.has_schemas())
                {
                    catalog.update(cx, |cat, cx| cat.ensure_schemas(c, d, force, cx));
                }
            }
            Node::Category(scope, kind) => {
                catalog.update(cx, |cat, cx| cat.ensure_objects(scope, *kind, force, cx))
            }
            _ => {}
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().trim().to_lowercase();
        let filtering = !query.is_empty();
        let builder = TreeBuilder {
            panel: self,
            store: ConnectionStore::global(cx).read(cx),
            sessions: Sessions::global(cx).read(cx),
            catalog: Catalog::global(cx).read(cx),
            query: &query,
            filtering,
        };
        let items = if self.hide_groups {
            let mut conns: Vec<_> = builder
                .store
                .connections()
                .iter()
                .filter_map(|c| builder.connection(c))
                .collect();
            conns.sort_by(|a, b| a.label.cmp(&b.label));
            conns
        } else {
            builder.groups(None)
        };
        self.tree.update(cx, |t, cx| t.set_items(items, cx));
        self.sync_selection(cx);
        cx.notify();
    }

    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let target =
            Node::from_selection(Workspace::global(cx).read(cx).selection()).map(|n| n.id());
        self.tree.update(cx, |t, cx| {
            let current = t.selected_item().map(|i| i.id.clone());
            if current == target {
                return;
            }
            let ix = target.as_ref().and_then(|id| t.index_of(id));
            t.set_selected_index(ix, cx);
        });
    }

    /// When the selection comes from another panel (objects tab, toolbar), expands and loads the
    /// ancestors so the selection is visible in the tree.
    fn reveal_selection(&mut self, cx: &mut Context<Self>) {
        let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection()) else {
            self.sync_selection(cx);
            return;
        };
        let dialect = Workspace::global(cx)
            .read(cx)
            .selection()
            .connection_id()
            .and_then(|c| Sessions::global(cx).read(cx).dialect(c));
        let mut changed = false;
        for a in node.ancestors(dialect) {
            if self.expanded.insert(a.id()) {
                changed = true;
                Self::ensure_children(&a, false, cx);
            }
        }
        if changed {
            self.rebuild(cx);
        } else {
            self.sync_selection(cx);
        }
    }

    fn toggle(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        match node {
            Node::Connection(c) if !Sessions::global(cx).read(cx).state(c).is_open() => {
                window.dispatch_action(Box::new(OpenConnection), cx);
                return;
            }
            Node::Object(o) => {
                window.dispatch_action(Box::new(OpenObject { object: o.clone() }), cx);
                return;
            }
            Node::Placeholder(_) => return,
            _ => {}
        }
        let id = node.id();
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
            Self::ensure_children(node, false, cx);
        }
        self.rebuild(cx);
    }

    fn select(node: &Node, cx: &mut App) {
        if let Some(sel) = node.selection() {
            Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
        }
    }

    fn selected_ids(cx: &App) -> Vec<SharedString> {
        Workspace::global(cx)
            .read(cx)
            .selections()
            .filter_map(Node::from_selection)
            .map(|n| n.id())
            .collect()
    }

    fn click_select(&mut self, id: &SharedString, cmd: bool, shift: bool, cx: &mut Context<Self>) {
        let selected = Self::selected_ids(cx);
        let tree = self.tree.read(cx);
        let visible: Vec<SharedString> = (0..)
            .map_while(|ix| tree.entry(ix).map(|e| e.item().id.clone()))
            .collect();
        let click = Click {
            selected: &selected,
            anchor: self.anchor.as_ref(),
            visible: &visible,
            cmd,
            shift,
        };
        let (ids, anchor) = selection::click(click, id);
        self.anchor = anchor;
        let mut selections = ids.iter().filter_map(|id| Node::parse(id)?.selection());
        let primary = selections.next().unwrap_or_default();
        let also = selections.collect();
        Workspace::global(cx).update(cx, |w, cx| w.select_many(primary, also, cx));
    }

    /// Selected connections that are dragged together with `id` (just `id` when it is not selected).
    fn drag_ids(&self, id: &SharedString, conn: &str) -> Vec<String> {
        if !self.marked.contains(id) {
            return vec![conn.to_string()];
        }
        self.marked
            .iter()
            .filter_map(|m| match Node::parse(m) {
                Some(Node::Connection(c)) => Some(c),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match Workspace::global(cx).read(cx).selection() {
            Selection::Group(g) => Some(g.clone()),
            _ => None,
        };
        if let Some(p) = &parent {
            self.expanded.insert(Node::Group(p.clone()).id());
        }
        let name = t!("nav.new_group_default").to_string();
        let id = ConnectionStore::global(cx).update(cx, |s, cx| s.add_group(name, parent, cx));
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Group(id.clone()), cx));
        self.start_rename(id, window, cx);
    }

    pub(crate) fn start_rename(
        &mut self,
        group: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = ConnectionStore::global(cx);
        let Some(name) = store
            .read(cx)
            .groups()
            .iter()
            .find(|g| g.id == group)
            .map(|g| g.name.clone())
        else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.finish_rename(true, window, cx);
                }
            });
        self.renaming = Some(Rename {
            group,
            input,
            _subscription: subscription,
        });
        self.refresh_rows(cx);
    }

    /// Rows are rendered by the tree's own view, which only re-renders when it is notified.
    fn refresh_rows(&self, cx: &mut Context<Self>) {
        self.tree.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.renaming.take() else {
            return;
        };
        let name = r.input.read(cx).value().trim().to_string();
        if commit && !name.is_empty() {
            ConnectionStore::global(cx).update(cx, |s, cx| s.rename_group(&r.group, name, cx));
        }
        window.focus(&self.focus_handle, cx);
        self.refresh_rows(cx);
    }

    fn context_menu(node: Option<&Node>, cx: &App) -> NativeMenu {
        let new_conn =
            navidog_db::DriverRegistry::all(cx)
                .into_iter()
                .fold(NativeMenu::new(), |m, d| {
                    let info = d.info();
                    m.menu(
                        info.name.clone(),
                        Box::new(NewConnectionOf {
                            driver: info.id.clone(),
                        }),
                    )
                });
        let refresh = |m: NativeMenu| {
            m.separator()
                .menu(t!("nav.refresh").to_string(), Box::new(Refresh))
        };
        let colors = navidog_ui::tokens::CONNECTION_COLORS.iter().fold(
            NativeMenu::new()
                .menu(
                    t!("nav.color_none").to_string(),
                    Box::new(SetColor { color: None }),
                )
                .separator(),
            |m, c| {
                m.menu(
                    t!(format!("nav.color_{c}")).to_string(),
                    Box::new(SetColor {
                        color: Some((*c).into()),
                    }),
                )
            },
        );
        match node {
            None => refresh(
                NativeMenu::new()
                    .submenu(t!("nav.new_connection").to_string(), new_conn)
                    .menu(t!("nav.new_group").to_string(), Box::new(NewGroup)),
            ),
            Some(Node::Group(_)) => refresh(
                NativeMenu::new()
                    .submenu(t!("nav.new_connection").to_string(), new_conn)
                    .menu(t!("nav.new_group").to_string(), Box::new(NewGroup))
                    .menu(t!("nav.rename_group").to_string(), Box::new(RenameGroup))
                    .menu(t!("nav.delete_group").to_string(), Box::new(DeleteGroup)),
            ),
            Some(Node::Connection(id)) => {
                let open = Sessions::global(cx).read(cx).state(id).is_open();
                let groups = ConnectionStore::global(cx).read(cx).groups().to_vec();
                let move_menu = groups.iter().fold(
                    NativeMenu::new()
                        .menu(
                            t!("nav.top_level").to_string(),
                            Box::new(MoveToGroup { group: None }),
                        )
                        .separator(),
                    |m, g| {
                        m.menu(
                            g.name.clone(),
                            Box::new(MoveToGroup {
                                group: Some(g.id.clone().into()),
                            }),
                        )
                    },
                );
                let menu = if open {
                    NativeMenu::new().menu(
                        t!("nav.close_connection").to_string(),
                        Box::new(CloseConnection),
                    )
                } else {
                    NativeMenu::new().menu(
                        t!("nav.open_connection").to_string(),
                        Box::new(OpenConnection),
                    )
                };
                let menu = menu.separator().menu(
                    t!("nav.edit_connection").to_string(),
                    Box::new(EditConnection),
                );
                let menu = if open {
                    menu.menu_with_disabled(
                        t!("nav.new_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .menu_with_disabled(t!("nav.new_query").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(
                        t!("nav.console").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                } else {
                    menu.submenu(t!("nav.new_connection").to_string(), new_conn)
                        .menu(
                            t!("nav.duplicate_connection").to_string(),
                            Box::new(DuplicateConnection),
                        )
                        .menu(
                            t!("nav.delete_connection").to_string(),
                            Box::new(DeleteConnection),
                        )
                };
                let mut menu = menu
                    .separator()
                    .submenu(t!("nav.color").to_string(), colors);
                if !open {
                    menu = menu.submenu(t!("nav.move_to_group").to_string(), move_menu);
                }
                refresh(menu)
            }
            Some(Node::Database(..)) => refresh(
                NativeMenu::new()
                    .menu(t!("nav.open_database").to_string(), Box::new(OpenDatabase))
                    .menu(
                        t!("nav.close_database").to_string(),
                        Box::new(CloseDatabase),
                    )
                    .separator()
                    .menu_with_disabled(t!("nav.new_query").to_string(), true, Box::new(Refresh)),
            ),
            Some(Node::Object(o)) => refresh(NativeMenu::new().menu(
                t!("obj.open").to_string(),
                Box::new(OpenObject { object: o.clone() }),
            )),
            Some(_) => NativeMenu::new().menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
        }
    }

    fn row_icon(
        &self,
        node: &Option<Node>,
        entry: &TreeEntry,
        cx: &App,
    ) -> (AnyElement, Option<Hsla>) {
        let p = palette(cx);
        let size = tokens::navigator::ROW_ICON.rems();
        let tinted = |name: &str| {
            lucide(name)
                .size(size)
                .text_color(p.label_secondary)
                .into_any_element()
        };
        match node {
            Some(Node::Group(_)) => (
                Icon::new(if entry.is_expanded() {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .size(size)
                .text_color(p.label_secondary)
                .into_any_element(),
                None,
            ),
            Some(Node::Connection(c)) => {
                let store = ConnectionStore::global(cx);
                let conn = store.read(cx).get(c);
                let driver = conn.map(|c| c.driver.clone()).unwrap_or_default();
                let open = Sessions::global(cx).read(cx).state(c).is_open();
                let color = conn
                    .and_then(|c| c.color.as_deref())
                    .and_then(|n| tokens::connection_color(n, cx));
                (
                    img(driver_icon(&driver))
                        .size(size)
                        .when(!open, |i| i.grayscale(true).opacity(0.6))
                        .into_any_element(),
                    color,
                )
            }
            Some(Node::Database(..)) => (tinted("database"), None),
            Some(Node::Schema(_)) => (tinted("layers"), None),
            Some(Node::Category(_, k)) | Some(Node::Object(ObjectRef { kind: k, .. })) => {
                (tinted(kind_icon(*k)), None)
            }
            _ => (div().into_any_element(), None),
        }
    }

    fn render_row(
        &self,
        entry: &TreeEntry,
        selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let p = palette(cx);
        let id = entry.item().id.clone();
        let node = Node::parse(&id);
        let selected = selected || self.marked.contains(&id);
        let focused = self.focus_handle.contains_focused(window, cx);
        let (icon, color) = self.row_icon(&node, entry, cx);
        let placeholder = matches!(node, Some(Node::Placeholder(_)));
        let chevron = entry.is_folder().then(|| {
            Icon::new(if entry.is_expanded() {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(tokens::navigator::ROW_ICON.rems())
            .text_color(if selected && focused {
                p.selection_text
            } else {
                p.label_tertiary
            })
        });
        let fg = if placeholder {
            p.label_tertiary
        } else if selected && focused {
            p.selection_text
        } else {
            p.label
        };
        let bg = selected.then_some(if focused {
            p.selection
        } else {
            p.selection_inactive
        });
        let right_node = node.clone();
        let click_node = node.clone();
        let label = match (&node, &self.renaming) {
            (Some(Node::Group(g)), Some(r)) if *g == r.group => {
                Input::new(&r.input).xsmall().into_any_element()
            }
            _ => entry.item().label.clone().into_any_element(),
        };
        let row = ListItem::new(ElementId::Name(id.clone()))
            .h(tokens::navigator::ROW_HEIGHT.rems())
            .pl(tokens::navigator::ROW_INSET_X.rems()
                + tokens::navigator::ROW_INDENT.rems() * (entry.depth() as f32))
            .when_some(bg, |l, bg| l.bg(bg))
            .text_color(fg)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .w(tokens::navigator::ROW_ICON.rems())
                            .children(chevron),
                    )
                    .child(icon)
                    .child(div().flex_1().min_w_0().overflow_hidden().child(label))
                    .when_some(color, |r, c| {
                        r.child(div().size_2().rounded_full().bg(c).mr_2())
                    }),
            );
        let row = match &node {
            Some(Node::Connection(c)) => {
                let ids = self.drag_ids(&id, c);
                let label = if ids.len() > 1 {
                    t!("nav.n_connections", n = ids.len()).to_string().into()
                } else {
                    entry.item().label.clone()
                };
                row.on_drag(DraggedConnections { ids, label }, |d, _, _, cx| {
                    cx.new(|_| d.clone())
                })
            }
            _ => row,
        };
        // Dropping onto a group moves into it; onto a connection, into that connection's group
        let target = match &node {
            Some(Node::Group(g)) => Some(Some(g.clone())),
            Some(Node::Connection(c)) => Some(
                ConnectionStore::global(cx)
                    .read(cx)
                    .get(c)
                    .and_then(|c| c.group.clone()),
            ),
            _ => None,
        };
        let row = match target {
            Some(group) => row
                .drag_over::<DraggedConnections>(move |s, _, _, _| s.bg(p.fill))
                .on_drop(cx.listener(move |_, d: &DraggedConnections, _, cx| {
                    cx.stop_propagation();
                    move_connections(&d.ids, group.clone(), cx);
                })),
            None => row,
        };
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(n) = &right_node {
                    Self::select(n, cx);
                }
                window.focus(&this.focus_handle, cx);
                Self::context_menu(right_node.as_ref(), cx).show(e.position, window, cx);
            }),
        )
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            window.focus(&this.focus_handle, cx);
            let Some(n) = &click_node else { return };
            if e.click_count() >= 2 {
                Self::select(n, cx);
                this.toggle(n, window, cx);
            } else {
                let m = e.modifiers();
                this.click_select(&n.id(), m.platform, m.shift, cx);
            }
        }))
    }

    fn render_filter(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        h_flex()
            .h(tokens::navigator::FILTER_HEIGHT.rems())
            .px(tokens::navigator::FILTER_INSET_X.rems())
            .gap(tokens::navigator::FILTER_ICON_GAP.rems())
            .child(
                Input::new(&self.filter)
                    .h(tokens::navigator::FILTER_SEARCH_HEIGHT.rems())
                    .prefix(Icon::new(IconName::Search).small())
                    .cleanable(true)
                    .flex_1(),
            )
            .children(["database", "circle-check", "star"].map(|i| {
                // Quick filters are not implemented yet; shown disabled
                lucide(i)
                    .size(tokens::navigator::FILTER_ICON.rems())
                    .text_color(p.disabled_text)
            }))
            .into_any_element()
    }

    /// Refresh reloads metadata of the selected scope; connection-level refresh is handled by the connection commands.
    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        if let Some(scope) = selection.scope() {
            Catalog::global(cx).update(cx, |cat, cx| cat.refresh_scope(&scope, cx));
        } else if let Some(c) = selection.connection_id() {
            let c = c.clone();
            Catalog::global(cx).update(cx, |cat, cx| cat.invalidate_connection(&c, cx));
        }
        cx.propagate();
    }

    fn on_open_database(&mut self, _: &OpenDatabase, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection())
            && !self.expanded.contains(&node.id())
        {
            self.toggle(&node, window, cx);
        }
    }

    fn on_close_database(&mut self, _: &CloseDatabase, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection()) {
            self.expanded.remove(&node.id());
            self.rebuild(cx);
        }
    }
}

pub(crate) fn kind_icon(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "table",
        ObjectKind::View | ObjectKind::MaterializedView => "eye",
        ObjectKind::Function => "square-function",
        ObjectKind::Event => "calendar-clock",
        ObjectKind::Trigger => "zap",
        ObjectKind::Index => "key-round",
        ObjectKind::Sequence => "list-ordered",
    }
}

/// Builds the tree items (read-only access to the globals).
struct TreeBuilder<'a> {
    panel: &'a NavigatorPanel,
    store: &'a ConnectionStore,
    sessions: &'a Sessions,
    catalog: &'a Catalog,
    query: &'a str,
    filtering: bool,
}

impl TreeBuilder<'_> {
    fn matches(&self, label: &str) -> bool {
        !self.filtering || label.to_lowercase().contains(self.query)
    }

    fn expanded(&self, node: &Node) -> bool {
        self.filtering || self.panel.expanded.contains(&node.id())
    }

    fn item(
        &self,
        node: Node,
        label: impl Into<SharedString>,
        children: Vec<TreeItem>,
    ) -> TreeItem {
        let expanded = self.expanded(&node);
        TreeItem::new(node.id(), label)
            .children(children)
            .expanded(expanded)
    }

    fn placeholder(parent: &Node, label: String) -> Vec<TreeItem> {
        vec![TreeItem::new(
            Node::Placeholder(parent.id().to_string()).id(),
            label,
        )]
    }

    fn groups(&self, parent: Option<&str>) -> Vec<TreeItem> {
        let mut items = Vec::new();
        for g in self
            .store
            .groups()
            .iter()
            .filter(|g| g.parent.as_deref() == parent)
        {
            let children = self.groups(Some(&g.id));
            if children.is_empty() && !self.matches(&g.name) {
                continue;
            }
            items.push(self.item(Node::Group(g.id.clone()), g.name.clone(), children));
        }
        items.extend(
            self.store
                .connections()
                .iter()
                .filter(|c| c.group.as_deref() == parent)
                .filter_map(|c| self.connection(c)),
        );
        items
    }

    fn connection(&self, c: &Connection) -> Option<TreeItem> {
        let dbs: Vec<TreeItem> = match self.sessions.state(&c.id) {
            ConnState::Open { databases, .. } => {
                let dialect = self.sessions.dialect(&c.id);
                databases
                    .iter()
                    .filter_map(|d| self.database(&c.id, d, dialect, &c.name))
                    .collect()
            }
            _ => Vec::new(),
        };
        if !self.matches(&c.name) && dbs.is_empty() {
            return None;
        }
        Some(self.item(Node::Connection(c.id.clone()), c.name.clone(), dbs))
    }

    fn database(
        &self,
        conn: &str,
        db: &str,
        dialect: Option<Dialect>,
        conn_name: &str,
    ) -> Option<TreeItem> {
        let node = Node::Database(conn.to_string(), db.to_string());
        let Some(dialect) = dialect else {
            return Some(self.item(node, db.to_string(), Vec::new()));
        };
        let children = if dialect.has_schemas() {
            match self.catalog.schemas(conn, db) {
                Some(Load::Ready(list)) => list
                    .iter()
                    .map(|s| {
                        let scope = Scope::new(conn, db, Some(s.clone()));
                        self.item(
                            Node::Schema(scope.clone()),
                            s.clone(),
                            self.categories(&scope, dialect),
                        )
                    })
                    .collect(),
                Some(Load::Failed(e)) => Self::placeholder(&node, e.to_string()),
                _ => Self::placeholder(&node, t!("nav.loading").to_string()),
            }
        } else {
            self.categories(&Scope::new(conn, db, None), dialect)
        };
        let any_match = self.matches(db)
            || self.matches(conn_name)
            || children.iter().any(|c| !c.children.is_empty());
        any_match.then(|| self.item(node, db.to_string(), children))
    }

    fn categories(&self, scope: &Scope, dialect: Dialect) -> Vec<TreeItem> {
        dialect
            .object_kinds()
            .iter()
            .filter_map(|&kind| {
                let node = Node::Category(scope.clone(), kind);
                let children = match self.catalog.objects(scope, kind) {
                    Some(Load::Ready(list)) => list
                        .iter()
                        .filter(|o| self.matches(&o.name))
                        .map(|o| {
                            let obj = ObjectRef {
                                scope: scope.clone(),
                                kind,
                                name: o.name.clone(),
                            };
                            TreeItem::new(Node::Object(obj).id(), o.name.clone())
                        })
                        .collect(),
                    Some(Load::Failed(e)) => Self::placeholder(&node, e.to_string()),
                    _ if self.filtering => return None,
                    _ => Self::placeholder(&node, t!("nav.loading").to_string()),
                };
                if self.filtering && children.is_empty() {
                    return None;
                }
                Some(self.item(node, kind_label(kind), children))
            })
            .collect()
    }
}

impl Render for NavigatorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let this = cx.entity().downgrade();
        let tree_view = tree(&self.tree, move |ix, entry, selected, window, cx| {
            this.upgrade()
                .map(|panel| {
                    panel.update(cx, |panel, cx| {
                        panel.render_row(entry, selected, window, cx)
                    })
                })
                .unwrap_or_else(|| ListItem::new(ix))
        });
        let filter = self.render_filter(cx);
        // Rows stop propagation of their right-click and drop, so these only fire on the blank area
        let body = div()
            .id("nav-tree")
            .flex_1()
            .min_h_0()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    Workspace::global(cx).update(cx, |w, cx| w.select(Selection::None, cx));
                    window.focus(&this.focus_handle, cx);
                    Self::context_menu(None, cx).show(e.position, window, cx);
                }),
            )
            .on_drop(
                cx.listener(|_, d: &DraggedConnections, _, cx| move_connections(&d.ids, None, cx)),
            )
            .child(tree_view);
        let filter = if self.filter_on_top {
            div().border_b_1().border_color(p.separator).child(filter)
        } else {
            div().border_t_1().border_color(p.separator).child(filter)
        };
        v_flex()
            .id("navigator")
            .key_context("Navigator")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.renaming.is_some() {
                    this.finish_rename(false, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(Self::on_open_database))
            .on_action(cx.listener(Self::on_close_database))
            .size_full()
            .bg(p.control_bg)
            .map(|v| {
                if self.filter_on_top {
                    v.child(filter).child(body)
                } else {
                    v.child(body).child(filter)
                }
            })
    }
}

fn with_navigator(cx: &mut App, f: impl FnOnce(&mut NavigatorPanel, &mut Context<NavigatorPanel>)) {
    let Some(panel) = cx
        .try_global::<ActiveNavigator>()
        .and_then(|a| a.0.upgrade())
    else {
        return;
    };
    panel.update(cx, f);
}

/// Group commands edit the name in place in the navigator, wherever they are dispatched from.
pub fn bind_actions<E: gpui_kit::InteractiveElement>(el: E) -> E {
    el.on_action(|_: &NewGroup, window, cx| {
        with_navigator(cx, |this, cx| this.new_group(window, cx))
    })
    .on_action(|_: &RenameGroup, window, cx| {
        if let Selection::Group(g) = Workspace::global(cx).read(cx).selection().clone() {
            with_navigator(cx, |this, cx| this.start_rename(g, window, cx));
        }
    })
}

/// Toggles "Show filter at top" and "Hide connection groups" (View > Navigation Pane).
pub fn set_view_options(filter_on_top: bool, hide_groups: bool, cx: &mut App) {
    with_navigator(cx, |this, cx| {
        this.filter_on_top = filter_on_top;
        this.hide_groups = hide_groups;
        this.rebuild(cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_round_trip() {
        let scope = Scope::new("c-1", "app", Some("public".into()));
        let nodes = [
            Node::Group("g-1".into()),
            Node::Connection("c-1".into()),
            Node::Database("c-1".into(), "app".into()),
            Node::Schema(scope.clone()),
            Node::Category(Scope::new("c-1", "main", None), ObjectKind::View),
            Node::Object(ObjectRef {
                scope,
                kind: ObjectKind::Table,
                name: "a:b".into(),
            }),
        ];
        for n in nodes {
            assert_eq!(Node::parse(&n.id()), Some(n.clone()), "{}", n.id());
        }
    }

    #[test]
    fn ancestors_skip_schema_level_without_schemas() {
        let obj = Node::Object(ObjectRef {
            scope: Scope::new("c", "db", None),
            kind: ObjectKind::Table,
            name: "t".into(),
        });
        let ids: Vec<_> = obj
            .ancestors(Some(Dialect::MySql))
            .iter()
            .map(|n| n.id().to_string())
            .collect();
        assert_eq!(ids, ["c:c", "d:c\u{1f}db", "k:c\u{1f}db\u{1f}\u{1f}table"]);
        let pg = Node::Category(
            Scope::new("c", "db", Some("public".into())),
            ObjectKind::Table,
        );
        assert_eq!(pg.ancestors(Some(Dialect::Postgres)).len(), 3);
    }
}
