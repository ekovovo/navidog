//! Info pane (10-shell.md §7, 20-objects.md §6): general, DDL and fields pages.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, panel_handle, register_panel};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, WeakEntity, Window, div,
};
use navidog_core::object::{ObjectKind, ObjectRef};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::sessions::{ConnState, Sessions};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{heading, truncated};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load};

const PANEL: &str = "InfoPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(InfoPanel::shared(window, cx))
    });
}

struct SharedInfo(WeakEntity<InfoPanel>);
impl Global for SharedInfo {}

type ToggleMaximize = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Ddl,
    Fields,
}

pub struct InfoPanel {
    focus_handle: FocusHandle,
    page: Page,
    maximized: bool,
    on_toggle_maximize: Option<ToggleMaximize>,
    ddl: Entity<EditorState>,
    /// Object currently shown in the DDL editor
    shown_ddl: Option<ObjectRef>,
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

fn selected_object(cx: &App) -> Option<ObjectRef> {
    match Workspace::global(cx).read(cx).selection() {
        Selection::Object(o) => Some(o.clone()),
        _ => None,
    }
}

pub(crate) fn sql_editor(window: &mut Window, cx: &mut App) -> Entity<EditorState> {
    let editor = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("sql")
            .line_number(true)
    });
    editor.update(cx, |s, cx| s.set_readonly(true, cx));
    editor
}

impl InfoPanel {
    /// Reuses the same instance when the dock restores its layout.
    pub fn shared(window: &mut Window, cx: &mut App) -> Entity<Self> {
        if let Some(panel) = cx.try_global::<SharedInfo>().and_then(|s| s.0.upgrade()) {
            return panel;
        }
        let ddl = sql_editor(window, cx);
        let panel = cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Sessions::global(cx), |_, _, cx| cx.notify()),
                cx.observe_in(&Catalog::global(cx), window, |this, _, window, cx| {
                    this.sync_ddl(window, cx)
                }),
                cx.observe_in(&Workspace::global(cx), window, |this, _, window, cx| {
                    this.ensure_loaded(cx);
                    this.sync_ddl(window, cx);
                }),
            ];
            Self {
                focus_handle: cx.focus_handle(),
                page: Page::General,
                maximized: false,
                on_toggle_maximize: None,
                ddl,
                shown_ddl: None,
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedInfo(panel.downgrade()));
        panel
    }

    /// The shell implements maximize/restore (resizing the right dock).
    pub fn set_on_toggle_maximize(&mut self, f: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_toggle_maximize = Some(Rc::new(f));
    }

    fn ensure_loaded(&self, cx: &mut App) {
        let Some(obj) = selected_object(cx) else {
            return;
        };
        let catalog = Catalog::global(cx);
        match self.page {
            Page::Ddl => catalog.update(cx, |c, cx| c.ensure_ddl(&obj, false, cx)),
            Page::Fields if obj.kind == ObjectKind::Table => catalog.update(cx, |c, cx| {
                c.ensure_columns(&obj.scope, &obj.name, false, cx)
            }),
            _ => {}
        }
    }

    /// Writes the DDL into the editor after loading or a selection change (needs `window`, so not in render).
    fn sync_ddl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let obj = selected_object(cx);
        let text = obj
            .as_ref()
            .and_then(|o| match Catalog::global(cx).read(cx).ddl(o) {
                Some(Load::Ready(s)) => Some(s.clone()),
                Some(Load::Failed(e)) => Some(format!("-- {e}")),
                _ => None,
            });
        if obj == self.shown_ddl && text.is_none() {
            return;
        }
        let value = text.unwrap_or_default();
        if self.ddl.read(cx).value() != value.as_str() || obj != self.shown_ddl {
            self.ddl.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.shown_ddl = obj;
        cx.notify();
    }

    fn general_fields(cx: &App) -> Vec<(String, SharedString)> {
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
            if !c.profiles.is_empty() {
                let name = if c.profile.is_empty() {
                    t!("nav.profile_default").to_string()
                } else {
                    c.profile.clone()
                };
                f.push((t!("info.profile").to_string(), name.into()));
            }
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
        let selection = Workspace::global(cx).read(cx).selection().clone();
        match &selection {
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
            Selection::Object(o) => {
                let mut f = vec![
                    (t!("info.object").to_string(), o.name.clone().into()),
                    (
                        t!("info.type").to_string(),
                        t!(format!("kind.{}", o.kind.key())).to_string().into(),
                    ),
                    (
                        t!("info.database").to_string(),
                        o.scope.database.clone().into(),
                    ),
                ];
                if let Some(s) = &o.scope.schema {
                    f.push((t!("info.schema").to_string(), s.clone().into()));
                }
                if let Some(info) = Catalog::global(cx).read(cx).object(o) {
                    f.extend(
                        info.properties
                            .iter()
                            .map(|(k, v)| (t!(format!("col.{k}")).to_string(), v.clone().into())),
                    );
                }
                f.extend(conn_fields(&o.scope.connection).into_iter().take(1));
                f
            }
            other => {
                let scope = other.scope().expect("数据库、模式、分类都有作用域");
                let mut f = vec![(
                    t!("info.database").to_string(),
                    scope.database.clone().into(),
                )];
                if let Some(s) = &scope.schema {
                    f.push((t!("info.schema").to_string(), s.clone().into()));
                }
                f.extend(conn_fields(&scope.connection));
                f
            }
        }
    }

    fn render_general(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let fields = Self::general_fields(cx);
        if fields.is_empty() {
            return self.message(t!("info.nothing_selected").to_string(), cx);
        }
        v_flex()
            .p(tokens::info::INSET.rems())
            .gap(tokens::info::ROW_GAP.rems())
            .children(fields.into_iter().enumerate().map(|(ix, (k, v))| {
                v_flex()
                    .min_w_0()
                    .child(
                        truncated(("info-key", ix), k)
                            .text_size(tokens::font::SIZE_SMALL.rems())
                            .text_color(p.label_secondary),
                    )
                    .child(truncated(("info-value", ix), v).text_color(p.label))
            }))
            .child(
                h_flex().child(
                    Button::new("info-share")
                        .small()
                        .label(t!("info.share").to_string())
                        .on_click(|_, window, cx| Share::open(window, cx)),
                ),
            )
            .into_any_element()
    }

    fn render_fields(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let Some(obj) = selected_object(cx).filter(|o| o.kind == ObjectKind::Table) else {
            return self.message(t!("info.nothing_selected").to_string(), cx);
        };
        match Catalog::global(cx).read(cx).columns(&obj.scope, &obj.name) {
            Some(Load::Ready(cols)) => v_flex()
                .p(tokens::info::INSET.rems())
                .gap(tokens::info::ROW_GAP.rems())
                .children(cols.iter().map(|c| {
                    let mut tags = vec![c.type_name.clone()];
                    if c.primary_key {
                        tags.push(t!("info.primary_key").to_string());
                    }
                    if c.nullable {
                        tags.push(t!("info.nullable").to_string());
                    }
                    if let Some(d) = &c.default {
                        tags.push(format!("{}: {d}", t!("info.default")));
                    }
                    let id = SharedString::from(format!("info-field-{}", c.name));
                    v_flex()
                        .min_w_0()
                        .child(truncated(id.clone(), c.name.clone()).text_color(p.label))
                        .child(
                            truncated(SharedString::from(format!("{id}-tags")), tags.join("  ·  "))
                                .text_size(tokens::font::SIZE_SMALL.rems())
                                .text_color(p.label_secondary),
                        )
                        .children(c.comment.clone().map(|m| {
                            truncated(SharedString::from(format!("{id}-comment")), m)
                                .text_size(tokens::font::SIZE_SMALL.rems())
                                .text_color(p.label_tertiary)
                        }))
                }))
                .into_any_element(),
            Some(Load::Failed(e)) => self.message(e.to_string(), cx),
            _ => self.message(t!("nav.loading").to_string(), cx),
        }
    }

    fn message(&self, text: String, cx: &App) -> AnyElement {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_color(palette(cx).label_secondary)
            .child(text)
            .into_any_element()
    }

    fn page_button(
        &self,
        id: &'static str,
        icon: &'static str,
        page: Page,
        tooltip: String,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(lucide(icon))
            .tooltip(tooltip)
            .when(self.page == page, |b| b.selected(true))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.page = page;
                this.ensure_loaded(cx);
                this.sync_ddl(window, cx);
                cx.notify();
            }))
    }
}

impl Render for InfoPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let has_object = selected_object(cx).is_some();
        if !has_object && self.page != Page::General {
            self.page = Page::General;
        }
        let body = match self.page {
            Page::General => self.render_general(cx),
            Page::Ddl => div()
                .flex_1()
                .min_h_0()
                .child(Editor::new(&self.ddl).bordered(false).size_full())
                .into_any_element(),
            Page::Fields => self.render_fields(cx),
        };
        let is_table = selected_object(cx).is_some_and(|o| o.kind == ObjectKind::Table);
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
                    .child(
                        h_flex()
                            .gap_1()
                            .child(self.page_button(
                                "info-general",
                                "info",
                                Page::General,
                                t!("info.general").to_string(),
                                cx,
                            ))
                            .when(has_object, |r| {
                                r.child(self.page_button(
                                    "info-ddl",
                                    "code",
                                    Page::Ddl,
                                    t!("info.ddl").to_string(),
                                    cx,
                                ))
                            })
                            .when(is_table, |r| {
                                r.child(self.page_button(
                                    "info-fields",
                                    "columns-3",
                                    Page::Fields,
                                    t!("info.fields").to_string(),
                                    cx,
                                ))
                            })
                            .child(heading(match self.page {
                                Page::General => t!("info.general").to_string(),
                                Page::Ddl => t!("info.ddl").to_string(),
                                Page::Fields => t!("info.fields").to_string(),
                            })),
                    )
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

/// "Share" dialog (F-APP-015): the navidog URI of the selection, optionally without host and port.
struct Share {
    sel: Selection,
    hide_host: bool,
}

impl Share {
    fn open(window: &mut Window, cx: &mut App) {
        let sel = Workspace::global(cx).read(cx).selection().clone();
        if sel.connection_id().is_none() {
            return;
        }
        let view = cx.new(|_| Share {
            sel,
            hide_host: false,
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false)
                .child(view.clone())
        });
    }

    fn uri(&self, cx: &App) -> String {
        let conn = self
            .sel
            .connection_id()
            .and_then(|id| ConnectionStore::global(cx).read(cx).get(id));
        conn.map(|c| navidog_core::uri::of_selection(&self.sel, c, self.hide_host))
            .unwrap_or_default()
    }
}

impl Render for Share {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let uri = self.uri(cx);
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("info.share").to_string()))
            .child(
                div()
                    .id("share-uri")
                    .p_2()
                    .rounded_md()
                    .bg(p.fill_tertiary)
                    .text_color(p.label)
                    .child(uri.clone()),
            )
            .child(
                Checkbox::new("share-hide-host")
                    .label(t!("info.hide_host").to_string())
                    .checked(self.hide_host)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.hide_host = *checked;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("share-copy")
                            .label(t!("info.copy").to_string())
                            .on_click(move |_, window, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    uri.clone(),
                                ));
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("share-ok")
                            .primary()
                            .label(t!("info.ok").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}
