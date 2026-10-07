//! Navigator pane (10-shell.md §6, 20-objects.md §2):
//! group > connection > database > (schema) > object category > object.

use std::collections::HashSet;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
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
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_core::vgroups::VirtualGroups;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions, SessionsEvent};
use navidog_ui::assets::{driver_icon, lucide};
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load};
use crate::selection::{self, Click};

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

fn move_connections(ids: &[String], group: Option<String>, cx: &mut App) {
    ConnectionStore::global(cx).update(cx, |s, cx| {
        for id in ids {
            s.move_to_group(id, group.clone(), cx);
        }
    });
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
                    let WorkspaceEvent::SelectionChanged = e;
                    this.marked = Self::selected_ids(cx).into_iter().collect();
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

    /// Moves the focused row. `extend` keeps the anchor and selects every visible row between them.
    fn step_selection(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let (len, ix) = {
            let tree = self.tree.read(cx);
            let len = (0..).position(|i| tree.entry(i).is_none()).unwrap_or(0);
            (len, tree.selected_index())
        };
        if len == 0 {
            return;
        }
        let next = match ix {
            None => Some(0),
            Some(ix) if down => (ix + 1 < len).then_some(ix + 1),
            Some(ix) => ix.checked_sub(1),
        };
        let Some(next) = next else { return };
        let id = self
            .tree
            .read(cx)
            .entry(next)
            .expect("next is in range")
            .item()
            .id
            .clone();
        if extend && self.anchor.is_none() {
            self.anchor = self.tree.read(cx).selected_item().map(|i| i.id.clone());
        }
        self.click_select(&id, false, extend, cx);
        let strategy = if down {
            gpui_kit::ScrollStrategy::Bottom
        } else {
            gpui_kit::ScrollStrategy::Top
        };
        self.tree.update(cx, |t, _| {
            if let Some(i) = t.selected_index() {
                t.scroll_to_item(i, strategy);
            }
        });
    }

    /// While a drag is inside the tree and within one row of its top or bottom, step one row that way.
    /// Holding still keeps stepping: each step asks for another frame. `scroll_edge` is the decision.
    fn autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_active_drag() {
            return;
        }
        let (bounds, top, bottom, len) = {
            let tree = self.tree.read(cx);
            let state = tree.scroll_handle().0.borrow();
            let bounds = state.base_handle.bounds();
            let top = state.base_handle.top_item();
            let bottom = state.base_handle.bottom_item();
            drop(state);
            let len = (0..).position(|i| tree.entry(i).is_none()).unwrap_or(0);
            (bounds, top, bottom, len)
        };
        let margin = tokens::navigator::ROW_HEIGHT.to_pixels(window.rem_size());
        let pos = window.mouse_position();
        let Some(down) = scroll_edge(
            pos.x.as_f32(),
            pos.y.as_f32(),
            bounds.left().as_f32(),
            bounds.right().as_f32(),
            bounds.top().as_f32(),
            bounds.bottom().as_f32(),
            margin.as_f32(),
        ) else {
            return;
        };
        if (!down && top == 0) || (down && bottom + 1 >= len) {
            return;
        }
        let now = std::time::Instant::now();
        if self.autoscroll_at.is_some_and(|at| {
            now.saturating_duration_since(at) < std::time::Duration::from_millis(80)
        }) {
            window.request_animation_frame();
            return;
        }
        self.autoscroll_at = Some(now);
        let target = if down { bottom + 1 } else { top - 1 };
        let strategy = if down {
            gpui_kit::ScrollStrategy::Bottom
        } else {
            gpui_kit::ScrollStrategy::Top
        };
        self.tree
            .update(cx, |t, _| t.scroll_to_item(target, strategy));
        window.request_animation_frame();
    }

    /// Right expands the focused folder (a closed connection opens). Left collapses it.
    fn fold(&mut self, expand: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (id, folder) = {
            let tree = self.tree.read(cx);
            let Some(entry) = tree.selected_entry() else {
                return;
            };
            (entry.item().id.clone(), entry.is_folder())
        };
        if !folder {
            return;
        }
        let Some(node) = Node::parse(&id) else { return };
        if expand {
            if let Node::Connection(c) = &node
                && !Sessions::global(cx).read(cx).state(c).is_open()
            {
                window.dispatch_action(Box::new(OpenConnection), cx);
                return;
            }
            if self.expanded.insert(id) {
                Self::ensure_children(&node, false, cx);
                self.rebuild(cx);
            }
        } else if self.expanded.remove(&id) {
            self.rebuild(cx);
        }
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
        self.start_rename_target(RenameTarget::Group(group), name, window, cx);
    }

    fn start_rename_vgroup(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = VirtualGroups::global(cx)
            .read(cx)
            .get(&id)
            .map(|g| g.name.clone());
        if let Some(name) = name {
            self.start_rename_target(RenameTarget::VGroup(id), name, window, cx);
        }
    }

    /// Creates a virtual group in the selected category (moving the selected objects into it)
    /// and edits its name in place.
    pub(crate) fn new_virtual_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let objects = crate::ops::selected_objects(cx);
        let (scope, kind) = match Workspace::global(cx).read(cx).selection() {
            Selection::Category(s, k) => (s.clone(), *k),
            Selection::Object(o) => (o.scope.clone(), o.kind),
            _ => return,
        };
        self.expanded
            .insert(Node::Category(scope.clone(), kind).id());
        let name = t!("nav.new_group_default").to_string();
        let id = VirtualGroups::global(cx).update(cx, |g, cx| {
            let id = g.add_group(scope, kind, name, cx);
            g.move_objects(&objects, Some(&id), cx);
            id
        });
        self.start_rename_vgroup(id, window, cx);
    }

    /// Edits an object's name in place; Enter runs the dialect's `RENAME` statement.
    pub(crate) fn start_rename_object(
        &mut self,
        obj: ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let renamable = Sessions::global(cx)
            .read(cx)
            .dialect(&obj.scope.connection)
            .and_then(|d| d.rename_object(&obj.scope, obj.kind, &obj.name, &obj.name))
            .is_some();
        if renamable {
            let name = obj.name.clone();
            self.start_rename_target(RenameTarget::Object(obj), name, window, cx);
        }
    }

    fn start_rename_target(
        &mut self,
        target: RenameTarget,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            target,
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
        window.focus(&self.focus_handle, cx);
        self.refresh_rows(cx);
        if !commit || name.is_empty() {
            return;
        }
        match r.target {
            RenameTarget::Group(g) => {
                ConnectionStore::global(cx).update(cx, |s, cx| s.rename_group(&g, name, cx))
            }
            RenameTarget::VGroup(g) => {
                VirtualGroups::global(cx).update(cx, |s, cx| s.rename_group(&g, name, cx))
            }
            RenameTarget::Object(o) if o.name != name => {
                let sql = Sessions::global(cx)
                    .read(cx)
                    .dialect(&o.scope.connection)
                    .and_then(|d| d.rename_object(&o.scope, o.kind, &o.name, &name));
                if let Some(sql) = sql {
                    let renamed = Selection::Object(ObjectRef { name, ..o.clone() });
                    crate::ops::run(vec![(o, sql)], Some(renamed), window, cx);
                }
            }
            RenameTarget::Object(_) => {}
        }
    }

    pub(crate) fn context_menu(node: Option<&Node>, cx: &App) -> NativeMenu {
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
                let mysql = Sessions::global(cx).read(cx).dialect(id) == Some(Dialect::MySql);
                let menu = if open && mysql {
                    menu.menu(t!("nav.reload").to_string(), Box::new(ReloadConnection))
                } else {
                    menu
                };
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
                let conn = ConnectionStore::global(cx).read(cx).get(id);
                let starred = conn.is_some_and(|c| c.starred);
                let star = if starred {
                    t!("nav.unstar")
                } else {
                    t!("nav.star")
                };
                // F-CONN-021
                let profiles = conn.map_or(NativeMenu::new(), |c| {
                    c.profile_names()
                        .fold(NativeMenu::new(), |m, n| {
                            let label = if n.is_empty() {
                                t!("nav.profile_default").to_string()
                            } else {
                                n.to_string()
                            };
                            m.menu_with_check(
                                label,
                                n == c.profile,
                                Box::new(SwitchProfile {
                                    name: n.to_string().into(),
                                }),
                            )
                        })
                        .separator()
                        .menu(t!("nav.new_profile").to_string(), Box::new(NewProfile))
                        .menu_with_disabled(
                            t!("nav.delete_profile").to_string(),
                            c.profiles.is_empty(),
                            Box::new(DeleteProfile),
                        )
                });
                let mut menu = menu
                    .separator()
                    .submenu(t!("nav.color").to_string(), colors)
                    .menu(star.to_string(), Box::new(ToggleStar))
                    .submenu(t!("nav.switch_profile").to_string(), profiles);
                if !open {
                    menu = menu.submenu(t!("nav.move_to_group").to_string(), move_menu);
                }
                refresh(menu)
            }
            // Items of later milestones (designers M4, queries M3, SQL files M6) are disabled
            Some(Node::Database(..)) => refresh(
                NativeMenu::new()
                    .menu(t!("nav.open_database").to_string(), Box::new(OpenDatabase))
                    .menu(
                        t!("nav.close_database").to_string(),
                        Box::new(CloseDatabase),
                    )
                    .separator()
                    .menu_with_disabled(t!("nav.new_database").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(
                        t!("nav.edit_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .menu_with_disabled(
                        t!("nav.delete_database").to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .separator()
                    .menu_with_disabled(t!("nav.new_query").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(t!("nav.run_sql_file").to_string(), true, Box::new(Refresh))
                    .menu_with_disabled(
                        t!("nav.dump_sql_file").to_string(),
                        true,
                        Box::new(Refresh),
                    ),
            ),
            Some(Node::Schema(_)) => refresh(NativeMenu::new().menu_with_disabled(
                t!("nav.new_query").to_string(),
                true,
                Box::new(Refresh),
            )),
            Some(Node::Category(_, k)) => refresh(
                NativeMenu::new()
                    .menu_with_disabled(
                        t!("obj.new_object", kind = kind_label(*k)).to_string(),
                        true,
                        Box::new(Refresh),
                    )
                    .separator()
                    .submenu(
                        t!("obj.manage_groups").to_string(),
                        NativeMenu::new()
                            .menu(t!("obj.new_vgroup").to_string(), Box::new(NewVirtualGroup)),
                    ),
            ),
            Some(Node::VGroup(_, _, g)) => refresh(
                NativeMenu::new()
                    .menu(
                        t!("obj.rename").to_string(),
                        Box::new(RenameVirtualGroup {
                            group: g.clone().into(),
                        }),
                    )
                    .menu(
                        t!("nav.delete_group").to_string(),
                        Box::new(DeleteVirtualGroup {
                            group: g.clone().into(),
                        }),
                    ),
            ),
            Some(Node::Object(o)) => refresh(Self::object_menu(o, cx)),
            Some(_) => NativeMenu::new().menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
        }
    }

    /// Context menu of an object, shared with the objects tab.
    pub(crate) fn object_menu(o: &ObjectRef, cx: &App) -> NativeMenu {
        let d = Sessions::global(cx).read(cx).dialect(&o.scope.connection);
        let kind = kind_label(o.kind);
        let droppable = d.and_then(|d| d.drop_keyword(o.kind)).is_some();
        let renamable = d
            .and_then(|d| d.rename_object(&o.scope, o.kind, &o.name, &o.name))
            .is_some();
        let mut menu = NativeMenu::new()
            .menu(
                t!("obj.open_kind", kind = kind).to_string(),
                Box::new(OpenObject { object: o.clone() }),
            )
            .menu_with_disabled(
                t!("obj.design_kind", kind = kind).to_string(),
                true,
                Box::new(Refresh),
            )
            .menu_with_disabled(
                t!("obj.new_object", kind = kind).to_string(),
                true,
                Box::new(Refresh),
            )
            .menu_with_disabled(
                t!("obj.delete_kind", kind = kind).to_string(),
                !droppable,
                Box::new(DeleteObjects),
            );
        if o.kind == ObjectKind::Table {
            menu = menu.menu(t!("obj.empty_table").to_string(), Box::new(EmptyTables));
            if d != Some(Dialect::Sqlite) {
                menu = menu.menu(
                    t!("obj.truncate_table").to_string(),
                    Box::new(TruncateTables),
                );
            }
            menu = menu.submenu(
                t!("obj.duplicate").to_string(),
                NativeMenu::new()
                    .menu(
                        t!("obj.duplicate_data").to_string(),
                        Box::new(DuplicateTable { with_data: true }),
                    )
                    .menu(
                        t!("obj.duplicate_structure").to_string(),
                        Box::new(DuplicateTable { with_data: false }),
                    ),
            );
        }
        let vgroups = VirtualGroups::global(cx).read(cx);
        let add = vgroups
            .groups_for(&o.scope, o.kind)
            .fold(NativeMenu::new(), |m, g| {
                m.menu(
                    g.name.clone(),
                    Box::new(AddToVirtualGroup {
                        group: g.id.clone().into(),
                    }),
                )
            });
        let has_groups = vgroups.groups_for(&o.scope, o.kind).next().is_some();
        let mut groups =
            NativeMenu::new().menu(t!("obj.new_vgroup").to_string(), Box::new(NewVirtualGroup));
        groups = if has_groups {
            groups.submenu(t!("obj.add_to_vgroup").to_string(), add)
        } else {
            groups.menu_with_disabled(t!("obj.add_to_vgroup").to_string(), true, Box::new(Refresh))
        };
        let grouped = vgroups.group_of(o).is_some();
        menu.menu_with_disabled(
            t!("obj.rename").to_string(),
            !renamable,
            Box::new(RenameObject),
        )
        .separator()
        .submenu(
            t!("obj.manage_groups").to_string(),
            groups.menu_with_disabled(
                t!("obj.remove_from_vgroup").to_string(),
                !grouped,
                Box::new(RemoveFromVirtualGroup),
            ),
        )
        .menu(t!("obj.copy_name").to_string(), Box::new(CopyObjectNames))
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
            Some(Node::TableFolder(_, c)) | Some(Node::TableItem(_, c, _)) => {
                (tinted(c.icon()), None)
            }
            Some(Node::VGroup(..)) => (
                tinted(if entry.is_expanded() {
                    "folder-open"
                } else {
                    "folder"
                }),
                None,
            ),
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
        let renaming = match (&node, &self.renaming) {
            (Some(Node::Group(g)), Some(r)) => r.target == RenameTarget::Group(g.clone()),
            (Some(Node::VGroup(_, _, g)), Some(r)) => r.target == RenameTarget::VGroup(g.clone()),
            (Some(Node::Object(o)), Some(r)) => r.target == RenameTarget::Object(o.clone()),
            _ => false,
        };
        let label = match &self.renaming {
            Some(r) if renaming => Input::new(&r.input).xsmall().into_any_element(),
            _ => truncated(
                SharedString::from(format!("{id}-label")),
                entry.item().label.clone(),
            )
            .into_any_element(),
        };
        let disclose = node.clone();
        let folder = entry.is_folder();
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
                            .id(SharedString::from(format!("nav-disclose-{id}")))
                            .w(tokens::navigator::ROW_ICON.rems())
                            .h(tokens::navigator::ROW_HEIGHT.rems())
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .children(chevron)
                            .when(folder, |d| {
                                let disclose = disclose.clone();
                                d.on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        let Some(n) = &disclose else { return };
                                        window.focus(&this.focus_handle, cx);
                                        this.click_select(&n.id(), false, false, cx);
                                        this.disclose(n, window, cx);
                                    },
                                ))
                            }),
                    )
                    .child(icon)
                    .child(div().flex_1().min_w_0().overflow_hidden().child(label))
                    .when_some(color, |r, c| {
                        r.child(div().size_2().rounded_full().bg(c).mr_2())
                    }),
            )
            // Before the drag handler, so a drag still starts. Stops the tree from expanding
            // on mouse down; a click on the label only selects.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus_handle, cx);
                }),
            );
        let row = match &node {
            Some(Node::Connection(c)) => {
                let (row_id, conn, name) = (id.clone(), c.clone(), entry.item().label.clone());
                row.on_drag(
                    DraggedConnections {
                        row: row_id,
                        conn,
                        name,
                    },
                    |d, _, _, cx| cx.new(|_| d.clone()),
                )
            }
            Some(Node::Object(o)) => {
                let (row_id, object, name) = (id.clone(), o.clone(), entry.item().label.clone());
                row.on_drag(
                    DraggedObjects {
                        row: row_id,
                        object,
                        name,
                    },
                    |d, _, _, cx| cx.new(|_| d.clone()),
                )
            }
            _ => row,
        };
        // Objects dropped onto a virtual group join it; onto their category, they leave their group
        let row = match &node {
            Some(Node::VGroup(_, _, g)) => {
                let g = g.clone();
                row.drag_over::<DraggedObjects>(move |s, _, _, _| s.bg(p.fill))
                    .on_drop(cx.listener(move |_, d: &DraggedObjects, _, cx| {
                        cx.stop_propagation();
                        move_objects(&d.objects(cx), Some(&g), cx);
                    }))
            }
            Some(Node::Category(..)) => row
                .drag_over::<DraggedObjects>(move |s, _, _, _| s.bg(p.fill))
                .on_drop(cx.listener(|_, d: &DraggedObjects, _, cx| {
                    cx.stop_propagation();
                    move_objects(&d.objects(cx), None, cx);
                })),
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
                    move_connections(&d.ids(cx), group.clone(), cx);
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

    fn render_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let quick = |id: &'static str, icon: &'static str, on: bool, tip: String| {
            gpui_kit::component::button::Button::new(id)
                .ghost()
                .small()
                .tooltip(tip)
                .selected(on)
                .icon(
                    lucide(icon)
                        .size(tokens::navigator::FILTER_ICON.rems())
                        .text_color(if on { p.accent } else { p.label_secondary }),
                )
        };
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
            .child(
                quick(
                    "nav-filter-type",
                    "database",
                    self.type_filter.is_some(),
                    t!("nav.filter_type").to_string(),
                )
                .on_click(|e, window, cx| {
                    let menu = navidog_db::DriverRegistry::all(cx).into_iter().fold(
                        NativeMenu::new()
                            .menu(
                                t!("nav.filter_all_types").to_string(),
                                Box::new(FilterByType { driver: None }),
                            )
                            .separator(),
                        |m, d| {
                            let info = d.info();
                            m.menu(
                                info.name.clone(),
                                Box::new(FilterByType {
                                    driver: Some(info.id.clone()),
                                }),
                            )
                        },
                    );
                    menu.show(e.position(), window, cx);
                }),
            )
            .child(
                quick(
                    "nav-filter-active",
                    "circle-check",
                    self.view.only_active,
                    t!("nav.filter_active").to_string(),
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleOnlyActive), cx)),
            )
            .child(
                quick(
                    "nav-filter-starred",
                    "star",
                    self.only_starred,
                    t!("nav.filter_starred").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.only_starred = !this.only_starred;
                    this.rebuild(cx);
                })),
            )
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
    vgroups: &'a VirtualGroups,
    query: &'a str,
    filtering: bool,
    show_children: bool,
    show_objects: bool,
}

impl TreeBuilder<'_> {
    fn matches(&self, label: &str) -> bool {
        !self.filtering || label.to_lowercase().contains(self.query)
    }

    fn object_item(&self, scope: &Scope, kind: ObjectKind, name: &str) -> TreeItem {
        let obj = ObjectRef {
            scope: scope.clone(),
            kind,
            name: name.to_string(),
        };
        if kind == ObjectKind::Table && self.show_children {
            let children = self.table_children(&obj);
            self.item(Node::Object(obj), name.to_string(), children)
        } else {
            TreeItem::new(Node::Object(obj).id(), name.to_string())
        }
    }

    /// Fields, indexes, foreign keys and triggers folders under a table.
    fn table_children(&self, table: &ObjectRef) -> Vec<TreeItem> {
        Child::ALL
            .into_iter()
            .map(|child| {
                let folder = Node::TableFolder(table.clone(), child);
                let loaded = match child {
                    Child::Fields => match self.catalog.columns(&table.scope, &table.name) {
                        Some(Load::Ready(cols)) => {
                            Ok(cols.iter().map(|c| c.name.clone()).collect())
                        }
                        Some(Load::Failed(e)) => Err(e.to_string()),
                        _ => Err(t!("nav.loading").to_string()),
                    },
                    _ => match self.catalog.table_children(&table.scope, &table.name) {
                        Some(Load::Ready(c)) => Ok(match child {
                            Child::Indexes => c.indexes.clone(),
                            Child::ForeignKeys => c.foreign_keys.clone(),
                            _ => c.triggers.clone(),
                        }),
                        Some(Load::Failed(e)) => Err(e.to_string()),
                        _ => Err(t!("nav.loading").to_string()),
                    },
                };
                let items = match loaded {
                    Ok(names) => names
                        .into_iter()
                        .map(|n: String| {
                            TreeItem::new(Node::TableItem(table.clone(), child, n.clone()).id(), n)
                        })
                        .collect(),
                    Err(message) => Self::placeholder(&folder, message),
                };
                self.item(
                    folder,
                    t!(format!("nav.child_{}", child.key())).to_string(),
                    items,
                )
            })
            .collect()
    }

    /// Whether the search or a quick filter is hiding nodes (empty groups are hidden then).
    fn narrowing(&self) -> bool {
        let p = self.panel;
        self.filtering || p.view.only_active || p.only_starred || p.type_filter.is_some()
    }

    fn shows_connection(&self, c: &Connection) -> bool {
        let p = self.panel;
        !c.hidden
            && (!p.only_starred || c.starred)
            && (!p.view.only_active || self.sessions.state(&c.id).is_open())
            && p.type_filter
                .as_ref()
                .is_none_or(|t| t.as_ref() == c.driver)
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
            if children.is_empty()
                && (!self.matches(&g.name) || (self.narrowing() && !self.filtering))
            {
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
        if !self.shows_connection(c) {
            return None;
        }
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
        let view = &self.panel.view;
        if (!view.show_hidden && dialect.is_system_database(db))
            || (view.only_active && !self.panel.expanded.contains(&node.id()))
        {
            return None;
        }
        let children = if dialect.has_schemas() {
            match self.catalog.schemas(conn, db) {
                Some(Load::Ready(list)) => list
                    .iter()
                    .filter(|s| view.show_hidden || !dialect.is_system_schema(s))
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
        if !self.show_objects {
            return Vec::new();
        }
        dialect
            .object_kinds()
            .iter()
            .filter_map(|&kind| {
                let node = Node::Category(scope.clone(), kind);
                let children = match self.catalog.objects(scope, kind) {
                    Some(Load::Ready(list)) => {
                        let names: Vec<&str> = list
                            .iter()
                            .map(|o| o.name.as_str())
                            .filter(|n| self.matches(n))
                            .collect();
                        let mut items = Vec::new();
                        let mut grouped = HashSet::new();
                        if !self.panel.view.hide_object_groups {
                            for g in self.vgroups.groups_for(scope, kind) {
                                let members: Vec<TreeItem> = g
                                    .members
                                    .iter()
                                    .filter(|m| names.contains(&m.as_str()))
                                    .map(|m| {
                                        grouped.insert(m.as_str());
                                        self.object_item(scope, kind, m)
                                    })
                                    .collect();
                                if self.filtering && members.is_empty() && !self.matches(&g.name) {
                                    continue;
                                }
                                let vg = Node::VGroup(scope.clone(), kind, g.id.clone());
                                items.push(self.item(vg, g.name.clone(), members));
                            }
                        }
                        items.extend(
                            names
                                .iter()
                                .filter(|n| !grouped.contains(**n))
                                .map(|n| self.object_item(scope, kind, n)),
                        );
                        items
                    }
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
            .on_drop(cx.listener(|_, d: &DraggedConnections, _, cx| {
                move_connections(&d.ids(cx), None, cx)
            }))
            .child(tree_view);
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
            .on_action(cx.listener(|this, _: &NavUp, _, cx| this.step_selection(false, false, cx)))
            .on_action(cx.listener(|this, _: &NavDown, _, cx| this.step_selection(true, false, cx)))
            .on_action(
                cx.listener(|this, _: &NavExtendUp, _, cx| this.step_selection(false, true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NavExtendDown, _, cx| this.step_selection(true, true, cx)),
            )
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
}
