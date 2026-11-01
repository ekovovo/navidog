//! One tree row (icon, label, disclosure, drag and drop) and the search and quick filter bar.

use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::relative;

use super::*;

impl NavigatorPanel {
    pub(super) fn row_icon(
        &self,
        node: &Option<Node>,
        entry: &TreeEntry,
        invert: bool,
        cx: &App,
    ) -> (AnyElement, Option<Hsla>) {
        let p = palette(cx);
        let size = tokens::navigator::ROW_ICON.rems();
        let tinted = |name: &str| {
            let color = if invert {
                p.selection_text
            } else {
                tokens::icon_color(name, cx).unwrap_or(p.label_secondary)
            };
            lucide(name).size(size).text_color(color).into_any_element()
        };
        match node {
            Some(Node::Group(_)) => (
                Icon::new(if entry.is_expanded() {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .size(size)
                .text_color(if invert {
                    p.selection_text
                } else {
                    tokens::icon_color("folder", cx).unwrap_or(p.label_secondary)
                })
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
                    img(navidog_session::DriverRegistry::icon(cx, &driver))
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
            Some(Node::TableItem(table, Child::Fields, name)) => {
                let pk = Catalog::global(cx)
                    .read(cx)
                    .columns(&table.scope, &table.name)
                    .and_then(|load| match load {
                        Load::Ready(cols) => {
                            cols.iter().find(|c| c.name == *name).map(|c| c.primary_key)
                        }
                        _ => None,
                    })
                    .unwrap_or(false);
                (tinted(if pk { "key" } else { "columns-3" }), None)
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

    pub(super) fn render_row(
        &self,
        entry: &TreeEntry,
        selected: bool,
        marked: &HashSet<SharedString>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> ListItem {
        let p = palette(cx);
        let id = entry.item().id.clone();
        let node = Node::parse(&id);
        let selected = selected || marked.contains(&id);
        let focused = self.focus_handle.contains_focused(window, cx);
        let (icon, color) = self.row_icon(&node, entry, selected && focused, cx);
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
            (Some(Node::TableItem(table, Child::Fields, name)), Some(r)) => {
                r.target == RenameTarget::Field(table.clone(), name.clone())
            }
            _ => false,
        };
        let label = match &self.renaming {
            Some(r) if renaming => Input::new(&r.input).xsmall().into_any_element(),
            _ if placeholder && entry.item().label.is_empty() => {
                let n = match &node {
                    Some(Node::Placeholder(key)) => key
                        .rsplit_once('#')
                        .and_then(|(_, n)| n.parse::<usize>().ok())
                        .unwrap_or(0),
                    _ => 0,
                };
                let fraction = [0.72, 0.55, 0.42][n % 3];
                div()
                    .flex()
                    .items_center()
                    .h(tokens::navigator::ROW_HEIGHT.rems())
                    .w_full()
                    .child(
                        Skeleton::new()
                            .h(tokens::font::LINE_BODY.rems())
                            .w(relative(fraction))
                            .rounded_sm(),
                    )
                    .into_any_element()
            }
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
                                        this.click_select(&n.id(), false, false, window, cx);
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
                .on_drop(cx.listener(move |_, d: &DraggedConnections, window, cx| {
                    cx.stop_propagation();
                    move_connections(&d.ids(cx), group.clone(), window, cx);
                })),
            None => row,
        };
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(n) = &right_node {
                    this.select(n, window, cx);
                }
                window.focus(&this.focus_handle, cx);
                Self::context_menu(right_node.as_ref(), cx).show(e.position, window, cx);
            }),
        )
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            window.focus(&this.focus_handle, cx);
            let Some(n) = &click_node else { return };
            if e.click_count() >= 2 {
                this.select(n, window, cx);
                this.toggle(n, window, cx);
            } else {
                let m = e.modifiers();
                this.click_select(&n.id(), m.platform, m.shift, window, cx);
            }
        }))
    }

    pub(super) fn render_filter(&self, cx: &mut Context<Self>) -> AnyElement {
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
                    .id("nav-search")
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
                .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                    if !this.focus_handle.contains_focused(window, cx) {
                        window.focus(&this.focus_handle, cx);
                    }
                    let menu = navidog_session::DriverRegistry::all(cx).into_iter().fold(
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
                                    driver: Some(info.id.clone().into()),
                                }),
                            )
                        },
                    );
                    menu.show(e.position(), window, cx);
                })),
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
}
