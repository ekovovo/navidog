//! Read-only DDL tab: until the M4 designers, double-clicking a non-table object opens this.

use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    WeakEntity, Window,
};
use navidog_core::object::ObjectRef;
use navidog_db::sessions::{Sessions, SessionsEvent};
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tabs};

use crate::catalog::{Catalog, Load};

const PANEL: &str = "DdlPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |ctx, window, cx| {
        let object = match ctx.info() {
            PanelInfo::Panel(v) => serde_json::from_value::<ObjectRef>(v.clone()).ok(),
            _ => None,
        };
        // A corrupt layout file must still yield a panel; use an empty object as placeholder
        let object = object.unwrap_or_else(|| ObjectRef {
            scope: navidog_core::object::Scope::new("", "", None),
            kind: navidog_core::object::ObjectKind::Table,
            name: String::new(),
        });
        panel_handle(DdlPanel::new(object, window, cx))
    });
}

pub struct DdlPanel {
    focus_handle: FocusHandle,
    object: ObjectRef,
    editor: Entity<EditorState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for DdlPanel {}

impl Focusable for DdlPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for DdlPanel {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(serde_json::to_value(&self.object).unwrap_or_default()),
        }
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

impl Panel for DdlPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = lucide(crate::navigator::kind_icon(self.object.kind));
        let store = navidog_core::store::ConnectionStore::global(cx);
        let color = store
            .read(cx)
            .get(&self.object.scope.connection)
            .and_then(|c| c.color.clone());
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(icon),
            self.object.name.clone(),
            color.and_then(|c| navidog_ui::tokens::connection_color(&c, cx)),
        )
    }
}

impl DdlPanel {
    pub fn new(object: ObjectRef, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let editor = crate::info::sql_editor(window, cx);
        cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.observe_in(&Catalog::global(cx), window, |this, _, window, cx| {
                    this.sync(window, cx)
                }),
                // The connection may still be closed when the layout is restored; load once it opens
                cx.subscribe(&Sessions::global(cx), |this, _, e: &SessionsEvent, cx| {
                    if matches!(e, SessionsEvent::Opened(id) if *id == this.object.scope.connection)
                    {
                        this.load(cx);
                    }
                }),
            ];
            let this = Self {
                focus_handle: cx.focus_handle(),
                object,
                editor,
                _subscriptions: subs,
            };
            this.load(cx);
            this
        })
    }

    fn load(&self, cx: &mut App) {
        let obj = self.object.clone();
        Catalog::global(cx).update(cx, |c, cx| c.ensure_ddl(&obj, false, cx));
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match Catalog::global(cx).read(cx).ddl(&self.object) {
            Some(Load::Ready(s)) => s.clone(),
            Some(Load::Failed(e)) => format!("-- {e}"),
            _ => return,
        };
        if self.editor.read(cx).value() != text.as_str() {
            self.editor
                .update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }
}

impl Render for DdlPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("ddl-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(palette(cx).control_bg)
            .child(Editor::new(&self.editor).bordered(false).size_full())
    }
}
