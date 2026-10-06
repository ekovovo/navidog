//! 导航窗格（10-shell.md 第 6 节、20-objects.md 第 2 节）：
//! 组 > 连接 > 数据库 >（模式）> 对象分类 > 对象。

use std::collections::HashSet;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Global, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Render, SharedString, Styled as _, StyledImage as _, Subscription,
    WeakEntity, Window, div, img,
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

const PANEL: &str = "NavigatorPanel";
const SEP: char = '\u{1f}';

struct ActiveNavigator(WeakEntity<NavigatorPanel>);
impl Global for ActiveNavigator {}

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(NavigatorPanel::shared(window, cx))
    });
}

/// 树节点。ID 编码为 `前缀:字段␟字段…`，模式为空串表示没有模式层级。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    Group(String),
    Connection(String),
    Database(String, String),
    Schema(Scope),
    Category(Scope, ObjectKind),
    Object(ObjectRef),
    /// 尚未加载的占位子项（让父节点显示展开箭头）
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

    /// 从树根到本节点的祖先（不含自身），用于展开以显示选择。
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

pub struct NavigatorPanel {
    focus_handle: FocusHandle,
    tree: Entity<TreeState>,
    filter: Entity<InputState>,
    expanded: HashSet<SharedString>,
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
    /// 主窗口只有一个导航窗格；Dock 恢复布局时复用同一实例。
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
                cx.observe(&tree, |_, tree, cx| {
                    let selection = tree
                        .read(cx)
                        .selected_item()
                        .and_then(|i| Node::parse(&i.id))
                        .and_then(|n| n.selection());
                    if let Some(sel) = selection {
                        Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
                    }
                }),
                cx.subscribe(&Workspace::global(cx), |this, _, e: &WorkspaceEvent, cx| {
                    let WorkspaceEvent::SelectionChanged = e;
                    this.reveal_selection(cx);
                }),
            ];
            let mut this = Self {
                focus_handle: cx.focus_handle(),
                tree,
                filter,
                expanded: HashSet::new(),
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

    /// 展开节点时按需加载子项。
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

    /// 选择来自其他面板（对象选项卡、工具栏）时，展开祖先节点并加载数据，使选择在树中可见。
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
        ix: usize,
        entry: &TreeEntry,
        selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let p = palette(cx);
        let node = Node::parse(&entry.item().id);
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
        ListItem::new(ix)
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(entry.item().label.clone()),
                    )
                    .when_some(color, |r, c| {
                        r.child(div().size_2().rounded_full().bg(c).mr_2())
                    }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
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
                Self::select(n, cx);
                if e.click_count() >= 2 {
                    this.toggle(n, window, cx);
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
                // 快速筛选尚未实现，显示为禁用
                lucide(i)
                    .size(tokens::navigator::FILTER_ICON.rems())
                    .text_color(p.disabled_text)
            }))
            .into_any_element()
    }

    /// 刷新：重新加载所选作用域的元数据；连接级别的刷新由连接命令处理。
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

/// 构建树数据（只读访问各全局状态）。
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
                        panel.render_row(ix, entry, selected, window, cx)
                    })
                })
                .unwrap_or_else(|| ListItem::new(ix))
        });
        let filter = self.render_filter(cx);
        let empty_area = div().flex_1().on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, e: &MouseDownEvent, window, cx| {
                Workspace::global(cx).update(cx, |w, cx| w.select(Selection::None, cx));
                window.focus(&this.focus_handle, cx);
                Self::context_menu(None, cx).show(e.position, window, cx);
            }),
        );
        let body = v_flex()
            .flex_1()
            .min_h_0()
            .child(div().flex_1().min_h_0().child(tree_view))
            .child(empty_area);
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

/// 切换“在顶部显示筛选”与“隐藏连接组”（查看 > 导航窗格）。
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
