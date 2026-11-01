//! One designer for views, routines, events, databases and the other definition objects.
//! The statement text comes from `navidog-schema`; this panel only edits the draft.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{
    BasePanel, Panel, PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
    register_panel,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyBinding, Render, SharedString, Task, WeakEntity, Window, div, prelude::*,
};
use navidog_app_state::actions::{NewQueryText, TableStructureChanged};
use navidog_db::{DbError, DbResult};
use navidog_model::object::{ObjectRef, Scope};
use navidog_schema::object_design::{self, DefinitionKind};
use navidog_session::sessions::Sessions;
use navidog_sql::definition::DefinitionDraft;
use navidog_sql::dialect::{Dialect, DialectFeatures};
use navidog_ui::assets::lucide;
use navidog_ui::skin::FormRow;
use navidog_ui::{palette, tabs, tokens};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::panel::{self, Save};

const PANEL: &str = "ObjectDesigner";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Definition,
    Options,
    Preview,
}

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    scope: Scope,
    kind: String,
    name: String,
    body: String,
    #[serde(default)]
    previous: Option<String>,
    #[serde(default)]
    raw: bool,
    #[serde(default)]
    loaded: String,
    #[serde(default)]
    server_ddl: String,
    #[serde(default)]
    options: BTreeMap<String, String>,
}

pub struct ObjectDesigner {
    focus: FocusHandle,
    scope: Scope,
    dialect: Arc<dyn Dialect>,
    kind: DefinitionKind,
    name: Entity<InputState>,
    body: Entity<EditorState>,
    sql: Entity<EditorState>,
    options: Vec<(&'static str, Entity<InputState>)>,
    run_args: Entity<InputState>,
    run_raw: bool,
    previous: Option<String>,
    raw: bool,
    loaded: String,
    server_ddl: String,
    page: Page,
    preview: Option<Vec<String>>,
    error: Option<String>,
    busy: bool,
    close_approved: bool,
    task: Option<Task<()>>,
}

impl ObjectDesigner {
    pub fn open(object: ObjectRef, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let kind = DefinitionKind::from_object(object.kind).unwrap_or(DefinitionKind::View);
        Self::create(object.scope, kind, Some(object.name), window, cx)
    }

    pub fn blank(
        scope: Scope,
        kind: &str,
        name: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let kind = DefinitionKind::from_key(kind)?;
        Some(Self::create(scope, kind, name, window, cx))
    }

    fn create(
        scope: Scope,
        kind: DefinitionKind,
        name: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let dialect = panel::connection_dialect(cx, &scope.connection)
            .expect("an object designer opens on a registered connection");
        let saved = Saved {
            scope,
            kind: kind.key().into(),
            name: name.clone().unwrap_or_default(),
            body: String::new(),
            previous: name,
            raw: false,
            loaded: String::new(),
            server_ddl: String::new(),
            options: BTreeMap::new(),
        };
        let entity = Self::restore(saved, dialect, window, cx);
        entity.update(cx, |this, cx| {
            if this.previous.is_some() && this.kind.catalog_kind().is_some() {
                this.load(window, cx);
            }
        });
        entity
    }

    fn restore(
        saved: Saved,
        dialect: Arc<dyn Dialect>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let kind = DefinitionKind::from_key(&saved.kind).unwrap_or(DefinitionKind::View);
        let keys = option_keys(kind, dialect.features());
        cx.new(|cx| {
            let name = cx.new(|cx| InputState::new(window, cx).default_value(saved.name.clone()));
            let body = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("sql")
                    .default_value(saved.body.clone())
            });
            let sql = cx.new(|cx| EditorState::new(window, cx).language("sql"));
            sql.update(cx, |s, cx| s.set_readonly(true, cx));
            let options = keys
                .iter()
                .map(|key| {
                    let value = saved.options.get(*key).cloned().unwrap_or_default();
                    let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
                    (*key, input)
                })
                .collect();
            let run_args = cx.new(|cx| InputState::new(window, cx).default_value(""));
            Self {
                focus: cx.focus_handle().tab_stop(true),
                scope: saved.scope,
                dialect,
                kind,
                name,
                body,
                sql,
                options,
                run_args,
                run_raw: false,
                previous: saved.previous,
                raw: saved.raw,
                loaded: saved.loaded,
                server_ddl: saved.server_ddl,
                page: Page::Definition,
                preview: None,
                error: None,
                busy: false,
                close_approved: false,
                task: None,
            }
        })
    }

    fn draft(&self, cx: &App) -> DefinitionDraft {
        let mut draft = DefinitionDraft::new(self.kind);
        draft.name = self.name.read(cx).value().trim().to_string();
        draft.body = self.body.read(cx).value().to_string();
        draft.previous = self.previous.clone();
        draft.raw = self.raw;
        draft.loaded = self.loaded.clone();
        draft.server_ddl = self.server_ddl.clone();
        if !self.raw {
            for (key, input) in &self.options {
                let value = input.read(cx).value().trim().to_string();
                if !value.is_empty() {
                    draft.options.insert((*key).to_string(), value);
                }
            }
        }
        draft
    }

    fn dirty(&self, cx: &App) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        let body = self.body.read(cx).value();
        // An existing object keeps its loaded name. It is dirty only after the name or the
        // definition changes, including the moment before the server DDL arrives.
        if self.raw || self.previous.is_some() {
            return body.trim() != self.loaded.trim()
                || name != self.previous.clone().unwrap_or_default();
        }
        !name.is_empty()
            || !body.trim().is_empty()
            || self
                .options
                .iter()
                .any(|(_, input)| !input.read(cx).value().trim().is_empty())
    }

    fn show_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match object_design::plan(self.dialect.as_ref(), &self.scope, &self.draft(cx)) {
            Ok(sql) => {
                let text = if sql.is_empty() {
                    format!("-- {}", t!("designer.no_changes"))
                } else {
                    sql.join(";\n")
                };
                self.sql.update(cx, |s, cx| s.set_value(text, window, cx));
                self.preview = Some(sql);
                self.page = Page::Preview;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn execute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.preview.as_ref().is_none_or(Vec::is_empty) {
            return;
        }
        // An empty baseline means the post-save read (or the initial load) did not
        // succeed. Saving now would skip the external-change check.
        if self.previous.is_some()
            && self.kind.catalog_kind().is_some()
            && self.server_ddl.is_empty()
        {
            self.error = Some(t!("designer.readback_failed").to_string());
            cx.notify();
            return;
        }
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .new_session(&self.scope.connection, Some(&self.scope.database))
        else {
            self.error = Some(t!("designer.disconnected").to_string());
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        let scope = self.scope.clone();
        let draft = self.draft(cx);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result: DbResult<()> = async {
                let mut session = job.await.map_err(|e| DbError::msg(e.to_string()))??;
                let job = navidog_db::runtime::spawn(async move {
                    let result = async {
                        let r = session.relational().ok_or_else(|| {
                            DbError::msg("This connection is not a relational database.")
                        })?;
                        object_design::apply(r, &scope, &draft).await
                    }
                    .await;
                    let _ = session.close().await;
                    result
                });
                job.await.map_err(|e| DbError::msg(e.to_string()))?
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.previous = Some(this.name.read(cx).value().trim().to_string());
                        // Dirty compares the editor body with `loaded`. The guard compares
                        // `server_ddl`, which is filled once the server reads the object back.
                        this.loaded = this.body.read(cx).value().to_string();
                        this.server_ddl.clear();
                        this.preview = None;
                        this.error = None;
                        window.push_notification(t!("designer.saved").to_string(), cx);
                        window.dispatch_action(
                            Box::new(TableStructureChanged {
                                scope: this.scope.clone(),
                            }),
                            cx,
                        );
                        this.fetch_ddl(false, window, cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fetch_ddl(true, window, cx);
    }

    /// `replace_body` loads an existing object into the editor. Otherwise only `server_ddl` is updated.
    fn fetch_ddl(&mut self, replace_body: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.previous.clone() else {
            return;
        };
        let Some(kind) = self.kind.catalog_kind() else {
            return;
        };
        let Some(job) = Sessions::global(cx)
            .read(cx)
            .new_session(&self.scope.connection, Some(&self.scope.database))
        else {
            if !replace_body {
                self.error = Some(t!("designer.readback_failed").to_string());
                cx.notify();
            }
            return;
        };
        self.busy = true;
        let scope = self.scope.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result: DbResult<String> = async {
                let mut session = job.await.map_err(|e| DbError::msg(e.to_string()))??;
                let job = navidog_db::runtime::spawn(async move {
                    let result = async {
                        let r = session.relational().ok_or_else(|| {
                            DbError::msg("This connection is not a relational database.")
                        })?;
                        if let Some(sql) = r.dialect().prepare_database_sql(&scope.database) {
                            r.execute(&sql).await?;
                        }
                        r.object_ddl(&scope, kind, &name).await
                    }
                    .await;
                    let _ = session.close().await;
                    result
                });
                job.await.map_err(|e| DbError::msg(e.to_string()))?
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(ddl) if !ddl.is_empty() => {
                        this.server_ddl = ddl.clone();
                        if replace_body {
                            this.loaded = ddl.clone();
                            this.raw = true;
                            this.body.update(cx, |s, cx| s.set_value(ddl, window, cx));
                        }
                    }
                    _ => this.error = Some(t!("designer.readback_failed").to_string()),
                }
                cx.notify();
            });
        }));
    }

    fn preview_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let sql = format!(
            "SELECT * FROM {}",
            self.dialect.qualified(&self.scope, &name)
        );
        self.dispatch_sql(sql, window, cx);
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.needs_arguments(cx) {
            self.ask_arguments(window, cx);
            return;
        }
        self.run_now(window, cx);
    }

    fn run_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .dialect
            .qualified(&self.scope, &self.name.read(cx).value());
        let args = self.run_args.read(cx).value().trim().to_string();
        let rendered = if self.run_raw || args.is_empty() {
            args
        } else {
            args.split(',')
                .map(|part| self.dialect.string_literal(part.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let procedure = self.kind == DefinitionKind::Procedure
            || self.options.iter().any(|(key, input)| {
                *key == "routine" && input.read(cx).value().eq_ignore_ascii_case("procedure")
            })
            || self
                .body
                .read(cx)
                .value()
                .to_ascii_uppercase()
                .contains("PROCEDURE");
        let sql = if procedure {
            format!("CALL {name}({rendered})")
        } else {
            format!("SELECT {name}({rendered})")
        };
        self.dispatch_sql(sql, window, cx);
    }

    fn needs_arguments(&self, cx: &App) -> bool {
        if let Some((_, input)) = self.options.iter().find(|(key, _)| *key == "parameters") {
            let params = input.read(cx).value().trim().to_string();
            if !params.is_empty() && params != "()" {
                return true;
            }
        }
        let body = self.body.read(cx).value();
        body.find('(')
            .is_some_and(|i| !body[i + 1..].trim_start().starts_with(')'))
    }

    fn ask_arguments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let args = self.run_args.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak_check = weak.clone();
            let weak_ok = weak.clone();
            let args = args.clone();
            dialog
                .title(t!("designer.arguments").to_string())
                .child(Input::new(&args).id("object-run-args"))
                .child(
                    Checkbox::new("object-run-raw")
                        .label(t!("designer.raw_arguments").to_string())
                        .checked(false)
                        .on_click(move |value, _, cx| {
                            let _ = weak_check.update(cx, |this, cx| {
                                this.run_raw = *value;
                                cx.notify();
                            });
                        }),
                )
                .footer(
                    h_flex().justify_end().gap_2().child(
                        Button::new("object-run-ok")
                            .label(t!("designer.run").to_string())
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = weak_ok.update(cx, |this, cx| this.run_now(window, cx));
                            }),
                    ),
                )
        });
    }

    fn dispatch_sql(&self, sql: String, window: &mut Window, cx: &mut App) {
        window.dispatch_action(
            Box::new(NewQueryText {
                connection: self.scope.connection.clone().into(),
                database: self.scope.database.clone().into(),
                schema: self.scope.schema.clone().map(Into::into),
                sql: sql.into(),
            }),
            cx,
        );
    }

    fn ask_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.dirty(cx) {
            return;
        }
        let weak = cx.entity().downgrade();
        panel::confirm_discard(
            PanelId::from(cx.entity_id()),
            t!("designer.object_close_title").to_string(),
            t!("designer.object_close_body").to_string(),
            window,
            cx,
            move |window, cx| {
                if let Ok(id) = weak.update(cx, |this, cx| {
                    this.close_approved = true;
                    PanelId::from(cx.entity_id())
                }) {
                    window.dispatch_action(
                        Box::new(tabs::CloseTabs {
                            panel: Some(id),
                            which: tabs::Close::This,
                        }),
                        cx,
                    );
                }
            },
        );
    }
}

impl EventEmitter<PanelEvent> for ObjectDesigner {}
impl Focusable for ObjectDesigner {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BasePanel for ObjectDesigner {
    fn panel_name(&self) -> &'static str {
        PANEL
    }
    fn closable(&self, cx: &App) -> bool {
        !self.busy && (self.close_approved || !self.dirty(cx))
    }
    fn dump(&self, cx: &App) -> PanelState {
        let saved = Saved {
            scope: self.scope.clone(),
            kind: self.kind.key().into(),
            name: self.name.read(cx).value().to_string(),
            body: self.body.read(cx).value().to_string(),
            previous: self.previous.clone(),
            raw: self.raw,
            loaded: self.loaded.clone(),
            server_ddl: self.server_ddl.clone(),
            options: self
                .options
                .iter()
                .map(|(key, input)| ((*key).to_string(), input.read(cx).value().to_string()))
                .collect(),
        };
        PanelState {
            panel_name: PANEL.into(),
            children: Vec::new(),
            info: PanelInfo::panel(serde_json::to_value(saved).unwrap_or_default()),
        }
    }
    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = PanelId::from(cx.entity_id());
        tabs::added(id, group, cx);
        let weak = cx.entity().downgrade();
        let home = window.window_handle();
        tabs::set_close_guard(
            id,
            Rc::new(move |cx| {
                let weak = weak.clone();
                let _ = home.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.ask_close(window, cx));
                });
            }),
            cx,
        );
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        tabs::set_active(PanelId::from(cx.entity_id()), active, cx);
    }
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if tabs::removed(PanelId::from(cx.entity_id()), cx) {
            return;
        }
        self.task.take();
    }
}

impl Panel for ObjectDesigner {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.name.read(cx).value().trim().to_string();
        let name = if name.is_empty() {
            t!("designer.untitled_object").to_string()
        } else {
            name
        };
        tabs::title(
            PanelId::from(cx.entity_id()),
            Some(lucide("pencil")),
            format!(
                "{name} — {} @{}",
                t!(format!("designer.kind_{}", self.kind.key())),
                self.scope.database
            ),
            None,
            self.dirty(cx),
        )
    }
}

impl Render for ObjectDesigner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy;
        let p = palette(cx);
        let preview = self.page == Page::Preview;
        let show_data = matches!(
            self.kind,
            DefinitionKind::View | DefinitionKind::MaterializedView
        );
        let show_run = matches!(
            self.kind,
            DefinitionKind::Function | DefinitionKind::Procedure
        );
        let pages: Vec<(Page, &'static str)> = [
            (Page::Definition, "definition"),
            (Page::Options, "options"),
            (Page::Preview, "preview"),
        ]
        .into_iter()
        .filter(|(page, _)| *page != Page::Options || !self.options.is_empty() && !self.raw)
        .collect();
        if !pages.iter().any(|(page, _)| *page == self.page) {
            self.page = Page::Definition;
        }
        let selected = pages
            .iter()
            .position(|(page, _)| *page == self.page)
            .unwrap_or(0);
        let pages_for_bar = pages.clone();
        v_flex()
            .id("object-designer")
            .key_context(PANEL)
            .track_focus(&self.focus)
            .size_full()
            .bg(p.window_bg)
            .on_action(cx.listener(|this, _: &Save, window, cx| this.show_preview(window, cx)))
            .child(
                h_flex()
                    .min_h(tokens::object_toolbar::HEIGHT.rems())
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.separator)
                    .when(preview, |bar| {
                        bar.child(
                            Button::new("object-execute")
                                .small()
                                .icon(lucide("play"))
                                .label(t!("designer.execute").to_string())
                                .disabled(
                                    disabled || self.preview.as_ref().is_none_or(Vec::is_empty),
                                )
                                .on_click(cx.listener(|this, _, w, cx| this.execute(w, cx))),
                        )
                    })
                    .when(!preview, |bar| {
                        bar.child(
                            Button::new("object-save")
                                .ghost()
                                .small()
                                .icon(lucide("save"))
                                .label(t!("designer.save").to_string())
                                .disabled(disabled)
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.show_preview(window, cx)
                                    }),
                                ),
                        )
                    })
                    .when(show_data, |bar| {
                        bar.child(
                            Button::new("object-preview-data")
                                .ghost()
                                .small()
                                .label(t!("designer.preview_data").to_string())
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, w, cx| this.preview_data(w, cx))),
                        )
                    })
                    .when(show_run, |bar| {
                        bar.child(
                            Button::new("object-run")
                                .ghost()
                                .small()
                                .icon(lucide("play"))
                                .label(t!("designer.run").to_string())
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, w, cx| this.run(w, cx))),
                        )
                    }),
            )
            .child(
                TabBar::new("object-pages")
                    .segmented()
                    .small()
                    .w_full()
                    .selected_index(selected)
                    .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                        if let Some((page, _)) = pages_for_bar.get(*ix) {
                            this.page = *page;
                            cx.notify();
                        }
                    }))
                    .children(pages.into_iter().map(|(page, key)| {
                        let label = t!(format!("designer.{key}")).to_string();
                        Tab::new().child(
                            div()
                                .id(SharedString::from(format!("object-page-{key}")))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.page = page;
                                    cx.notify();
                                }))
                                .child(label)
                                .test_support(),
                        )
                    })),
            )
            .when_some(self.error.clone(), |v, error| {
                v.child(gpui_kit::div().px_2().text_color(p.danger).child(error))
            })
            .child(div().flex_1().min_h_0().w_full().child(match self.page {
                Page::Preview => Editor::new(&self.sql).size_full().into_any_element(),
                Page::Options => self.render_options(disabled),
                Page::Definition => self.render_definition(disabled),
            }))
    }
}

impl ObjectDesigner {
    fn render_definition(&self, disabled: bool) -> gpui_kit::AnyElement {
        v_flex()
            .id("object-definition")
            .size_full()
            .p_3()
            .gap_2()
            .child(
                FormRow::new(
                    t!("designer.name").to_string(),
                    Input::new(&self.name)
                        .id("object-name")
                        .small()
                        .disabled(disabled),
                )
                .hug_label(),
            )
            .when(self.kind_has_body(), |col| {
                col.child(
                    Editor::new(&self.body)
                        .disabled(disabled)
                        .flex_1()
                        .min_h_0(),
                )
            })
            .into_any_element()
    }

    fn render_options(&self, disabled: bool) -> gpui_kit::AnyElement {
        v_flex()
            .id("object-options")
            .size_full()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .children(self.options.iter().map(|(key, input)| {
                FormRow::new(
                    t!(format!("designer.opt_{key}")).to_string(),
                    Input::new(input)
                        .id(SharedString::from(format!("object-opt-{key}")))
                        .small()
                        .disabled(disabled),
                )
                .hug_label()
            }))
            .into_any_element()
    }

    fn kind_has_body(&self) -> bool {
        !matches!(
            self.kind,
            DefinitionKind::Database
                | DefinitionKind::Schema
                | DefinitionKind::Sequence
                | DefinitionKind::Extension
                | DefinitionKind::Domain
                | DefinitionKind::Collation
                | DefinitionKind::Attach
                | DefinitionKind::Detach
                | DefinitionKind::Tablespace
                | DefinitionKind::Aggregate
                | DefinitionKind::Cast
                | DefinitionKind::Operator
        )
    }
}

fn option_keys(kind: DefinitionKind, features: DialectFeatures) -> &'static [&'static str] {
    match kind {
        DefinitionKind::View if features.inline_table_options => {
            &["algorithm", "security", "check_option"]
        }
        DefinitionKind::MaterializedView => &["with_data"],
        DefinitionKind::Function | DefinitionKind::Procedure if features.inline_table_options => {
            &["routine", "parameters", "returns"]
        }
        DefinitionKind::Function | DefinitionKind::Procedure => {
            &["routine", "parameters", "returns", "language"]
        }
        DefinitionKind::Event => &["status", "schedule"],
        DefinitionKind::Database if features.inline_table_options => &["charset", "collation"],
        DefinitionKind::Database => &["owner", "encoding", "template"],
        DefinitionKind::Schema => &["owner"],
        DefinitionKind::Tablespace if features.inline_table_options => &["engine", "datafile"],
        DefinitionKind::Tablespace => &["datafile"],
        DefinitionKind::Sequence => &["start", "increment"],
        DefinitionKind::Type => &["type_kind"],
        DefinitionKind::Domain => &["base_type", "not_null", "default"],
        DefinitionKind::Extension => &["action"],
        DefinitionKind::ForeignServer => &["wrapper"],
        DefinitionKind::Aggregate => &["input_type", "state_func", "state_type"],
        DefinitionKind::Cast => &["source", "target", "function"],
        DefinitionKind::Operator => &["left", "right", "function"],
        DefinitionKind::Collation => &["provider", "locale"],
        DefinitionKind::Index => &["table"],
        DefinitionKind::Trigger => &["table", "timing", "events"],
        DefinitionKind::Attach => &["path"],
        _ => &[],
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, Some(PANEL)),
        KeyBinding::new("cmd-s", Save, Some("ObjectDesigner > Input")),
    ]);
    register_panel(cx, PANEL, |ctx, window, cx| {
        let saved = match ctx.info() {
            PanelInfo::Panel(v) => serde_json::from_value::<Saved>(v.clone()).ok(),
            _ => None,
        };
        let Some(saved) = saved else {
            return panel_handle(panel::MissingConnection::open(ctx.state().clone(), cx));
        };
        let Some(dialect) = panel::connection_dialect(cx, &saved.scope.connection) else {
            return panel_handle(panel::MissingConnection::open(ctx.state().clone(), cx));
        };
        panel_handle(ObjectDesigner::restore(saved, dialect, window, cx))
    });
}
