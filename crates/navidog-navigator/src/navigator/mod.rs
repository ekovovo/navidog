//! Navigator pane (10-shell.md §6, 20-objects.md §2):
//! group > connection > database > (schema) > object category > object.

use std::collections::HashSet;

use gpui_kit::base::{Tree, TreeEntry, TreeEvent, TreeItem, TreeState};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, panel_handle, register_panel,
};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    Icon, IconName, Selectable as _, Sizable as _, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Global, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled as _, StyledImage as _, Subscription, WeakEntity, Window, div, img,
};
use navidog_core::actions::*;
use navidog_core::connection::Connection;
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::store::{ConnectionStore, error_message};
use navidog_core::vgroups::VirtualGroups;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent};
use navidog_ui::assets::{driver_icon, lucide};
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load};
use crate::objects::ObjectsPanel;
use crate::selection::{self, Click};

mod keys;
mod menus;
mod rename;
mod row;
mod tree;

use tree::TreeBuilder;

const PANEL: &str = "NavigatorPanel";
const SEP: char = '\u{1f}';

// Arrow keys are handled here: a row click focuses the panel, not the tree, so the tree's own
// bindings never see them. Shift extends the visible range; left and right fold the focused row.
gpui_kit::actions!(
    nav_keys,
    [
        NavUp,
        NavDown,
        NavLeft,
        NavRight,
        NavExtendUp,
        NavExtendDown
    ]
);

struct ActiveNavigator(WeakEntity<NavigatorPanel>);
impl Global for ActiveNavigator {}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("delete", DeleteSelected, Some("Navigator")),
        gpui_kit::KeyBinding::new("cmd-backspace", DeleteSelected, Some("Navigator")),
        gpui_kit::KeyBinding::new("up", NavUp, Some("Navigator")),
        gpui_kit::KeyBinding::new("down", NavDown, Some("Navigator")),
        gpui_kit::KeyBinding::new("left", NavLeft, Some("Navigator")),
        gpui_kit::KeyBinding::new("right", NavRight, Some("Navigator")),
        gpui_kit::KeyBinding::new("shift-up", NavExtendUp, Some("Navigator")),
        gpui_kit::KeyBinding::new("shift-down", NavExtendDown, Some("Navigator")),
    ]);
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
    /// A virtual group of objects in a category (F-NAV-007)
    VGroup(Scope, ObjectKind, String),
    /// A folder under a table (fields, indexes, …) when "Show objects under tables" is on
    TableFolder(ObjectRef, Child),
    TableItem(ObjectRef, Child, String),
    /// Placeholder child before loading, so the parent shows a disclosure arrow
    Placeholder(String),
}

/// Element folders shown under a table (F-NAV-002).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Child {
    Fields,
    Indexes,
    ForeignKeys,
    Triggers,
}

impl Child {
    const ALL: [Child; 4] = [
        Child::Fields,
        Child::Indexes,
        Child::ForeignKeys,
        Child::Triggers,
    ];

    fn key(self) -> &'static str {
        match self {
            Child::Fields => "fields",
            Child::Indexes => "indexes",
            Child::ForeignKeys => "foreign_keys",
            Child::Triggers => "triggers",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == key)
    }

    fn icon(self) -> &'static str {
        match self {
            Child::Fields => "columns-3",
            Child::Indexes => "list-ordered",
            Child::ForeignKeys => "key-round",
            Child::Triggers => "zap",
        }
    }
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
            Self::VGroup(s, k, g) => format!("v:{}{SEP}{}{SEP}{g}", scope_id(s), k.key()),
            Self::TableFolder(t, c) => {
                format!("t:{}{SEP}{}{SEP}{}", scope_id(&t.scope), t.name, c.key())
            }
            Self::TableItem(t, c, n) => {
                format!(
                    "e:{}{SEP}{}{SEP}{}{SEP}{n}",
                    scope_id(&t.scope),
                    t.name,
                    c.key()
                )
            }
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
            "v" => Self::VGroup(
                parse_scope(parts.get(..3)?)?,
                ObjectKind::from_key(parts.get(3)?)?,
                parts.get(4)?.to_string(),
            ),
            "t" | "e" => {
                let table = ObjectRef {
                    scope: parse_scope(parts.get(..3)?)?,
                    kind: ObjectKind::Table,
                    name: parts.get(3)?.to_string(),
                };
                let child = Child::from_key(parts.get(4)?)?;
                match kind {
                    "t" => Self::TableFolder(table, child),
                    _ => Self::TableItem(table, child, parts.get(5..)?.join(&SEP.to_string())),
                }
            }
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
            Self::Object(o) | Self::TableFolder(o, _) | Self::TableItem(o, _, _) => {
                Selection::Object(o)
            }
            Self::VGroup(s, k, _) => Selection::Category(s, k),
            Self::Placeholder(_) => return None,
        })
    }

    pub(crate) fn from_selection(s: &Selection) -> Option<Self> {
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

/// Quick filter: show only connections of one driver (`None` shows all).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = navigator, no_json)]
struct FilterByType {
    driver: Option<SharedString>,
}

/// Connections being dragged onto a group (or onto the blank area, meaning top level).
/// The ids are resolved when the drop happens: the row's drag value is captured at render,
/// which can be older than the selection.
#[derive(Clone)]
struct DraggedConnections {
    row: SharedString,
    conn: String,
    name: SharedString,
}

impl DraggedConnections {
    fn ids(&self, cx: &App) -> Vec<String> {
        NavigatorPanel::drag_ids(&self.row, &self.conn, cx)
    }
}

impl Render for DraggedConnections {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let n = self.ids(cx).len();
        let label = if n > 1 {
            t!("nav.n_connections", n = n).to_string()
        } else {
            self.name.to_string()
        };
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(p.window_bg)
            .border_1()
            .border_color(p.separator)
            .text_color(p.label)
            .child(label)
    }
}

fn move_connections(ids: &[String], group: Option<String>, window: &mut Window, cx: &mut App) {
    let result = ConnectionStore::global(cx).update(cx, |s, cx| {
        s.update_many(ids, |c| c.group = group.clone(), cx)
    });
    if let Err(error) = result {
        window.push_notification(error_message(&error), cx);
    }
}

/// A group or object being renamed in place.
#[derive(Clone, PartialEq)]
enum RenameTarget {
    Group(String),
    VGroup(String),
    Object(ObjectRef),
}

/// Objects being dragged onto a virtual group (or onto their category, meaning no group).
/// Resolved at drop time, same as [`DraggedConnections`].
#[derive(Clone)]
struct DraggedObjects {
    row: SharedString,
    object: ObjectRef,
    name: SharedString,
}

impl DraggedObjects {
    fn objects(&self, cx: &App) -> Vec<ObjectRef> {
        NavigatorPanel::drag_objects(&self.row, &self.object, cx)
    }
}

impl Render for DraggedObjects {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let n = self.objects(cx).len();
        let label = if n > 1 {
            t!("nav.n_objects", n = n).to_string()
        } else {
            self.name.to_string()
        };
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(p.window_bg)
            .border_1()
            .border_color(p.separator)
            .text_color(p.label)
            .child(label)
    }
}

fn move_objects(objects: &[ObjectRef], group: Option<&str>, cx: &mut App) {
    VirtualGroups::global(cx).update(cx, |g, cx| g.move_objects(objects, group, cx));
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = navigator, no_json)]
struct RenameVirtualGroup {
    group: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = navigator, no_json)]
struct DeleteVirtualGroup {
    group: SharedString,
}

gpui_kit::actions!(navigator, [CopyObjectNames, DeleteSelected]);

struct Rename {
    target: RenameTarget,
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
    view: ViewOptions,
    /// Quick filters next to the search field
    type_filter: Option<SharedString>,
    only_starred: bool,
    /// Catalog revision the tree was built from. See `render`.
    built_revision: u64,
    /// Last time a drag near the pane edge stepped the scroll position.
    autoscroll_at: Option<std::time::Instant>,
    _subscriptions: Vec<Subscription>,
}

/// View > Navigation Pane options, persisted by the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ViewOptions {
    pub filter_on_top: bool,
    /// "Hide Connection Groups" (flat connections)
    pub hide_groups: bool,
    /// Only open connections and their opened (expanded) databases
    pub only_active: bool,
    /// Show system databases and schemas
    pub show_hidden: bool,
    /// Ignore virtual groups of objects (flat object lists)
    pub hide_object_groups: bool,
    /// "Show Table Profiles": a one-line summary under the object list
    pub show_table_profile: bool,
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
                cx.observe_global::<Settings>(|this, cx| this.rebuild(cx)),
                cx.observe(&VirtualGroups::global(cx), |this, _, cx| this.rebuild(cx)),
                cx.observe(&Sessions::global(cx), |this, _, cx| this.rebuild(cx)),
                cx.observe(&Catalog::global(cx), |this, _, cx| {
                    // Runs only while no pane is reading the catalog during
                    // render. After the first frame, `render` schedules this.
                    this.ensure_expanded(cx);
                    this.rebuild(cx);
                }),
                cx.subscribe(&Sessions::global(cx), |this, _, e: &SessionsEvent, cx| {
                    if let SessionsEvent::Opened(id) = e {
                        this.expanded.insert(Node::Connection(id.clone()).id());
                        // Opening drops the catalog. Nodes that were already
                        // expanded do not emit Expanded again, so restart them.
                        this.ensure_expanded(cx);
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
                // Clicks and arrow keys write the workspace selection themselves.
                // Do not copy TreeState's index back: its notify can arrive late and
                // would replace a newer click with the previous row.
                cx.subscribe(&Workspace::global(cx), |this, _, e: &WorkspaceEvent, cx| {
                    if !matches!(e, WorkspaceEvent::SelectionChanged) {
                        return;
                    }
                    this.marked = Self::selected_ids(cx).into_iter().collect();
                    // A selection made elsewhere (objects tab, editor tab) starts a new range
                    let anchor = this
                        .anchor
                        .as_ref()
                        .and_then(|a| Node::parse(a)?.selection());
                    let workspace = Workspace::global(cx).read(cx);
                    if !anchor.is_some_and(|a| workspace.selections().any(|s| *s == a)) {
                        this.anchor = None;
                    }
                    this.reveal_selection(cx);
                    cx.notify();
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
                view: ViewOptions::default(),
                type_filter: None,
                only_starred: false,
                built_revision: 0,
                autoscroll_at: None,
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
            Node::Object(t) | Node::TableFolder(t, _)
                if t.kind == ObjectKind::Table && Settings::global(cx).show_table_children =>
            {
                catalog.update(cx, |cat, cx| {
                    cat.ensure_columns(&t.scope, &t.name, force, cx);
                    cat.ensure_table_children(&t.scope, &t.name, force, cx);
                })
            }
            _ => {}
        }
    }

    /// Restarts loads for nodes that are already expanded. Opening a connection
    /// drops the cache; the tree does not emit Expanded again for those nodes.
    fn ensure_expanded(&mut self, cx: &mut Context<Self>) {
        let nodes: Vec<Node> = self
            .expanded
            .iter()
            .filter_map(|id| Node::parse(id))
            .collect();
        for node in nodes {
            Self::ensure_children(&node, false, cx);
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.built_revision = Catalog::global(cx).read(cx).revision();
        let query = self.filter.read(cx).value().trim().to_lowercase();
        let filtering = !query.is_empty();
        let builder = TreeBuilder {
            panel: self,
            store: ConnectionStore::global(cx).read(cx),
            sessions: Sessions::global(cx).read(cx),
            catalog: Catalog::global(cx).read(cx),
            vgroups: VirtualGroups::global(cx).read(cx),
            query: &query,
            filtering,
            show_children: Settings::global(cx).show_table_children,
            show_objects: Settings::global(cx).show_schema_objects,
        };
        let items = if self.view.hide_groups {
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

    /// Double-click and Return. A closed connection opens; an object opens; anything else
    /// expands or collapses.
    fn toggle(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        match node {
            Node::Connection(c) if !Sessions::global(cx).read(cx).state(c).is_open() => {
                window.dispatch_action(Box::new(OpenConnection), cx);
            }
            Node::Object(o) => {
                window.dispatch_action(Box::new(OpenObject { object: o.clone() }), cx);
            }
            Node::TableItem(object, _, _) => {
                window.dispatch_action(
                    Box::new(DesignTable {
                        object: Some(object.clone()),
                    }),
                    cx,
                );
            }
            Node::Placeholder(_) => {}
            _ => self.disclose(node, window, cx),
        }
    }

    /// The disclosure arrow. Expands or collapses, and opens a closed connection. A table
    /// stays closed as an editor: only the arrow shows the objects under it.
    fn disclose(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        if let Node::Connection(c) = node
            && !Sessions::global(cx).read(cx).state(c).is_open()
        {
            window.dispatch_action(Box::new(OpenConnection), cx);
            return;
        }
        if matches!(node, Node::Placeholder(_)) {
            return;
        }
        let id = node.id();
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
            Self::ensure_children(node, false, cx);
        }
        self.rebuild(cx);
    }

    /// Return on the focused row.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tree.read(cx).selected_item().map(|i| i.id.clone()) else {
            return;
        };
        let Some(node) = Node::parse(&id) else { return };
        self.toggle(&node, window, cx);
    }

    /// Selected connections dragged together with `id` (just `conn` when that row is not selected).
    /// Reads the workspace, not the cached `marked` set, so a drop sees the latest click.
    fn drag_ids(id: &SharedString, conn: &str, cx: &App) -> Vec<String> {
        let selected = Self::selected_ids(cx);
        if !selected.iter().any(|s| s == id) {
            return vec![conn.to_string()];
        }
        selected
            .iter()
            .filter_map(|m| match Node::parse(m) {
                Some(Node::Connection(c)) => Some(c),
                _ => None,
            })
            .collect()
    }

    /// Selected objects of the same category dragged with `o` (just `o` when that row is not selected).
    fn drag_objects(id: &SharedString, o: &ObjectRef, cx: &App) -> Vec<ObjectRef> {
        let selected = Self::selected_ids(cx);
        if !selected.iter().any(|s| s == id) {
            return vec![o.clone()];
        }
        selected
            .iter()
            .filter_map(|m| match Node::parse(m) {
                Some(Node::Object(x)) if x.scope == o.scope && x.kind == o.kind => Some(x),
                _ => None,
            })
            .collect()
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
        ObjectKind::Query => "file-code",
    }
}

/// `Some(true)` scrolls down, `Some(false)` scrolls up. A pointer outside the pane does neither.
fn scroll_edge(
    x: f32,
    y: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    margin: f32,
) -> Option<bool> {
    if x < left || x > right || y < top || y > bottom || bottom <= top {
        return None;
    }
    if y - top < margin {
        Some(false)
    } else if bottom - y < margin {
        Some(true)
    } else {
        None
    }
}

/// The vertical scroll offset after one autoscroll step of `step`, or `None` when already at that
/// end. Offsets run from 0 at the top down to `-max`.
fn step_offset(y: f32, max: f32, step: f32, down: bool) -> Option<f32> {
    let next = if down {
        (y - step).max(-max)
    } else {
        (y + step).min(0.)
    };
    (next != y).then_some(next)
}

impl Render for NavigatorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Other panes read the catalog while they render, so gpui skips its
        // observers and only redraws those panes. This tree is a snapshot, and
        // would keep showing "Loading…" until the next click. Reading the
        // revision subscribes this pane; a change rebuilds on the next frame.
        let revision = Catalog::global(cx).read(cx).revision();
        if revision != self.built_revision {
            cx.defer_in(window, |this, _, cx| {
                this.ensure_expanded(cx);
                this.rebuild(cx);
            });
        }
        self.autoscroll(window, cx);
        let p = palette(cx);
        let scroll = self.tree.read(cx).scroll_handle().clone();
        let this = cx.entity().downgrade();
        // The styled tree wraps every row in an empty context menu, and that popup takes focus.
        // Native menu items dispatch along the focused element, so the row actions never run.
        let tree_view = Tree::new(&self.tree)
            .item(move |ix, entry, state, window, cx| {
                let selected = state.is_selected();
                let item = this
                    .upgrade()
                    .map(|panel| {
                        panel.update(cx, |panel, cx| {
                            panel.render_row(entry, selected, window, cx)
                        })
                    })
                    .unwrap_or_else(|| ListItem::new(ix));
                item.disabled(entry.is_disabled())
                    .selected(selected)
                    .secondary_selected(state.is_right_clicked())
                    .into_any_element()
            })
            .list_style(StyleRefinement::default().flex_grow_1().size_full())
            .relative()
            .size_full();
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
            .on_drop(cx.listener(|_, d: &DraggedConnections, window, cx| {
                move_connections(&d.ids(cx), None, window, cx)
            }))
            .child(
                div()
                    .size_full()
                    .child(tree_view)
                    .vertical_scrollbar(&scroll),
            );
        let filter = if self.view.filter_on_top {
            div().border_b_1().border_color(p.separator).child(filter)
        } else {
            div().border_t_1().border_color(p.separator).child(filter)
        };
        v_flex()
            .id("navigator")
            .key_context("Navigator")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(|this, a: &FilterByType, _, cx| {
                this.type_filter = a.driver.clone();
                this.rebuild(cx);
            }))
            .on_action(cx.listener(|this, a: &RenameVirtualGroup, window, cx| {
                this.start_rename_vgroup(a.group.to_string(), window, cx)
            }))
            .on_action(|a: &DeleteVirtualGroup, _, cx| {
                VirtualGroups::global(cx).update(cx, |g, cx| g.delete_group(&a.group, cx))
            })
            .on_action(|_: &CopyObjectNames, _, cx| {
                let names: Vec<String> = crate::ops::selected_objects(cx)
                    .into_iter()
                    .map(|o| o.name)
                    .collect();
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(names.join("\n")));
            })
            // Delete / ⌘⌫: delete what is selected, after confirming
            .on_action(|_: &DeleteSelected, window, cx| {
                let action: Box<dyn gpui_kit::Action> =
                    match Workspace::global(cx).read(cx).selection() {
                        Selection::Group(_) => Box::new(DeleteGroup),
                        Selection::Connection(_) => Box::new(DeleteConnection),
                        Selection::Object(_) => Box::new(DeleteObjects),
                        _ => return,
                    };
                window.dispatch_action(action, cx);
            })
            .on_action(cx.listener(|this, _: &NavUp, window, cx| {
                this.step_selection(false, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NavDown, window, cx| {
                this.step_selection(true, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NavExtendUp, window, cx| {
                this.step_selection(false, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NavExtendDown, window, cx| {
                this.step_selection(true, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NavLeft, window, cx| this.fold(false, window, cx)))
            .on_action(cx.listener(|this, _: &NavRight, window, cx| this.fold(true, window, cx)))
            // Not a key binding: Enter in the rename field must still commit. Handled only when
            // the panel itself is focused.
            .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, window, cx| {
                if e.keystroke.key != "enter" || e.keystroke.modifiers.modified() {
                    return;
                }
                if this.renaming.is_some() || !this.focus_handle.is_focused(window) {
                    return;
                }
                this.open_selected(window, cx);
            }))
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
                if self.view.filter_on_top {
                    v.child(filter).child(body)
                } else {
                    v.child(body).child(filter)
                }
            })
    }
}

/// The navigator's View > Navigation Pane options (defaults before it exists).
pub(crate) fn view_options(cx: &App) -> ViewOptions {
    cx.try_global::<ActiveNavigator>()
        .and_then(|a| a.0.upgrade())
        .map(|n| n.read(cx).view)
        .unwrap_or_default()
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

/// Group and object commands, wherever they are dispatched from (menus, toolbars, keys).
/// Renaming edits the name in place in the navigator.
pub fn bind_actions<E: gpui_kit::InteractiveElement>(el: E) -> E {
    let el = el
        .on_action(|_: &ShowSnippets, _, cx| {
            let info = crate::info::InfoPanel::shared_entity(cx);
            if let Some(info) = info {
                info.update(cx, |p, cx| p.show_snippets(cx));
            }
        })
        .on_action(|a: &CreateSnippet, window, cx| {
            crate::snippets::open_editor(None, a.code.to_string(), window, cx)
        })
        .on_action(|_: &NewGroup, window, cx| {
            with_navigator(cx, |this, cx| this.new_group(window, cx))
        })
        .on_action(|_: &RenameGroup, window, cx| {
            if let Selection::Group(g) = Workspace::global(cx).read(cx).selection().clone() {
                with_navigator(cx, |this, cx| this.start_rename(g, window, cx));
            }
        })
        .on_action(|_: &RenameObject, window, cx| {
            if let Selection::Object(o) = Workspace::global(cx).read(cx).selection().clone() {
                with_navigator(cx, |this, cx| this.start_rename_object(o, window, cx));
            }
        })
        .on_action(|_: &NewVirtualGroup, window, cx| {
            with_navigator(cx, |this, cx| this.new_virtual_group(window, cx))
        })
        .on_action(|a: &AddToVirtualGroup, _, cx| {
            let objects = crate::ops::selected_objects(cx);
            move_objects(&objects, Some(&a.group), cx);
        })
        .on_action(|_: &RemoveFromVirtualGroup, _, cx| {
            let objects = crate::ops::selected_objects(cx);
            move_objects(&objects, None, cx);
        });
    crate::ops::bind(el)
}

/// Sets the View > Navigation Pane options.
pub fn set_view_options(view: ViewOptions, cx: &mut App) {
    with_navigator(cx, |this, cx| {
        this.view = view;
        this.rebuild(cx);
    });
    crate::objects::refresh(cx);
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
                scope: scope.clone(),
                kind: ObjectKind::Table,
                name: "a:b".into(),
            }),
            Node::VGroup(scope.clone(), ObjectKind::View, "vg-1".into()),
        ];
        let table = ObjectRef {
            scope,
            kind: ObjectKind::Table,
            name: "actor".into(),
        };
        let nodes = nodes.into_iter().chain([
            Node::TableFolder(table.clone(), Child::ForeignKeys),
            Node::TableItem(table, Child::Indexes, "idx:name".into()),
        ]);
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

    #[test]
    fn pointer_near_the_pane_edge_scrolls_that_way() {
        // Viewport x 0..100, y 100..400, one row of margin.
        assert_eq!(
            super::scroll_edge(10., 110., 0., 100., 100., 400., 22.),
            Some(false)
        );
        assert_eq!(
            super::scroll_edge(10., 390., 0., 100., 100., 400., 22.),
            Some(true)
        );
        assert_eq!(
            super::scroll_edge(10., 200., 0., 100., 100., 400., 22.),
            None
        );
        // The object list sits to the right of the tree; a drag there must not scroll it.
        assert_eq!(
            super::scroll_edge(150., 110., 0., 100., 100., 400., 22.),
            None
        );
        assert_eq!(
            super::scroll_edge(10., 50., 0., 100., 100., 400., 22.),
            None
        );
    }

    #[test]
    fn autoscroll_steps_the_offset_and_stops_at_the_ends() {
        // Offsets run from 0 (top) to -max; a step down makes the offset more negative
        assert_eq!(super::step_offset(0., 100., 22., true), Some(-22.));
        assert_eq!(super::step_offset(-90., 100., 22., true), Some(-100.));
        assert_eq!(super::step_offset(-100., 100., 22., true), None);
        assert_eq!(super::step_offset(-10., 100., 22., false), Some(0.));
        assert_eq!(super::step_offset(0., 100., 22., false), None);
        // Nothing to scroll
        assert_eq!(super::step_offset(0., 0., 22., true), None);
    }
}
