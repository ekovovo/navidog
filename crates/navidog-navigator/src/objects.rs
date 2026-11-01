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
use navidog_app_state::actions::{
    DeleteObjects, DesignTable, EmptyTables, NewQuery, NewTable, OpenConnection, OpenDefinition,
    OpenObject, RunQuery, TruncateTables,
};
use navidog_app_state::settings::Settings;
use navidog_app_state::store::ConnectionStore;
use navidog_app_state::vgroups::VirtualGroups;
use navidog_app_state::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::ObjectInfo;
use navidog_model::object::{ObjectKind, ObjectRef, Scope};
use navidog_session::sessions::{ConnState, Sessions};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{skeleton_rows, truncated};
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load, listed_kind};
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
    /// Selection and row count from the last render. The status bar reads this
    /// instead of rebuilding every row (and cloning every property) on each
    /// frame, but only while the selection still matches.
    last_rows: Option<(Selection, usize)>,
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
    /// Catalog or connection is still arriving; the pane draws skeleton rows.
    Loading,
    Message(SharedString),
    Empty,
}

/// Redraws the objects pane after a View menu option changes.
pub(crate) fn refresh(cx: &mut App) {
    if let Some(panel) = cx.try_global::<SharedObjects>().and_then(|s| s.0.upgrade()) {
        panel.update(cx, |_, cx| cx.notify());
    }
}

/// Joins already-labeled property pairs into the one-line table profile.
pub(crate) fn join_profile(parts: &[(String, String)]) -> String {
    parts
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn displayed_property(key: &str, value: &str) -> String {
    if key == "data_length" {
        navidog_util::format::format_bytes(value)
    } else {
        value.to_string()
    }
}

fn profile_line(properties: &[(String, String)]) -> String {
    let parts = properties
        .iter()
        .map(|(k, v)| (t!(format!("col.{k}")).to_string(), displayed_property(k, v)))
        .collect::<Vec<_>>();
    join_profile(&parts)
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
            false,
        )
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
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| {
                    Self::defer_recount(cx);
                    cx.notify();
                }),
                cx.observe(&Sessions::global(cx), |_, _, cx| {
                    Self::defer_recount(cx);
                    cx.notify();
                }),
                cx.observe(&Catalog::global(cx), |_, _, cx| {
                    Self::defer_recount(cx);
                    cx.notify();
                }),
                cx.subscribe(&Workspace::global(cx), |_, _, e: &WorkspaceEvent, cx| {
                    if !matches!(e, WorkspaceEvent::SelectionChanged) {
                        return;
                    }
                    Self::defer_recount(cx);
                    cx.notify();
                }),
            ];
            Self {
                focus_handle: cx.focus_handle(),
                view: ViewMode::Detail,
                sort: None,
                scroll: UniformListScrollHandle::new(),
                last_rows: None,
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedObjects(panel.downgrade()));
        panel
    }

    /// Recomputes the count after the current update finishes. `content` reads
    /// the navigator's view options, which is not re-entrant when the navigator
    /// itself triggered the workspace/catalog notification.
    fn defer_recount(cx: &mut Context<Self>) {
        let panel = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Some(panel) = panel.upgrade() {
                panel.update(cx, |this, cx| {
                    this.recount(cx);
                    cx.notify();
                });
            }
        });
    }

    fn recount(&mut self, cx: &App) {
        let selection = Workspace::global(cx).read(cx).selection().clone();
        self.last_rows = match Self::content(cx) {
            Content::Rows(mut rows, listed) => {
                if listed.is_some() && self.view == ViewMode::Detail {
                    rows.retain(|row| row.target != Selection::None);
                }
                Some((selection, rows.len()))
            }
            _ => None,
        };
    }

    /// Recomputes the status-bar count after a setting change that does not move the selection.
    pub fn refresh_count(cx: &mut App) {
        let Some(panel) = cx
            .try_global::<SharedObjects>()
            .and_then(|shared| shared.0.upgrade())
        else {
            return;
        };
        panel.update(cx, |this, cx| {
            this.recount(cx);
            cx.notify();
        });
    }

    fn hidden_selection(selection: &Selection, sessions: &Sessions, show_system: bool) -> bool {
        if show_system {
            return false;
        }
        let scope_hidden = |scope: &Scope| {
            let Some(dialect) = sessions.dialect(&scope.connection) else {
                return false;
            };
            dialect.is_system_database(&scope.database)
                || scope
                    .schema
                    .as_deref()
                    .is_some_and(|schema| dialect.is_system_schema(schema))
        };
        match selection {
            Selection::Database(connection, name) => sessions
                .dialect(connection)
                .is_some_and(|dialect| dialect.is_system_database(name)),
            Selection::Schema(scope) | Selection::Category(scope, _) => scope_hidden(scope),
            Selection::Object(object) => {
                scope_hidden(&object.scope)
                    || sessions
                        .dialect(&object.scope.connection)
                        .is_some_and(|dialect| dialect.is_system_object(&object.name))
            }
            _ => false,
        }
    }

    /// Drops workspace selections that the "show system objects" switch just hid, so the
    /// objects toolbar cannot still delete or design them.
    pub fn drop_hidden_selections(cx: &mut App) {
        if Settings::global(cx).show_system_objects {
            return;
        }
        let (primary, rest) = {
            let workspace = Workspace::global(cx).read(cx);
            let primary = workspace.selection().clone();
            let rest: Vec<_> = workspace.selections().skip(1).cloned().collect();
            let sessions = Sessions::global(cx).read(cx);
            let hidden = |selection: &Selection| Self::hidden_selection(selection, sessions, false);
            if !hidden(&primary) && rest.iter().all(|selection| !hidden(selection)) {
                return;
            }
            let rest: Vec<_> = rest
                .into_iter()
                .filter(|selection| !hidden(selection))
                .collect();
            if hidden(&primary) {
                let mut rest = rest.into_iter();
                (rest.next().unwrap_or(Selection::None), rest.collect())
            } else {
                (primary, rest)
            }
        };
        Workspace::global(cx).update(cx, |workspace, cx| {
            workspace.select_many(primary, rest, cx);
        });
    }

    /// The row count of the last render, or `None` until the panel rendered once.
    pub(crate) fn cached_rows(cx: &App) -> Option<usize> {
        let current = Workspace::global(cx).read(cx).selection().clone();
        let panel = cx.try_global::<SharedObjects>()?.0.upgrade()?;
        let panel = panel.read(cx);
        panel
            .last_rows
            .as_ref()
            .filter(|(selection, _)| *selection == current)
            .map(|(_, rows)| *rows)
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
        let show_system = Settings::global(cx).show_system_objects;
        let system_object = |connection: &str, name: &str| {
            !show_system
                && sessions
                    .dialect(connection)
                    .is_some_and(|dialect| dialect.is_system_object(name))
        };

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
                let visible = |name: &str| !system_object(&scope.connection, name);
                if grouping {
                    for g in vgroups.groups_for(scope, kind) {
                        let members: Vec<_> = g
                            .members
                            .iter()
                            .filter(|m| visible(m))
                            .filter_map(|m| list.iter().find(|o| &o.name == m))
                            .collect();
                        let only_hidden = members.is_empty()
                            && g.members.iter().any(|m| {
                                list.iter()
                                    .any(|o| &o.name == m && system_object(&scope.connection, m))
                            });
                        if only_hidden {
                            continue;
                        }
                        rows.push(row(RowIcon::Folder, g.name.clone(), Selection::None));
                        for o in members {
                            grouped.insert(o.name.as_str());
                            rows.push(object_row(o, 1));
                        }
                    }
                }
                rows.extend(
                    list.iter()
                        .filter(|o| visible(&o.name) && !grouped.contains(o.name.as_str()))
                        .map(|o| object_row(o, 0)),
                );
                Content::Rows(rows, Some((scope.clone(), kind)))
            }
            Some(Load::Failed(e)) => Content::Message(e.clone()),
            _ => Content::Loading,
        };

        let conn = selection.connection_id().cloned();
        if let Some(c) = &conn {
            match sessions.state(c) {
                ConnState::Open { .. } => {}
                ConnState::Opening => return Content::Loading,
                ConnState::Failed(e) => return Content::Message(e.clone().into()),
                ConnState::Closed => {
                    return Content::Message(t!("obj.not_connected").to_string().into());
                }
            }
        }
        if Self::hidden_selection(&selection, sessions, show_system) {
            return Content::Empty;
        }
        match &selection {
            Selection::None => connections(None),
            Selection::Group(g) => connections(Some(g)),
            Selection::Connection(c) => match sessions.state(c) {
                ConnState::Open { databases, .. } => Content::Rows(
                    databases
                        .iter()
                        .filter(|d| {
                            show_system
                                || !sessions
                                    .dialect(c)
                                    .is_some_and(|dialect| dialect.is_system_database(d))
                        })
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
                Some(dialect) if dialect.features().has_schemas => match catalog.schemas(c, d) {
                    Some(Load::Ready(list)) => Content::Rows(
                        list.iter()
                            .filter(|s| show_system || !dialect.is_system_schema(s))
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
                    _ => Content::Loading,
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
        let is_query = kind == Some(ObjectKind::Query);
        let droppable = is_query
            || kind
                .zip(dialect.clone())
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
                            // "Open Query" opens and runs it (30 §7)
                            if object.kind == ObjectKind::Query {
                                window.dispatch_action(Box::new(RunQuery { object }), cx);
                            } else {
                                window.dispatch_action(Box::new(OpenObject { object }), cx);
                            }
                        }
                    })),
            )
            .child(
                button("obj-design", "pencil", t!("obj.design").to_string())
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|_, _, window, cx| {
                        if let Some(object) = Self::selected_object(cx) {
                            if object.kind == ObjectKind::Table {
                                window.dispatch_action(
                                    Box::new(DesignTable {
                                        object: Some(object),
                                    }),
                                    cx,
                                );
                            } else {
                                window.dispatch_action(Box::new(OpenObject { object }), cx);
                            }
                        }
                    })),
            )
            .child(
                button("obj-new", "plus", t!("obj.new").to_string())
                    .disabled(
                        listed.is_none()
                            || !(is_query
                                || is_table
                                || kind.is_some_and(|k| {
                                    navidog_sql::definition::DefinitionKind::from_object(k)
                                        .is_some()
                                })),
                    )
                    .on_click({
                        let listed = listed.cloned();
                        move |_, window, cx| {
                            if is_table {
                                window.dispatch_action(Box::new(NewTable), cx);
                            } else if is_query {
                                window.dispatch_action(Box::new(NewQuery), cx);
                            } else if let Some((scope, kind)) = listed.clone() {
                                window.dispatch_action(
                                    Box::new(OpenDefinition {
                                        scope,
                                        kind: kind.key().into(),
                                        name: None,
                                    }),
                                    cx,
                                );
                            }
                        }
                    }),
            )
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
                .when(
                    dialect.as_ref().is_none_or(|d| !d.features().exposes_rowid),
                    |bar| {
                        bar.child(
                            button("obj-truncate", "scissors", t!("obj.truncate").to_string())
                                .disabled(selected.is_none())
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(TruncateTables), cx)
                                }),
                        )
                    },
                )
            })
            .child(div().flex_1())
            .when_some(kind.filter(|_| view == ViewMode::Detail), |bar, kind| {
                bar.child(
                    Button::new("choose-columns")
                        .ghost()
                        .small()
                        .icon(lucide("columns-3"))
                        .tooltip(t!("obj.choose_columns").to_string())
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !this.focus_handle.contains_focused(window, cx) {
                                window.focus(&this.focus_handle, cx);
                            }
                            Self::columns_menu(kind, cx).show(e.position(), window, cx);
                        })),
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
                                this.recount(cx);
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
                                this.recount(cx);
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
                            RowIcon::Driver(d) => img(navidog_session::DriverRegistry::icon(cx, d))
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
                                    .map(|(_, v)| displayed_property(c, v))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let content = Self::content(cx);
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let show_profile = crate::navigator::view_options(cx).show_table_profile;
        let mut profile = None;
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
            Content::Loading => (
                div()
                    .flex_1()
                    .min_h_0()
                    .px(tokens::navigator::ROW_INSET_X.rems())
                    .py_1()
                    .child(skeleton_rows(8, tokens::list::ROW_HEIGHT.rems(), None))
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
                if show_profile {
                    profile = rows.iter().find(|r| r.target == selection).and_then(|r| {
                        (!r.properties.is_empty()).then(|| profile_line(&r.properties))
                    });
                }
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
            .when_some(profile, |el, text| {
                el.child(
                    h_flex()
                        .id("table-profile")
                        .w_full()
                        .h(tokens::list::ROW_HEIGHT.rems())
                        .px_2()
                        .min_w_0()
                        .flex_none()
                        .overflow_hidden()
                        .border_t_1()
                        .border_color(p.separator)
                        .text_color(p.label_secondary)
                        .items_center()
                        .child(truncated("table-profile-text", text)),
                )
            })
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

#[cfg(test)]
mod tests {
    use super::join_profile;

    #[test]
    fn profile_joins_labeled_properties() {
        let parts = vec![
            ("行".into(), "200".into()),
            ("数据长度".into(), "16384".into()),
            ("引擎".into(), "InnoDB".into()),
        ];
        assert_eq!(
            join_profile(&parts),
            "行: 200 · 数据长度: 16384 · 引擎: InnoDB"
        );
        assert_eq!(join_profile(&[]), "");
    }
}
