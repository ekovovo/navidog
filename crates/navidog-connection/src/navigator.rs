//! 导航窗格（10-shell.md 第 6 节）：连接组 > 连接 > 数据库。

use std::collections::HashSet;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::tree::{TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::component::{Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    Styled as _, StyledImage as _, Subscription, WeakEntity, Window, div, img,
};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_ui::assets::driver_icon;
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::actions::*;
use crate::sessions::{ConnState, Sessions};

const PANEL: &str = "NavigatorPanel";

struct ActiveNavigator(WeakEntity<NavigatorPanel>);
impl Global for ActiveNavigator {}

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(NavigatorPanel::shared(window, cx))
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Group(String),
    Connection(String),
    Database(String, String),
}

impl Node {
    fn id(&self) -> SharedString {
        match self {
            Self::Group(g) => format!("g:{g}").into(),
            Self::Connection(c) => format!("c:{c}").into(),
            Self::Database(c, d) => format!("d:{c}\u{1f}{d}").into(),
        }
    }

    fn parse(id: &str) -> Option<Self> {
        let (kind, rest) = id.split_once(':')?;
        Some(match kind {
            "g" => Self::Group(rest.into()),
            "c" => Self::Connection(rest.into()),
            "d" => {
                let (c, d) = rest.split_once('\u{1f}')?;
                Self::Database(c.into(), d.into())
            }
            _ => return None,
        })
    }

    fn selection(&self) -> Selection {
        match self.clone() {
            Self::Group(g) => Selection::Group(g),
            Self::Connection(c) => Selection::Connection(c),
            Self::Database(c, d) => Selection::Database(c, d),
        }
    }

    fn from_selection(s: &Selection) -> Option<Self> {
        Some(match s.clone() {
            Selection::Group(g) => Self::Group(g),
            Selection::Connection(c) => Self::Connection(c),
            Selection::Database(c, d) => Self::Database(c, d),
            Selection::None => return None,
        })
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
                cx.new(|cx| InputState::new(window, cx).placeholder(t!("conn.search").to_string()));
            let store = ConnectionStore::global(cx);
            let sessions = Sessions::global(cx);
            let workspace = Workspace::global(cx);
            let subs = vec![
                cx.observe(&store, |this, _, cx| this.rebuild(cx)),
                cx.observe(&sessions, |this, _, cx| this.rebuild(cx)),
                cx.subscribe(&sessions, |this, _, e, cx| {
                    if let crate::sessions::SessionsEvent::Opened(id) = e {
                        this.expanded.insert(Node::Connection(id.clone()).id());
                        this.rebuild(cx);
                    }
                }),
                cx.observe(&filter, |this, _, cx| this.rebuild(cx)),
                cx.subscribe(&tree, |this, _, e: &TreeEvent, cx| {
                    match e {
                        TreeEvent::Expanded(id) => this.expanded.insert(id.clone()),
                        TreeEvent::Collapsed(id) => this.expanded.remove(id),
                    };
                    cx.notify();
                }),
                cx.observe(&tree, |this, tree, cx| {
                    let node = tree
                        .read(cx)
                        .selected_item()
                        .and_then(|i| Node::parse(&i.id));
                    let sel = node.map(|n| n.selection()).unwrap_or_default();
                    Workspace::global(cx).update(cx, |w, cx| w.select(sel, cx));
                    let _ = this;
                }),
                cx.subscribe(&workspace, |this, _, e: &WorkspaceEvent, cx| {
                    let WorkspaceEvent::SelectionChanged = e;
                    this.sync_selection(cx);
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

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().trim().to_lowercase();
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        let sessions = Sessions::global(cx);
        let sessions = sessions.read(cx);
        let filtering = !query.is_empty();
        let matches = |label: &str| !filtering || label.to_lowercase().contains(&query);

        let conn_item = |c: &navidog_core::connection::Connection| -> Option<TreeItem> {
            let node = Node::Connection(c.id.clone());
            let dbs: Vec<TreeItem> = match sessions.state(&c.id) {
                ConnState::Open { databases, .. } => databases
                    .iter()
                    .filter(|d| matches(d) || matches(&c.name))
                    .map(|d| TreeItem::new(Node::Database(c.id.clone(), d.clone()).id(), d.clone()))
                    .collect(),
                _ => Vec::new(),
            };
            if !matches(&c.name) && dbs.is_empty() {
                return None;
            }
            let expanded = filtering || self.expanded.contains(&node.id());
            Some(
                TreeItem::new(node.id(), c.name.clone())
                    .children(dbs)
                    .expanded(expanded),
            )
        };

        fn group_items(
            parent: Option<&str>,
            store: &navidog_core::store::ConnectionStore,
            this: &NavigatorPanel,
            filtering: bool,
            conn_item: &dyn Fn(&navidog_core::connection::Connection) -> Option<TreeItem>,
            matches: &dyn Fn(&str) -> bool,
        ) -> Vec<TreeItem> {
            let mut items = Vec::new();
            for g in store
                .groups()
                .iter()
                .filter(|g| g.parent.as_deref() == parent)
            {
                let children = group_items(Some(&g.id), store, this, filtering, conn_item, matches);
                if children.is_empty() && !matches(&g.name) {
                    continue;
                }
                let node = Node::Group(g.id.clone());
                let expanded = filtering || this.expanded.contains(&node.id());
                items.push(
                    TreeItem::new(node.id(), g.name.clone())
                        .children(children)
                        .expanded(expanded),
                );
            }
            items.extend(
                store
                    .connections()
                    .iter()
                    .filter(|c| c.group.as_deref() == parent)
                    .filter_map(conn_item),
            );
            items
        }

        let items = if self.hide_groups {
            let mut conns: Vec<_> = store.connections().iter().filter_map(conn_item).collect();
            conns.sort_by(|a, b| a.label.cmp(&b.label));
            conns
        } else {
            group_items(None, store, self, filtering, &conn_item, &matches)
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

    fn toggle(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        let id = node.id();
        match node {
            Node::Connection(c) if !Sessions::global(cx).read(cx).state(c).is_open() => {
                crate::open_connection(c.clone(), window, cx);
                return;
            }
            Node::Database(..) => return,
            _ => {}
        }
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.rebuild(cx);
    }

    fn select(node: &Node, cx: &mut App) {
        Workspace::global(cx).update(cx, |w, cx| w.select(node.selection(), cx));
    }

    fn context_menu(node: Option<&Node>, cx: &App) -> NativeMenu {
        let new_conn = crate::drivers(cx)
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
            None => NativeMenu::new()
                .submenu(t!("conn.new_connection").to_string(), new_conn)
                .menu(t!("nav.new_group").to_string(), Box::new(NewGroup))
                .separator()
                .menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
            Some(Node::Group(_)) => NativeMenu::new()
                .submenu(t!("conn.new_connection").to_string(), new_conn)
                .menu(t!("nav.new_group").to_string(), Box::new(NewGroup))
                .menu(t!("nav.rename_group").to_string(), Box::new(RenameGroup))
                .menu(t!("nav.delete_group").to_string(), Box::new(DeleteGroup))
                .separator()
                .menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
            Some(Node::Connection(id)) => {
                let open = Sessions::global(cx).read(cx).state(id).is_open();
                let store = ConnectionStore::global(cx);
                let groups = store.read(cx).groups().to_vec();
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
                    menu.submenu(t!("conn.new_connection").to_string(), new_conn)
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
                menu.separator()
                    .menu(t!("nav.refresh").to_string(), Box::new(Refresh))
            }
            Some(Node::Database(..)) => NativeMenu::new()
                .menu(t!("nav.open_database").to_string(), Box::new(OpenDatabase))
                .menu(
                    t!("nav.close_database").to_string(),
                    Box::new(CloseDatabase),
                )
                .separator()
                .menu(t!("nav.refresh").to_string(), Box::new(Refresh)),
        }
    }

    fn render_row(
        &self,
        ix: usize,
        entry: &gpui_kit::component::tree::TreeEntry,
        selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let p = palette(cx);
        let id = entry.item().id.clone();
        let node = Node::parse(&id);
        let store = ConnectionStore::global(cx);
        let sessions = Sessions::global(cx);
        let focused = self.focus_handle.contains_focused(window, cx);
        let (icon, color): (gpui_kit::AnyElement, Option<gpui_kit::Hsla>) = match &node {
            Some(Node::Group(_)) => (
                Icon::new(if entry.is_expanded() {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .size(tokens::navigator::ROW_ICON.rems())
                .text_color(p.label_secondary)
                .into_any_element(),
                None,
            ),
            Some(Node::Connection(c)) => {
                let conn = store.read(cx).get(c);
                let driver = conn.map(|c| c.driver.clone()).unwrap_or_default();
                let open = sessions.read(cx).state(c).is_open();
                let color = conn
                    .and_then(|c| c.color.as_deref())
                    .and_then(|n| tokens::connection_color(n, cx));
                (
                    img(driver_icon(&driver))
                        .size(tokens::navigator::ROW_ICON.rems())
                        .when(!open, |i| i.grayscale(true).opacity(0.6))
                        .into_any_element(),
                    color,
                )
            }
            _ => (
                lucide("database")
                    .size(tokens::navigator::ROW_ICON.rems())
                    .text_color(p.label_secondary)
                    .into_any_element(),
                None,
            ),
        };
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
        let fg = if selected && focused {
            p.selection_text
        } else {
            p.label
        };
        let bg = if selected {
            Some(if focused {
                p.selection
            } else {
                p.selection_inactive
            })
        } else {
            None
        };
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
                cx.listener(move |this, e: &gpui_kit::MouseDownEvent, window, cx| {
                    if let Some(n) = &right_node {
                        Self::select(n, cx);
                    }
                    window.focus(&this.focus_handle, cx);
                    Self::context_menu(right_node.as_ref(), cx).show(e.position, window, cx);
                }),
            )
            .on_click(
                cx.listener(move |this, e: &gpui_kit::ClickEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    let Some(n) = &click_node else { return };
                    Self::select(n, cx);
                    if e.click_count() >= 2 {
                        this.toggle(n, window, cx);
                    }
                }),
            )
    }

    fn render_filter(&self, cx: &App) -> gpui_kit::AnyElement {
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
                // M2 实现快速筛选，M1 显示为禁用
                lucide(i)
                    .size(tokens::navigator::FILTER_ICON.rems())
                    .text_color(p.disabled_text)
            }))
            .into_any_element()
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
            cx.listener(|this, e: &gpui_kit::MouseDownEvent, window, cx| {
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

pub(crate) fn expand_selected(window: &mut Window, cx: &mut App) {
    let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection()) else {
        return;
    };
    with_navigator(cx, |this, cx| {
        if !this.expanded.contains(&node.id()) {
            this.toggle(&node, window, cx);
        }
    });
}

pub(crate) fn collapse_selected(_: &mut Window, cx: &mut App) {
    let Some(node) = Node::from_selection(Workspace::global(cx).read(cx).selection()) else {
        return;
    };
    with_navigator(cx, |this, cx| {
        this.expanded.remove(&node.id());
        this.rebuild(cx);
    });
}

/// 切换“在顶部显示筛选”与“隐藏连接组”（查看 > 导航窗格）。
pub fn set_view_options(filter_on_top: bool, hide_groups: bool, cx: &mut App) {
    with_navigator(cx, |this, cx| {
        this.filter_on_top = filter_on_top;
        this.hide_groups = hide_groups;
        this.rebuild(cx);
    });
}

pub(crate) fn new_group(window: &mut Window, cx: &mut App) {
    let parent = match Workspace::global(cx).read(cx).selection() {
        Selection::Group(g) => Some(g.clone()),
        _ => None,
    };
    prompt_name(
        t!("nav.new_group_default").to_string(),
        window,
        cx,
        move |name, cx| {
            let store = ConnectionStore::global(cx);
            let id = store.update(cx, |s, cx| s.add_group(name, parent.clone(), cx));
            Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Group(id), cx));
        },
    );
}

pub(crate) fn rename_group(window: &mut Window, cx: &mut App) {
    let Selection::Group(id) = Workspace::global(cx).read(cx).selection().clone() else {
        return;
    };
    let Some(current) = ConnectionStore::global(cx)
        .read(cx)
        .groups()
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.name.clone())
    else {
        return;
    };
    prompt_name(current, window, cx, move |name, cx| {
        ConnectionStore::global(cx).update(cx, |s, cx| s.rename_group(&id, name, cx));
    });
}

/// 组名输入对话框。
// 偏差：Navicat 在树中原地编辑组名，M1 用对话框代替，M2 改为原地编辑
fn prompt_name(
    initial: String,
    window: &mut Window,
    cx: &mut App,
    on_ok: impl Fn(String, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    input.update(cx, |s, cx| s.focus(window, cx));
    let on_ok = std::rc::Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, window, _| {
        let commit = {
            let input = input.clone();
            let on_ok = on_ok.clone();
            move |cx: &mut App| {
                let name = input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return false;
                }
                on_ok(name, cx);
                true
            }
        };
        let on_button = commit.clone();
        dialog
            .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
            .title(t!("nav.group_name").to_string())
            .child(Input::new(&input))
            .on_ok(move |_, _, cx| commit(cx))
            .footer(crate::secrets::footer(
                t!("conn.ok").to_string(),
                move |_, window, cx| {
                    if on_button(cx) {
                        window.close_dialog(cx);
                    }
                },
            ))
    });
}
