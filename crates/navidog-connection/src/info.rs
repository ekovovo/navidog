//! 信息窗格（10-shell.md 第 7 节，M1 只有“常规”页）。

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_ui::assets::lucide;
use navidog_ui::skin::heading;
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::sessions::{ConnState, Sessions};

const PANEL: &str = "InfoPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(InfoPanel::shared(window, cx))
    });
}

struct SharedInfo(gpui_kit::WeakEntity<InfoPanel>);
impl gpui_kit::Global for SharedInfo {}

type ToggleMaximize = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct InfoPanel {
    focus_handle: FocusHandle,
    maximized: bool,
    on_toggle_maximize: Option<ToggleMaximize>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for InfoPanel {}

impl Focusable for InfoPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for InfoPanel {
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

impl Panel for InfoPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        t!("info.general").to_string()
    }
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl InfoPanel {
    /// Dock 恢复布局时复用同一实例。
    pub fn shared(_: &mut Window, cx: &mut App) -> Entity<Self> {
        if let Some(panel) = cx.try_global::<SharedInfo>().and_then(|s| s.0.upgrade()) {
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
                maximized: false,
                on_toggle_maximize: None,
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedInfo(panel.downgrade()));
        panel
    }

    /// 由外壳提供“最大化/还原”的实现（调整右侧 dock 宽度）。
    pub fn set_on_toggle_maximize(&mut self, f: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_toggle_maximize = Some(Rc::new(f));
    }

    fn fields(cx: &App) -> Vec<(String, SharedString)> {
        let store = ConnectionStore::global(cx);
        let store = store.read(cx);
        let sessions = Sessions::global(cx);
        let sessions = sessions.read(cx);
        let conn_fields = |id: &str| -> Vec<(String, SharedString)> {
            let Some(c) = store.get(id) else {
                return Vec::new();
            };
            let mut f = vec![
                (t!("info.connection").to_string(), c.name.clone().into()),
                (
                    t!("info.type").to_string(),
                    crate::driver_name(&c.driver, cx),
                ),
            ];
            if c.supports_network() {
                f.push((t!("info.host").to_string(), c.general.host.clone().into()));
                f.push((
                    t!("info.port").to_string(),
                    c.general
                        .port
                        .map(|p| p.to_string())
                        .unwrap_or_default()
                        .into(),
                ));
                f.push((t!("info.user").to_string(), c.general.user.clone().into()));
            } else {
                f.push((
                    t!("info.file").to_string(),
                    c.general.file.clone().unwrap_or_default().into(),
                ));
            }
            match sessions.state(id) {
                ConnState::Open { version, .. } => {
                    f.push((
                        t!("info.server_version").to_string(),
                        version.clone().into(),
                    ));
                    f.push((
                        t!("info.status").to_string(),
                        t!("info.status_open").to_string().into(),
                    ));
                    f.push((
                        t!("info.sessions").to_string(),
                        sessions.session_count(id).to_string().into(),
                    ));
                }
                _ => f.push((
                    t!("info.status").to_string(),
                    t!("info.status_closed").to_string().into(),
                )),
            }
            f
        };
        match Workspace::global(cx).read(cx).selection() {
            Selection::None => Vec::new(),
            Selection::Group(g) => {
                let name = store
                    .groups()
                    .iter()
                    .find(|x| &x.id == g)
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                let count = store
                    .connections()
                    .iter()
                    .filter(|c| c.group.as_ref() == Some(g))
                    .count();
                vec![
                    (t!("info.group").to_string(), name.into()),
                    (t!("info.connections").to_string(), count.to_string().into()),
                ]
            }
            Selection::Connection(c) => conn_fields(c),
            Selection::Database(c, d) => {
                let mut f = vec![(t!("info.database").to_string(), d.clone().into())];
                f.extend(conn_fields(c));
                f
            }
        }
    }
}

impl Render for InfoPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let fields = Self::fields(cx);
        let body = if fields.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(p.label_secondary)
                .child(t!("info.nothing_selected").to_string())
                .into_any_element()
        } else {
            v_flex()
                .p(tokens::info::INSET.rems())
                .gap(tokens::info::ROW_GAP.rems())
                .children(fields.into_iter().map(|(k, v)| {
                    v_flex()
                        .child(
                            div()
                                .text_size(tokens::font::SIZE_SMALL.rems())
                                .text_color(p.label_secondary)
                                .child(k),
                        )
                        .child(div().text_color(p.label).child(v))
                }))
                .into_any_element()
        };
        v_flex()
            .id("info")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(p.window_bg)
            .child(
                h_flex()
                    .h(tokens::info::HEADER_HEIGHT.rems())
                    .px(tokens::info::HEADER_BUTTON_INSET.rems())
                    .justify_between()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(heading(t!("info.general").to_string()))
                    .child(
                        Button::new("maximize")
                            .ghost()
                            .xsmall()
                            .icon(lucide(if self.maximized {
                                "minimize-2"
                            } else {
                                "maximize-2"
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.maximized = !this.maximized;
                                if let Some(f) = this.on_toggle_maximize.clone() {
                                    f(window, cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(body)
    }
}
