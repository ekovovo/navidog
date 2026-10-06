//! 对象选项卡（20-objects.md 第 4 节）：对象工具栏、列表视图、详细信息视图。

use std::cmp::Ordering;
use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Global, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    UniformListScrollHandle, WeakEntity, Window, div, img, uniform_list,
};
use navidog_core::actions::{OpenConnection, OpenObject};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace, WorkspaceEvent};
use navidog_db::ObjectInfo;
use navidog_db::dialect::Dialect;
use navidog_db::sessions::{ConnState, Sessions};
use navidog_ui::assets::{driver_icon, lucide};
use navidog_ui::{palette, tokens};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    List,
    Detail,
}

pub struct ObjectsPanel {
    focus_handle: FocusHandle,
    view: ViewMode,
    /// 详细信息视图的排序：(列键, 是否升序)
    sort: Option<(String, bool)>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// 列表中的一项。
#[derive(Clone)]
pub(crate) struct Row {
    icon: RowIcon,
    label: SharedString,
    target: Selection,
    properties: Vec<(String, String)>,
}

#[derive(Clone)]
enum RowIcon {
    Driver(String),
    Folder,
    Lucide(&'static str),
}

/// 当前选择对应的列表内容。
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
}

impl Panel for ObjectsPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        t!("obj.objects_tab").to_string()
    }
}

/// 选择所在作用域要显示的对象分类：选中分类或对象时用它的分类，否则默认显示表。
fn listed_kind(selection: &Selection) -> Option<(Scope, ObjectKind)> {
    match selection {
        Selection::Category(s, k) => Some((s.clone(), *k)),
        Selection::Object(o) => Some((o.scope.clone(), o.kind)),
        Selection::Schema(s) => Some((s.clone(), ObjectKind::Table)),
        _ => None,
    }
}

impl ObjectsPanel {
    /// Dock 恢复布局时复用同一实例。
    pub fn shared(_: &mut Window, cx: &mut App) -> Entity<Self> {
        if let Some(panel) = cx.try_global::<SharedObjects>().and_then(|s| s.0.upgrade()) {
            return panel;
        }
        let panel = cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Sessions::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Catalog::global(cx), |_, _, cx| cx.notify()),
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

    /// 选择变化时加载对象列表（MySQL / SQLite 选中数据库即列出表）。
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
            Some(Load::Ready(list)) => Content::Rows(
                list.iter()
                    .map(|o: &ObjectInfo| Row {
                        icon: RowIcon::Lucide(kind_icon(kind)),
                        label: o.name.clone().into(),
                        target: Selection::Object(ObjectRef {
                            scope: scope.clone(),
                            kind,
                            name: o.name.clone(),
                        }),
                        properties: o.properties.clone(),
                    })
                    .collect(),
                Some((scope.clone(), kind)),
            ),
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

    /// 删除、清空、截断：确认后在元数据会话上执行，成功后刷新对象列表。
    fn confirm_sql(
        &self,
        obj: ObjectRef,
        message: String,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (obj, sql) = (obj.clone(), sql.clone());
            alert
                .icon(lucide("triangle-alert"))
                .title(message.clone())
                .show_cancel(true)
                .ok_text(t!("obj.ok").to_string())
                .cancel_text(t!("obj.cancel").to_string())
                .on_ok(move |_, window, cx| {
                    run_and_refresh(obj.clone(), sql.clone(), window, cx);
                    true
                })
        });
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
        let droppable = kind.is_some_and(|k| drop_keyword(k, dialect).is_some());
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(obj) = Self::selected_object(cx) else {
                            return;
                        };
                        let Some(keyword) = drop_keyword(obj.kind, dialect) else {
                            return;
                        };
                        let d = dialect.unwrap_or(Dialect::MySql);
                        let sql = format!("DROP {keyword} {}", d.qualified(&obj.scope, &obj.name));
                        let kind = t!(format!("kind.{}", obj.kind.key())).to_string();
                        let msg = t!("obj.confirm_drop", kind = kind, name = obj.name).to_string();
                        this.confirm_sql(obj, msg, sql, window, cx);
                    })),
            )
            .when(is_table, |bar| {
                bar.child(
                    button("obj-empty", "eraser", t!("obj.empty").to_string())
                        .disabled(selected.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let Some(obj) = Self::selected_object(cx) else {
                                return;
                            };
                            let d = dialect.unwrap_or(Dialect::MySql);
                            let sql = format!("DELETE FROM {}", d.qualified(&obj.scope, &obj.name));
                            let msg = t!("obj.confirm_empty", name = obj.name).to_string();
                            this.confirm_sql(obj, msg, sql, window, cx);
                        })),
                )
                .when(dialect != Some(Dialect::Sqlite), |bar| {
                    bar.child(
                        button("obj-truncate", "scissors", t!("obj.truncate").to_string())
                            .disabled(selected.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(obj) = Self::selected_object(cx) else {
                                    return;
                                };
                                let d = dialect.unwrap_or(Dialect::MySql);
                                let sql = format!(
                                    "TRUNCATE TABLE {}",
                                    d.qualified(&obj.scope, &obj.name)
                                );
                                let msg = t!("obj.confirm_truncate", name = obj.name).to_string();
                                this.confirm_sql(obj, msg, sql, window, cx);
                            })),
                    )
                })
            })
            .child(div().flex_1())
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
                        let name = h_flex()
                            .px_2()
                            .gap_2()
                            .overflow_hidden()
                            .child(icon)
                            .child(row.label.clone());
                        // 列表视图名称占满整行；详细信息视图名称为固定宽度的第一列
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
                            });
                        if let Some(cols) = &detail_cols {
                            for c in cols {
                                let v = row
                                    .properties
                                    .iter()
                                    .find(|(k, _)| k == c)
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or_default();
                                line = line.child(
                                    div()
                                        .w(tokens::list::DETAIL_COLUMN_WIDTH.rems())
                                        .flex_none()
                                        .px_2()
                                        .overflow_hidden()
                                        .child(v),
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

/// `DROP` 关键字；不能删除（或需要参数签名才能定位）的分类返回 `None`。
fn drop_keyword(kind: ObjectKind, dialect: Option<Dialect>) -> Option<&'static str> {
    Some(match kind {
        ObjectKind::Table => "TABLE",
        ObjectKind::View => "VIEW",
        ObjectKind::MaterializedView => "MATERIALIZED VIEW",
        ObjectKind::Event => "EVENT",
        ObjectKind::Trigger if dialect == Some(Dialect::Sqlite) => "TRIGGER",
        ObjectKind::Index if dialect == Some(Dialect::Sqlite) => "INDEX",
        ObjectKind::Sequence => "SEQUENCE",
        _ => return None,
    })
}

fn run_and_refresh(obj: ObjectRef, sql: String, window: &mut Window, cx: &mut App) {
    let Some(job) = Sessions::global(cx).read(cx).run_rel(
        &obj.scope.connection,
        Some(&obj.scope.database),
        move |r| Box::pin(async move { r.execute(&sql).await.map(|_| ()) }),
    ) else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = cx.update(|window, cx| {
                Catalog::global(cx).update(cx, |cat, cx| {
                    cat.ensure_objects(&obj.scope, obj.kind, true, cx)
                });
                if let Err(e) = result {
                    window.open_alert_dialog(cx, move |alert, _, _| {
                        alert
                            .icon(lucide("circle-x"))
                            .title(t!("obj.failed").to_string())
                            .description(e.clone())
                    });
                }
            });
        })
        .detach();
}

impl Render for ObjectsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            Content::Rows(rows, listed) => {
                let detail = (self.view == ViewMode::Detail && listed.is_some())
                    .then(|| Self::detail_columns(&rows));
                let rows = if detail.is_some() {
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
            .child(self.render_toolbar(listed.as_ref(), cx))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_keywords_per_dialect() {
        assert_eq!(
            drop_keyword(ObjectKind::Table, Some(Dialect::MySql)),
            Some("TABLE")
        );
        assert_eq!(
            drop_keyword(ObjectKind::Index, Some(Dialect::Sqlite)),
            Some("INDEX")
        );
        assert_eq!(drop_keyword(ObjectKind::Index, Some(Dialect::MySql)), None);
        assert_eq!(
            drop_keyword(ObjectKind::Function, Some(Dialect::Postgres)),
            None
        );
    }
}
