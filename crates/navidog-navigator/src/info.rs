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
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
};
use navidog_app_state::store::ConnectionStore;
use navidog_app_state::workspace::{Selection, Workspace};
use navidog_db::ColumnInfo;
use navidog_model::object::{ObjectKind, ObjectRef};
use navidog_session::sessions::{ConnState, Sessions};
use navidog_ui::assets::lucide;
use navidog_ui::skin::{heading, skeleton_rows, truncated};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::catalog::{Catalog, Load, Requirement};

const PANEL: &str = "InfoPanel";

pub(crate) fn init(cx: &mut App) {
    register_panel(cx, PANEL, |_, window, cx| {
        panel_handle(InfoPanel::shared(window, cx))
    });
}

struct SharedInfo(WeakEntity<InfoPanel>);
impl Global for SharedInfo {}

type ToggleMaximize = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    General,
    Ddl,
    Fields,
    /// A saved query's SQL (F-INFO-004)
    Preview,
    /// While a query editor is active (F-INFO-005, F-INFO-006)
    Snippets,
    Identifiers,
    /// Objects this one uses, and objects that use it.
    Used,
    /// Members, member-of, and privileges. Hidden until a user or role is a navigator object (43 §4).
    #[allow(dead_code)]
    Members,
}

pub struct InfoPanel {
    focus_handle: FocusHandle,
    pub(crate) page: Page,
    preview: Entity<EditorState>,
    shown_preview: Option<ObjectRef>,
    pub(crate) snippets: crate::snippets::SnippetsPage,
    pub(crate) identifiers: crate::identifiers::IdentifiersPage,
    /// The page shown before a query editor became active, to return to afterwards
    page_before_query: Option<Page>,
    in_query: bool,
    maximized: bool,
    on_toggle_maximize: Option<ToggleMaximize>,
    ddl: Entity<EditorState>,
    /// Object currently shown in the DDL editor
    shown_ddl: Option<ObjectRef>,
    relation: String,
    /// Object and page (membership or not) already loaded into `relation`.
    relation_key: Option<(ObjectRef, bool)>,
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
        t!("info.title").to_string()
    }
    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

/// The info pane shows the column focused in the table viewer, or every column when that
/// name is not on this table.
fn visible_columns<'a>(cols: &'a [ColumnInfo], focus: Option<&str>) -> Vec<&'a ColumnInfo> {
    if let Some(name) = focus {
        let hit: Vec<_> = cols.iter().filter(|c| c.name == name).collect();
        if !hit.is_empty() {
            return hit;
        }
    }
    cols.iter().collect()
}

fn selected_object(cx: &App) -> Option<ObjectRef> {
    match Workspace::global(cx).read(cx).selection() {
        Selection::Object(o) => Some(o.clone()),
        _ => None,
    }
}

fn section_label(section: navidog_sql::security::Section) -> String {
    match section {
        navidog_sql::security::Section::Uses => t!("info.uses").to_string(),
        navidog_sql::security::Section::UsedBy => t!("info.used_by").to_string(),
        navidog_sql::security::Section::Members => t!("info.members").to_string(),
        navidog_sql::security::Section::MemberOf => t!("info.member_of").to_string(),
        navidog_sql::security::Section::Privileges => t!("info.privileges").to_string(),
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
        let preview = sql_editor(window, cx);
        let snippets = crate::snippets::SnippetsPage::new(window, cx);
        let identifiers = crate::identifiers::IdentifiersPage::new(window, cx);
        let panel = cx.new(|cx: &mut Context<Self>| {
            let subs = vec![
                cx.observe(
                    &navidog_app_state::snippets::Snippets::global(cx),
                    |_, _, cx| cx.notify(),
                ),
                cx.observe(
                    &navidog_session::outline::OutlineCache::global(cx),
                    |_, _, cx| cx.notify(),
                ),
                cx.observe(&snippets.search, |_, _, cx| cx.notify()),
                cx.observe(&identifiers.search, |_, _, cx| cx.notify()),
                cx.observe(&ConnectionStore::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Sessions::global(cx), |_, _, cx| cx.notify()),
                cx.observe_in(&Catalog::global(cx), window, |this, _, window, cx| {
                    this.sync_ddl(window, cx);
                    cx.notify();
                }),
                cx.observe_in(&Workspace::global(cx), window, |this, _, window, cx| {
                    this.require_page(cx);
                    this.sync_ddl(window, cx);
                    this.sync_preview(window, cx);
                    this.follow_query_editor(cx);
                    this.sync_relation(cx);
                }),
            ];
            Self {
                focus_handle: cx.focus_handle(),
                page: Page::General,
                preview,
                shown_preview: None,
                snippets,
                identifiers,
                page_before_query: None,
                in_query: false,
                maximized: false,
                on_toggle_maximize: None,
                ddl,
                shown_ddl: None,
                relation: String::new(),
                relation_key: None,
                _subscriptions: subs,
            }
        });
        cx.set_global(SharedInfo(panel.downgrade()));
        panel
    }

    /// The pane if it exists already.
    pub(crate) fn shared_entity(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<SharedInfo>().and_then(|s| s.0.upgrade())
    }

    /// The shell implements maximize/restore (resizing the right dock).
    pub fn set_on_toggle_maximize(&mut self, f: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_toggle_maximize = Some(Rc::new(f));
    }

    /// Loads what the current page shows. The body only names the entry; the catalog decides
    /// whether that starts a query.
    fn require_page(&self, cx: &mut App) {
        let Some(obj) = selected_object(cx) else {
            return;
        };
        let requirement = match self.page {
            Page::Ddl => Requirement::Ddl(obj),
            Page::Fields if obj.kind == ObjectKind::Table => Requirement::Columns {
                scope: obj.scope,
                table: obj.name,
            },
            _ => return,
        };
        Catalog::global(cx).update(cx, |c, cx| c.require(requirement, cx));
    }

    /// Loads uses/used-by or members for the selected object. SQLite membership shows the
    /// missing-extension reason and does not run SQL.
    fn sync_relation(&mut self, cx: &mut Context<Self>) {
        let membership = match self.page {
            Page::Used => false,
            Page::Members => true,
            _ => return,
        };
        let Some(object) = selected_object(cx) else {
            self.relation.clear();
            self.relation_key = None;
            return;
        };
        let key = (object.clone(), membership);
        if self.relation_key.as_ref() == Some(&key) {
            return;
        }
        self.relation_key = Some(key);
        let Some(dialect) = Sessions::global(cx)
            .read(cx)
            .dialect(&object.scope.connection)
        else {
            self.relation.clear();
            return;
        };
        let plan = navidog_sql::security::relation_plan(
            dialect.as_ref(),
            dialect.definition_family(),
            &object,
            membership,
        );
        if let Some(note) = plan.note {
            self.relation = note.to_string();
            return;
        }
        if plan.queries.is_empty() {
            self.relation.clear();
            return;
        }
        let labeled: Vec<(String, String)> = plan
            .queries
            .into_iter()
            .map(|query| (section_label(query.section), query.sql))
            .collect();
        self.relation = "…".into();
        let database = if object.scope.database.is_empty() {
            None
        } else {
            Some(object.scope.database.clone())
        };
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .new_session(&object.scope.connection, database.as_deref())
        else {
            self.relation = t!("info.nothing_selected").to_string();
            return;
        };
        let expected = self.relation_key.clone();
        cx.spawn(async move |this, cx| {
            let result: navidog_db::DbResult<String> = async {
                let mut session = job
                    .await
                    .map_err(|e| navidog_db::DbError::msg(e.to_string()))??;
                let job = navidog_db::runtime::spawn(async move {
                    let result = async {
                        let r = session.relational().ok_or_else(|| {
                            navidog_db::DbError::msg(
                                "This connection is not a relational database.",
                            )
                        })?;
                        if let Some(database) = database.as_deref()
                            && let Some(sql) = r.dialect().prepare_database_sql(database)
                        {
                            r.execute(&sql).await?;
                        }
                        let mut out = String::new();
                        for (label, sql) in &labeled {
                            out.push_str(label);
                            out.push('\n');
                            match r.execute_read(sql).await {
                                Ok(sets) => {
                                    for set in sets {
                                        if set.rows.is_empty() {
                                            out.push_str("(0)\n");
                                        }
                                        for row in set.rows {
                                            for (i, cell) in row.iter().enumerate() {
                                                if i > 0 {
                                                    out.push('\t');
                                                }
                                                out.push_str(cell.as_deref().unwrap_or("NULL"));
                                            }
                                            out.push('\n');
                                        }
                                    }
                                }
                                Err(error) => {
                                    out.push_str(&error.to_string());
                                    out.push('\n');
                                }
                            }
                        }
                        Ok(out)
                    }
                    .await;
                    let _ = session.close().await;
                    result
                });
                job.await
                    .map_err(|e| navidog_db::DbError::msg(e.to_string()))?
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.relation_key != expected {
                    return;
                }
                this.relation = match result {
                    Ok(text) => text,
                    Err(error) => error.to_string(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// The snippets and identifiers pages come with an active query editor and go with it,
    /// returning to the page shown before (30 §6).
    fn follow_query_editor(&mut self, cx: &mut Context<Self>) {
        let in_query = Workspace::global(cx).read(cx).query_context().is_some();
        if in_query == self.in_query {
            if in_query && self.page == Page::Identifiers {
                self.ensure_identifiers(cx);
            }
            return;
        }
        self.in_query = in_query;
        if in_query {
            self.page_before_query = Some(self.page);
            self.page = Page::Snippets;
        } else if matches!(self.page, Page::Snippets | Page::Identifiers) {
            self.page = self.page_before_query.take().unwrap_or(Page::General);
        }
        cx.notify();
    }

    /// The selected saved query's text for the preview page.
    fn sync_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let obj = selected_object(cx).filter(|o| o.kind == ObjectKind::Query);
        if obj == self.shown_preview {
            return;
        }
        let text = obj
            .as_ref()
            .and_then(|o| navidog_model::queries::read(&o.scope, &o.name).ok())
            .unwrap_or_default();
        self.preview
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.shown_preview = obj;
    }

    pub(crate) fn show_snippets(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Snippets;
        cx.notify();
    }

    /// Menu items dispatch along the focused element; keep focus inside the pane.
    pub(crate) fn focus_for_menu(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.contains_focused(window, cx) {
            window.focus(&self.focus_handle, cx);
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
                    navidog_session::DriverRegistry::name(cx, &c.driver),
                ),
            ];
            if c.profiles.len() > 1 {
                let name = if c.active_profile_name().is_empty() {
                    t!("nav.profile_default").to_string()
                } else {
                    c.active_profile_name().to_string()
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
            Selection::Object(o) if o.kind == ObjectKind::Query => {
                let path = navidog_model::queries::path(&o.scope, &o.name);
                let mut f = vec![
                    (t!("info.object").to_string(), o.name.clone().into()),
                    (
                        t!("info.type").to_string(),
                        t!("kind.query").to_string().into(),
                    ),
                    (
                        t!("info.location").to_string(),
                        path.display().to_string().into(),
                    ),
                ];
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
                let scope = other
                    .scope()
                    .expect("database, schema and category have a scope");
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
            Some(Load::Ready(cols)) => {
                let focus = Workspace::global(cx)
                    .read(cx)
                    .focused_column()
                    .map(str::to_string);
                let shown = visible_columns(cols, focus.as_deref());
                v_flex()
                    .p(tokens::info::INSET.rems())
                    .gap(tokens::info::ROW_GAP.rems())
                    .children(shown.iter().map(|c| {
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
                                truncated(
                                    SharedString::from(format!("{id}-tags")),
                                    tags.join("  ·  "),
                                )
                                .text_size(tokens::font::SIZE_SMALL.rems())
                                .text_color(p.label_secondary),
                            )
                            .children(c.comment.clone().map(|m| {
                                truncated(SharedString::from(format!("{id}-comment")), m)
                                    .text_size(tokens::font::SIZE_SMALL.rems())
                                    .text_color(p.label_tertiary)
                            }))
                    }))
                    .into_any_element()
            }
            Some(Load::Failed(e)) => self.message(e.to_string(), cx),
            _ => div()
                .p(tokens::info::INSET.rems())
                .child(skeleton_rows(
                    6,
                    tokens::font::LINE_BODY.rems(),
                    Some(tokens::font::LINE_SMALL.rems()),
                ))
                .into_any_element(),
        }
    }

    pub(crate) fn message(&self, text: String, cx: &App) -> AnyElement {
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
            .flex_none()
            .icon(lucide(icon))
            .tooltip(tooltip)
            .when(self.page == page, |b| b.selected(true))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.page = page;
                this.require_page(cx);
                this.sync_ddl(window, cx);
                if page == Page::Identifiers {
                    this.ensure_identifiers(cx);
                }
                this.sync_relation(cx);
                cx.notify();
            }))
    }
}

impl Render for InfoPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let has_object = selected_object(cx).is_some();
        let is_query_object = selected_object(cx).is_some_and(|o| o.kind == ObjectKind::Query);
        let in_query = Workspace::global(cx).read(cx).query_context().is_some();
        let page_ok = match self.page {
            Page::General => true,
            Page::Ddl => has_object && !is_query_object,
            Page::Fields => has_object,
            Page::Preview => is_query_object,
            Page::Snippets | Page::Identifiers => in_query,
            Page::Used => has_object && !is_query_object,
            // Users and roles are not navigator objects yet (43 §4).
            Page::Members => false,
        };
        if !page_ok {
            self.page = Page::General;
        }
        let body = match self.page {
            Page::General => self.render_general(cx),
            Page::Ddl => {
                let loading = selected_object(cx).is_some_and(|object| {
                    !matches!(
                        Catalog::global(cx).read(cx).ddl(&object),
                        Some(Load::Ready(_)) | Some(Load::Failed(_))
                    )
                });
                if loading {
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(tokens::info::INSET.rems())
                        .child(skeleton_rows(6, tokens::font::LINE_BODY.rems(), None))
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(Editor::new(&self.ddl).bordered(false).size_full())
                        .into_any_element()
                }
            }
            Page::Fields => self.render_fields(cx),
            Page::Preview => div()
                .flex_1()
                .min_h_0()
                .child(Editor::new(&self.preview).bordered(false).size_full())
                .into_any_element(),
            Page::Snippets => self.render_snippets(cx),
            Page::Identifiers => self.render_identifiers(cx),
            Page::Used | Page::Members => v_flex()
                .id("info-relation")
                .flex_1()
                .min_h_0()
                .p(tokens::info::INSET.rems())
                .gap_1()
                .overflow_y_scroll()
                .children(
                    self.relation
                        .lines()
                        .map(|line| div().child(line.to_string())),
                )
                .into_any_element(),
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
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(p.separator)
                    .child(
                        // The page buttons must not shrink, and this group must
                        // be allowed to shrink below its content: otherwise a
                        // narrow panel (or five page buttons in a query) pushes
                        // the maximize button past the panel's right edge and
                        // it looks like it disappeared. The heading, being
                        // last in the group, is what gets clipped instead.
                        h_flex()
                            .gap_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(self.page_button(
                                "info-general",
                                "info",
                                Page::General,
                                t!("info.general").to_string(),
                                cx,
                            ))
                            .when(has_object && !is_query_object, |r| {
                                r.child(self.page_button(
                                    "info-ddl",
                                    "code",
                                    Page::Ddl,
                                    t!("info.ddl").to_string(),
                                    cx,
                                ))
                            })
                            .when(is_query_object, |r| {
                                r.child(self.page_button(
                                    "info-preview",
                                    "file-code",
                                    Page::Preview,
                                    t!("info.preview").to_string(),
                                    cx,
                                ))
                            })
                            .when(in_query, |r| {
                                r.child(self.page_button(
                                    "info-snippets",
                                    "scroll-text",
                                    Page::Snippets,
                                    t!("info.snippets").to_string(),
                                    cx,
                                ))
                                .child(self.page_button(
                                    "info-identifiers",
                                    "list-tree",
                                    Page::Identifiers,
                                    t!("info.identifiers").to_string(),
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
                            .when(has_object && !is_query_object, |r| {
                                r.child(self.page_button(
                                    "info-used",
                                    "list",
                                    Page::Used,
                                    t!("info.used").to_string(),
                                    cx,
                                ))
                            })
                            .child(div().min_w_0().overflow_hidden().child(heading(
                                match self.page {
                                    Page::General => t!("info.general").to_string(),
                                    Page::Ddl => t!("info.ddl").to_string(),
                                    Page::Fields => t!("info.fields").to_string(),
                                    Page::Preview => t!("info.preview").to_string(),
                                    Page::Snippets => t!("info.snippets").to_string(),
                                    Page::Identifiers => t!("info.identifiers").to_string(),
                                    Page::Used => t!("info.used").to_string(),
                                    Page::Members => t!("info.members").to_string(),
                                },
                            ))),
                    )
                    .child(
                        Button::new("maximize")
                            .ghost()
                            .xsmall()
                            .flex_none()
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
        conn.map(|c| navidog_app_state::uri::of_selection(&self.sel, c, self.hide_host))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            type_name: "int".into(),
            nullable: true,
            primary_key: false,
            default: None,
            comment: None,
        }
    }

    #[test]
    fn fields_page_follows_the_focused_column() {
        let cols = [col("id"), col("name")];
        assert_eq!(
            visible_columns(&cols, Some("name"))
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["name"]
        );
        assert_eq!(visible_columns(&cols, None).len(), 2);
        assert_eq!(visible_columns(&cols, Some("missing")).len(), 2);
    }
}
