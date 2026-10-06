//! 对象窗格的“对象”选项卡（10-shell.md 第 5 节，M1 部分）。

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::{Icon, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, img,
};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_ui::assets::driver_icon;
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::sessions::{ConnState, Sessions};

const PANEL: &str = "ObjectsPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(ObjectsPanel::shared(window, cx))
    });
}

struct SharedObjects(gpui_kit::WeakEntity<ObjectsPanel>);
impl gpui_kit::Global for SharedObjects {}

pub struct ObjectsPanel {
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
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

/// 列表中的一项。
struct Row {
    id: SharedString,
    icon: RowIcon,
    label: SharedString,
    target: Selection,
}

enum RowIcon {
    Driver(String),
    Folder,
    Database,
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
                cx.observe(&Workspace::global(cx), |_, _, cx| cx.notify()),
            ];
            Self {
                focus_handle: cx.focus_handle(),
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedObjects(panel.downgrade()));
        panel
    }

    fn rows(cx: &App) -> (Vec<Row>, Option<SharedString>) {
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        let sessions = Sessions::global(cx);
        let sessions = sessions.read(cx);
        let conn_rows = |group: Option<&str>| -> Vec<Row> {
            let groups = store
                .groups()
                .iter()
                .filter(|g| g.parent.as_deref() == group)
                .map(|g| Row {
                    id: format!("g-{}", g.id).into(),
                    icon: RowIcon::Folder,
                    label: g.name.clone().into(),
                    target: Selection::Group(g.id.clone()),
                });
            let conns = store
                .connections()
                .iter()
                .filter(|c| c.group.as_deref() == group)
                .map(|c| Row {
                    id: format!("c-{}", c.id).into(),
                    icon: RowIcon::Driver(c.driver.clone()),
                    label: c.name.clone().into(),
                    target: Selection::Connection(c.id.clone()),
                });
            groups.chain(conns).collect()
        };
        match Workspace::global(cx).read(cx).selection().clone() {
            Selection::None => (conn_rows(None), None),
            Selection::Group(g) => (conn_rows(Some(&g)), None),
            Selection::Connection(c) | Selection::Database(c, _) => match sessions.state(&c) {
                ConnState::Open { databases, .. } => (
                    databases
                        .iter()
                        .map(|d| Row {
                            id: format!("d-{d}").into(),
                            icon: RowIcon::Database,
                            label: d.clone().into(),
                            target: Selection::Database(c.clone(), d.clone()),
                        })
                        .collect(),
                    None,
                ),
                ConnState::Opening => (Vec::new(), Some(t!("obj.connecting").to_string().into())),
                ConnState::Failed(e) => (Vec::new(), Some(e.clone().into())),
                ConnState::Closed => (Vec::new(), Some(t!("obj.not_connected").to_string().into())),
            },
        }
    }
}

impl Render for ObjectsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let no_connections = ConnectionStore::global(cx)
            .read(cx)
            .connections()
            .is_empty();
        let selection = Workspace::global(cx).read(cx).selection().clone();
        let (rows, message) = Self::rows(cx);
        let content = if no_connections {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(p.label_secondary)
                .child(lucide("plug").size(tokens::conn_type::TILE_ICON.rems()))
                .child(t!("obj.empty_hint").to_string())
                .into_any_element()
        } else if let Some(m) = message {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(p.label_secondary)
                .child(m)
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .children(rows.into_iter().map(|row| {
                    let target = row.target.clone();
                    let selected = target == selection;
                    let icon = match &row.icon {
                        RowIcon::Driver(d) => img(driver_icon(d))
                            .size(tokens::navigator::ROW_ICON.rems())
                            .into_any_element(),
                        RowIcon::Folder => Icon::new(IconName::Folder)
                            .size(tokens::navigator::ROW_ICON.rems())
                            .text_color(p.label_secondary)
                            .into_any_element(),
                        RowIcon::Database => lucide("database")
                            .size(tokens::navigator::ROW_ICON.rems())
                            .text_color(p.label_secondary)
                            .into_any_element(),
                    };
                    h_flex()
                        .id(row.id)
                        .h(tokens::list::ROW_HEIGHT.rems())
                        .px_2()
                        .gap_2()
                        .when(selected, |r| r.bg(p.selection).text_color(p.selection_text))
                        .child(icon)
                        .child(row.label)
                        .on_click(move |e: &ClickEvent, window, cx| {
                            let target = target.clone();
                            if e.click_count() >= 2
                                && let Selection::Connection(id) = &target
                            {
                                crate::open_connection(id.clone(), window, cx);
                            }
                            Workspace::global(cx).update(cx, |w, cx| w.select(target, cx));
                        })
                }))
                .into_any_element()
        };
        v_flex()
            .id("objects")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.control_bg)
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .px_2()
                    .border_b_1()
                    .border_color(p.separator)
                    .text_color(p.label_secondary)
                    .child(t!("conn.col_name").to_string()),
            )
            .child(content)
            .child(div())
    }
}
