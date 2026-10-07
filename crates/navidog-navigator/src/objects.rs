//! Objects tab (20-objects.md §4): object toolbar, list view, and detail view.

use std::cmp::Ordering;
use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, TabGroup, panel_handle, register_panel,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Global, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, UniformListScrollHandle, WeakEntity, Window, div, img, uniform_list,
};
use navidog_core::actions::{
    DeleteObjects, EmptyTables, OpenConnection, OpenObject, TruncateTables,
};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::settings::Settings;
use navidog_core::store::ConnectionStore;
use navidog_core::vgroups::VirtualGroups;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::ObjectInfo;
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions};
use navidog_ui::assets::{driver_icon, lucide};
use navidog_ui::skin::truncated;
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load};
use crate::navigator::kind_icon;

const PANEL: &str = "ObjectsPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(ObjectsPanel::shared(window, cx))
    });
}

struct SharedObjects(WeakEntity<ObjectsPanel>);
impl Global for SharedObjects {}

/// Shows or hides a property column of the detail view for the listed kind.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = objects, no_json)]
struct ToggleObjectColumn {
    key: SharedString,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    List,
    Detail,
}

pub struct ObjectsPanel {
    focus_handle: FocusHandle,
    view: ViewMode,
    /// Detail view sort: (column key, ascending)
    sort: Option<(String, bool)>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// One item in the list.
#[derive(Clone)]
pub(crate) struct Row {
    icon: RowIcon,
    label: SharedString,
    /// `Selection::None` for virtual group headers, which select nothing
    target: Selection,
    properties: Vec<(String, String)>,
    /// 1 for objects listed under their virtual group
    depth: u8,
}

#[derive(Clone)]
enum RowIcon {
    Driver(String),
    Folder,
    Lucide(&'static str),
}

/// List contents for the current selection.
pub(crate) enum Content {
    Rows(Vec<Row>, Option<(Scope, ObjectKind)>),
    Message(SharedString),
    Empty,
}

impl EventEmitter<PanelEvent> for ObjectsPanel {}

impl Focusable for ObjectsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ObjectsPanel {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn on_added_to(&mut self, group: WeakEntity<TabGroup>, _: &mut Window, cx: &mut Context<Self>) {
        tabs::added(PanelId::from(cx.entity_id()), group, cx);
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let _ = tabs::removed(PanelId::from(cx.entity_id()), cx);
    }
}

impl Panel for ObjectsPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tabs::title(
            PanelId::from(cx.entity_id()),
            None,
            t!("obj.objects_tab").to_string(),
            None,
        )
    }
}

/// The object category to show for the selection's scope: the selected category or the selected
/// object's category, otherwise tables by default.
fn listed_kind(selection: &Selection) -> Option<(Scope, ObjectKind)> {
    match selection {
        Selection::Category(s, k) => Some((s.clone(), *k)),
        Selection::Object(o) => Some((o.scope.clone(), o.kind)),
        Selection::Schema(s) => Some((s.clone(), ObjectKind::Table)),
        _ => None,
    }
}

impl ObjectsPanel {
    /// Reuses the same instance when the Dock restores its layout.
    pub fn shared(_: &mut Window, cx: &mut App) -> Entity<Self> {
        if let Some(panel) = cx.try_global::<SharedObjects>().and_then(|s| s.0.upgrade()) {
            return panel;
        }
        let panel = cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Sessions::global(cx), |_, _, cx| {
                    Self::ensure_loaded(cx);
                    cx.notify();
                }),
                cx.observe(&Catalog::global(cx), |_, _, cx| {
                    Self::ensure_loaded(cx);
                    cx.notify();
                }),
                cx.subscribe(&Workspace::global(cx), |_, _, e: &WorkspaceEvent, cx| {
                    let WorkspaceEvent::SelectionChanged = e;
                    Self::ensure_loaded(cx);
                    cx.notify();
                }),
            ];
            Self {
                focus_handle: cx.focus_handle(),
                view: ViewMode::List,
                sort: None,
                scroll: UniformListScrollHandle::new(),
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedObjects(panel.downgrade()));
        panel
    }

    /// Loads the object list when the selection changes (for MySQL / SQLite, selecting a database lists
    /// its tables).
    fn ensure_loaded(cx: &mut App) {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let catalog = Catalog::global(cx);
        if let Selection::Database(c, d) = &selection {
            match Sessions::global(cx).read(cx).dialect(c) {
                Some(dialect) if dialect.has_schemas() => {
                    catalog.update(cx, |cat, cx| cat.ensure_schemas(c, d, false, cx))
                }
                Some(_) => {
                    let scope = Scope::new(c.clone(), d.clone(), None);
                    catalog.update(cx, |cat, cx| {
                        cat.ensure_objects(&scope, ObjectKind::Table, false, cx)
                    });
                }
                None => {}
            }
        }
        if let Some((scope, kind)) = listed_kind(&selection) {
            catalog.update(cx, |cat, cx| cat.ensure_objects(&scope, kind, false, cx));
        }
    }

    /// True when the selection wants a catalog entry that was wiped (the connection
    /// opened and dropped in-flight loads) and the session can serve it again.
    fn stalled_load(cx: &App) -> bool {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let catalog = Catalog::global(cx).read(cx);
        let sessions = Sessions::global(cx).read(cx);
        let missing = |scope: &Scope, kind: ObjectKind| {
            catalog.objects(scope, kind).is_none()
                && sessions.state(&scope.connection).is_open()
                && sessions.dialect(&scope.connection).is_some()
        };
        if let Selection::Database(c, d) = &selection {
            match sessions.dialect(c) {
                Some(dialect) if dialect.has_schemas() => {
                    if catalog.schemas(c, d).is_none() && sessions.state(c).is_open() {
                        return true;
                    }
                }
                Some(_) => {
                    let scope = Scope::new(c.clone(), d.clone(), None);
                    if missing(&scope, ObjectKind::Table) {
                        return true;
                    }
                }
                None => {}
            }
        }
        listed_kind(&selection).is_some_and(|(scope, kind)| missing(&scope, kind))
    }

    pub(crate) fn content(cx: &App) -> Content {
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        let sessions = Sessions::global(cx);
        let sessions = sessions.read(cx);
        let catalog = Catalog::global(cx);
        let catalog = catalog.read(cx);
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let row = |icon, label: String, target| Row {
            icon,
            label: label.into(),
            target,
            properties: Vec::new(),
            depth: 0,
        };
        let vgroups = VirtualGroups::global(cx).read(cx);
        let grouping = !crate::navigator::view_options(cx).hide_object_groups;

        let connections = |group: Option<&str>| {
            if store.connections().is_empty() {
                return Content::Empty;
            }
            let groups = store
                .groups()
                .iter()
                .filter(|g| g.parent.as_deref() == group)
                .map(|g| {
                    row(
                        RowIcon::Folder,
                        g.name.clone(),
                        Selection::Group(g.id.clone()),
                    )
                });
            let conns = store
                .connections()
                .iter()
                .filter(|c| c.group.as_deref() == group)
                .map(|c| {
                    row(
                        RowIcon::Driver(c.driver.clone()),
                        c.name.clone(),
                        Selection::Connection(c.id.clone()),
                    )
                });
            Content::Rows(groups.chain(conns).collect(), None)
        };
        let objects = |scope: &Scope, kind: ObjectKind| match catalog.objects(scope, kind) {
            Some(Load::Ready(list)) => {
                let object_row = |o: &ObjectInfo, depth| Row {
                    icon: RowIcon::Lucide(kind_icon(kind)),
                    label: o.name.clone().into(),
                    target: Selection::Object(ObjectRef {
                        scope: scope.clone(),
                        kind,
                        name: o.name.clone(),
                    }),
                    properties: o.properties.clone(),
                    depth,
                };
                let mut rows = Vec::new();
                let mut grouped = std::collections::HashSet::new();
                if grouping {
                    for g in vgroups.groups_for(scope, kind) {
                        rows.push(row(RowIcon::Folder, g.name.clone(), Selection::None));
                        for o in g
                            .members
                            .iter()
                            .filter_map(|m| list.iter().find(|o| &o.name == m))
                        {
                            grouped.insert(o.name.as_str());
                            rows.push(object_row(o, 1));
                        }
                    }
                }
                rows.extend(
                    list.iter()
                        .filter(|o| !grouped.contains(o.name.as_str()))
                        .map(|o| object_row(o, 0)),
                );
                Content::Rows(rows, Some((scope.clone(), kind)))
            }
            Some(Load::Failed(e)) => Content::Message(e.clone()),
            _ => Content::Message(t!("obj.loading").to_string().into()),
        };

        let conn = selection.connection_id().cloned();
        if let Some(c) = &conn {
            match sessions.state(c) {
                ConnState::Open { .. } => {}
                ConnState::Opening => {
                    return Content::Message(t!("obj.connecting").to_string().into());
                }
                ConnState::Failed(e) => return Content::Message(e.clone().into()),
                ConnState::Closed => {
                    return Content::Message(t!("obj.not_connected").to_string().into());
                }
            }
        }
        match &selection {
            Selection::None => connections(None),
            Selection::Group(g) => connections(Some(g)),
            Selection::Connection(c) => match sessions.state(c) {
                ConnState::Open { databases, .. } => Content::Rows(
                    databases
                        .iter()
                        .map(|d| {
                            row(
                                RowIcon::Lucide("database"),
                                d.clone(),
                                Selection::Database(c.clone(), d.clone()),
                            )
                        })
                        .collect(),
                    None,
                ),
                _ => Content::Empty,
            },
            Selection::Database(c, d) => match sessions.dialect(c) {
                Some(dialect) if dialect.has_schemas() => match catalog.schemas(c, d) {
                    Some(Load::Ready(list)) => Content::Rows(
                        list.iter()
                            .map(|s| {
                                let scope = Scope::new(c.clone(), d.clone(), Some(s.clone()));
                                row(
                                    RowIcon::Lucide("layers"),
                                    s.clone(),
                                    Selection::Schema(scope),
                                )
                            })
                            .collect(),
                        None,
                    ),
                    Some(Load::Failed(e)) => Content::Message(e.clone()),
                    _ => Content::Message(t!("obj.loading").to_string().into()),
                },
                _ => objects(&Scope::new(c.clone(), d.clone(), None), ObjectKind::Table),
            },
            other => match listed_kind(other) {
                Some((scope, kind)) => objects(&scope, kind),
                None => Content::Empty,
            },
        }
    }

    fn selected_object(cx: &App) -> Option<ObjectRef> {
        match Workspace::global(cx).read(cx).selection() {
            Selection::Object(o) => Some(o.clone()),
            _ => None,
        }
    }

    fn on_row_click(target: Selection, e: &ClickEvent, window: &mut Window, cx: &mut App) {
        if target == Selection::None {
            return;
        }
        Workspace::global(cx).update(cx, |w, cx| w.select(target.clone(), cx));
        if e.click_count() < 2 {
            return;
        }
        match target {
            Selection::Object(object) => {
                window.dispatch_action(Box::new(OpenObject { object }), cx)
            }
            Selection::Connection(_) => window.dispatch_action(Box::new(OpenConnection), cx),
            _ => {}
        }
    }

    fn render_toolbar(
        &self,
        listed: Option<&(Scope, ObjectKind)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = palette(cx);
        let selected = Self::selected_object(cx);
        let dialect =
            listed.and_then(|(s, _)| Sessions::global(cx).read(cx).dialect(&s.connection));
        let kind = listed.map(|(_, k)| *k);
        let is_table = kind == Some(ObjectKind::Table);
        let droppable = kind
            .zip(dialect)
            .is_some_and(|(k, d)| d.drop_keyword(k).is_some());
        let button = |id: &'static str, icon: &'static str, label: String| {
            Button::new(id)
                .ghost()
                .small()
                .icon(lucide(icon))
                .label(label)
        };
        let view = self.view;
        h_flex()
            .h(tokens::object_toolbar::HEIGHT.rems())
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(p.separator)
            .child(
                button("obj-open", "folder-open", t!("obj.open").to_string())
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|_, _, window, cx| {
                        if let Some(object) = Self::selected_object(cx) {
                            window.dispatch_action(Box::new(OpenObject { object }), cx);
                        }
                    })),
            )
            .child(button("obj-design", "pencil", t!("obj.design").to_string()).disabled(true))
            .child(button("obj-new", "plus", t!("obj.new").to_string()).disabled(true))
            .child(
                button("obj-delete", "trash", t!("obj.delete").to_string())
                    .disabled(selected.is_none() || !droppable)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(DeleteObjects), cx)),
            )
            .when(is_table, |bar| {
                bar.child(
                    button("obj-empty", "eraser", t!("obj.empty").to_string())
                        .disabled(selected.is_none())
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(EmptyTables), cx)
                        }),
                )
                .when(dialect != Some(Dialect::Sqlite), |bar| {
                    bar.child(
                        button("obj-truncate", "scissors", t!("obj.truncate").to_string())
                            .disabled(selected.is_none())
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(TruncateTables), cx)
                            }),
                    )
                })
            })
            .child(div().flex_1())
            .when_some(kind.filter(|_| view == ViewMode::Detail), |bar, kind| {
                bar.child(
                    Button::new("choose-columns")
                        .ghost()
                        .small()
                        .icon(lucide("columns-3"))
                        .tooltip(t!("obj.choose_columns").to_string())
                        .on_click(move |e, window, cx| {
                            Self::columns_menu(kind, cx).show(e.position(), window, cx);
                        }),
                )
            })
            .child(
                h_flex()
                    .child(
                        Button::new("view-list")
                            .ghost()
                            .small()
                            .icon(lucide("list"))
                            .tooltip(t!("obj.list_view").to_string())
                            .when(view == ViewMode::List, |b| b.selected(true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view = ViewMode::List;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("view-detail")
                            .ghost()
                            .small()
                            .icon(lucide("table-properties"))
                            .tooltip(t!("obj.detail_view").to_string())
                            .when(view == ViewMode::Detail, |b| b.selected(true))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view = ViewMode::Detail;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn hidden_columns(kind: ObjectKind, cx: &App) -> Vec<String> {
        Settings::global(cx)
            .object_hidden_columns
            .get(kind.key())
            .cloned()
            .unwrap_or_default()
    }

    /// "Choose Columns": every property column of the listed kind, with a check mark when shown.
    fn columns_menu(kind: ObjectKind, cx: &App) -> NativeMenu {
        let hidden = Self::hidden_columns(kind, cx);
        let rows = match Self::content(cx) {
            Content::Rows(rows, _) => rows,
            _ => Vec::new(),
        };
        Self::detail_columns(&rows)
            .into_iter()
            .fold(NativeMenu::new(), |m, c| {
                let label = t!(format!("col.{c}")).to_string();
                m.menu_with_check(
                    label,
                    !hidden.contains(&c),
                    Box::new(ToggleObjectColumn { key: c.into() }),
                )
            })
    }

    fn detail_columns(rows: &[Row]) -> Vec<String> {
        let mut cols: Vec<String> = Vec::new();
        for r in rows {
            for (k, _) in &r.properties {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
        cols
    }

    fn sorted(&self, mut rows: Vec<Row>) -> Vec<Row> {
        let Some((key, asc)) = &self.sort else {
            return rows;
        };
        let value = |r: &Row| -> String {
            if key == "name" {
                r.label.to_string()
            } else {
                r.properties
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            }
        };
        rows.sort_by(|a, b| {
            let (va, vb) = (value(a), value(b));
            let ord = match (va.parse::<f64>(), vb.parse::<f64>()) {
                (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                _ => va.to_lowercase().cmp(&vb.to_lowercase()),
            };
            if *asc { ord } else { ord.reverse() }
        });
        rows
    }

    fn render_rows(
        &self,
        rows: Vec<Row>,
        detail_cols: Option<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let count = rows.len();
        let header = detail_cols.clone().map(|cols| {
            let sort = self.sort.clone();
            let cell = |key: String, width: tokens::Pt, cx: &mut Context<Self>| {
                let label = t!(format!("col.{key}")).to_string();
                let arrow = match &sort {
                    Some((k, asc)) if *k == key => {
                        if *asc {
                            " ▲"
                        } else {
                            " ▼"
                        }
                    }
                    _ => "",
                };
                div()
                    .id(SharedString::from(format!("th-{key}")))
                    .w(width.rems())
                    .flex_none()
                    .px_2()
                    .overflow_hidden()
                    .child(format!("{label}{arrow}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sort = match &this.sort {
                            Some((k, true)) if *k == key => Some((key.clone(), false)),
                            Some((k, false)) if *k == key => None,
                            _ => Some((key.clone(), true)),
                        };
                        cx.notify();
                    }))
            };
            let mut header = h_flex()
                .h(tokens::list::HEADER_HEIGHT.rems())
                .border_b_1()
                .border_color(p.separator)
                .text_color(p.label_secondary)
                .child(cell("name".into(), tokens::list::NAME_COLUMN_WIDTH, cx));
            for c in cols {
                header = header.child(cell(c, tokens::list::DETAIL_COLUMN_WIDTH, cx));
            }
            header
        });
        let list = uniform_list(
            "objects-rows",
            count,
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let p = palette(cx);
                range
                    .map(|ix| {
                        let row = &rows[ix];
                        let selected = row.target == selection;
                        let icon = match &row.icon {
                            RowIcon::Driver(d) => img(driver_icon(d))
                                .size(tokens::navigator::ROW_ICON.rems())
                                .into_any_element(),
                            RowIcon::Folder => lucide("folder")
                                .size(tokens::navigator::ROW_ICON.rems())
                                .text_color(p.label_secondary)
                                .into_any_element(),
                            RowIcon::Lucide(name) => lucide(name)
                                .size(tokens::navigator::ROW_ICON.rems())
                                .text_color(p.label_secondary)
                                .into_any_element(),
                        };
                        let target = row.target.clone();
                        let menu_target = row.target.clone();
                        let name = h_flex()
                            .px_2()
                            .pl(tokens::navigator::ROW_INSET_X.rems()
                                + tokens::navigator::ROW_INDENT.rems() * row.depth as f32)
                            .gap_2()
                            .min_w_0()
                            .overflow_hidden()
                            .child(icon)
                            .child(truncated(("obj-name", ix), row.label.clone()));
                        // In list view the name fills the row; in detail view it is a fixed-width
                        // first column
                        let name = match detail_cols {
                            Some(_) => name.w(tokens::list::NAME_COLUMN_WIDTH.rems()).flex_none(),
                            None => name.flex_1(),
                        };
                        let mut line = h_flex()
                            .id(ElementId::NamedInteger("obj-row".into(), ix as u64))
                            .w_full()
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .when(selected, |r| r.bg(p.selection).text_color(p.selection_text))
                            .child(name)
                            .on_click(move |e, window, cx| {
                                Self::on_row_click(target.clone(), e, window, cx)
                            })
                            .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                                cx.stop_propagation();
                                if menu_target == Selection::None {
                                    return;
                                }
                                if !Workspace::global(cx)
                                    .read(cx)
                                    .selections()
                                    .any(|s| *s == menu_target)
                                {
                                    Workspace::global(cx)
                                        .update(cx, |w, cx| w.select(menu_target.clone(), cx));
                                }
                                let menu = match &menu_target {
                                    Selection::Object(o) => {
                                        crate::navigator::NavigatorPanel::object_menu(o, cx)
                                    }
                                    other => crate::navigator::NavigatorPanel::context_menu(
                                        crate::navigator::Node::from_selection(other).as_ref(),
                                        cx,
                                    ),
                                };
                                menu.show(e.position, window, cx);
                            });
                        if let Some(cols) = &detail_cols {
                            for (ci, c) in cols.iter().enumerate() {
                                let v = row
                                    .properties
                                    .iter()
                                    .find(|(k, _)| k == c)
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or_default();
                                let id = SharedString::from(format!("obj-prop-{ix}-{ci}"));
                                line = line.child(
                                    truncated(id, v)
                                        .w(tokens::list::DETAIL_COLUMN_WIDTH.rems())
                                        .flex_none()
                                        .px_2(),
                                );
                            }
                        }
                        line
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1();
        v_flex()
            .flex_1()
            .min_h_0()
            .children(header)
            .child(list)
            .into_any_element()
    }
}

impl Render for ObjectsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A load that was wiped when the connection opened leaves an empty
        // slot. The pane then stays on "Loading…" because nothing asks again.
        if Self::stalled_load(cx) {
            cx.defer_in(window, |_, _, cx| Self::ensure_loaded(cx));
        }
        let p = palette(cx);
        let content = Self::content(cx);
        let (body, listed) = match content {
            Content::Empty => (
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(p.label_secondary)
                    .child(lucide("plug").size(tokens::conn_type::TILE_ICON.rems()))
                    .child(t!("obj.empty_hint").to_string())
                    .into_any_element(),
                None,
            ),
            Content::Message(m) => (
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(p.label_secondary)
                    .child(m)
                    .into_any_element(),
                None,
            ),
            Content::Rows(mut rows, listed) => {
                let detail = listed
                    .as_ref()
                    .filter(|_| self.view == ViewMode::Detail)
                    .map(|(_, kind)| {
                        let hidden = Self::hidden_columns(*kind, cx);
                        Self::detail_columns(&rows)
                            .into_iter()
                            .filter(|c| !hidden.contains(c))
                            .collect::<Vec<_>>()
                    });
                let rows = if detail.is_some() {
                    // The detail view is a flat, sortable list without virtual group headers
                    rows.retain(|r| r.target != Selection::None);
                    rows.iter_mut().for_each(|r| r.depth = 0);
                    self.sorted(rows)
                } else {
                    rows
                };
                (self.render_rows(rows, detail, cx), listed)
            }
        };
        v_flex()
            .id("objects")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.control_bg)
            .on_action({
                let kind = listed.as_ref().map(|(_, k)| *k);
                cx.listener(move |_, a: &ToggleObjectColumn, _, cx| {
                    let Some(kind) = kind else { return };
                    Settings::update(cx, |s| {
                        let hidden = s
                            .object_hidden_columns
                            .entry(kind.key().to_string())
                            .or_default();
                        match hidden.iter().position(|c| c.as_str() == a.key.as_ref()) {
                            Some(ix) => drop(hidden.remove(ix)),
                            None => hidden.push(a.key.to_string()),
                        }
                    });
                    cx.notify();
                })
            })
            .child(self.render_toolbar(listed.as_ref(), cx))
            .child(body)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    // Blank area: the category's menu (rows show their own and stop propagation)
                    let Some((scope, kind)) = &listed else { return };
                    window.focus(&this.focus_handle, cx);
                    let node = crate::navigator::Node::Category(scope.clone(), *kind);
                    crate::navigator::NavigatorPanel::context_menu(Some(&node), cx)
                        .show(e.position, window, cx);
                }),
            )
    }
}
