//! Saving queries (30-query-editor.md §7): saved queries in the profile folder, external `.sql`
//! files, "Save As", and noticing edits made outside the app.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window, actions, div,
};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::queries;
use navidog_core::workspace::Workspace;
use navidog_ui::assets::lucide;
use navidog_ui::skin::{confirm_with, heading};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use crate::editor::{QueryEditor, QuerySource};

actions!(
    query,
    [SaveQuery, SaveQueryAs, SaveAsExternal, SaveAsNavicat]
);

/// Every open query editor, for the autosave timer and for following renamed files.
#[derive(Default)]
pub(crate) struct OpenEditors(pub Vec<WeakEntity<QueryEditor>>);
impl Global for OpenEditors {}

pub(crate) fn register(editor: WeakEntity<QueryEditor>, cx: &mut App) {
    let list = &mut cx.default_global::<OpenEditors>().0;
    list.retain(|e| e.upgrade().is_some());
    list.push(editor);
}

pub(crate) fn open_editors(cx: &mut App) -> Vec<Entity<QueryEditor>> {
    let list = &mut cx.default_global::<OpenEditors>().0;
    list.retain(|e| e.upgrade().is_some());
    list.iter().filter_map(|e| e.upgrade()).collect()
}

/// The open editor of a saved query.
pub fn editor_for(object: &ObjectRef, cx: &mut App) -> Option<Entity<QueryEditor>> {
    open_editors(cx)
        .into_iter()
        .find(|e| e.read(cx).source == QuerySource::Saved(object.clone()))
}

pub(crate) fn init(cx: &mut App) {
    cx.on_action(|a: &navidog_core::actions::QueryRenamed, cx| {
        for editor in open_editors(cx) {
            editor.update(cx, |e, cx| {
                if e.source == QuerySource::Saved(a.from.clone()) {
                    e.source = QuerySource::Saved(a.to.clone());
                    e.file_mtime = queries::modified(&queries::path(&a.to.scope, &a.to.name));
                    cx.notify();
                }
            });
        }
    });
}

type Named = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// "Query Name" prompt; the name must be usable in `scope` (30 §7).
struct NamePrompt {
    input: Entity<InputState>,
    scope: Scope,
    current: Option<String>,
    invalid: bool,
    done: Named,
}

fn ask_name(
    scope: Scope,
    initial: String,
    current: Option<String>,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    input.update(cx, |s, cx| {
        s.focus(window, cx);
        s.select_all(window, cx);
    });
    let view = cx.new(|_| NamePrompt {
        input,
        scope,
        current,
        invalid: false,
        done: Rc::new(done),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(
            dialog
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false),
            &view,
            NamePrompt::ok,
        )
        .child(view.clone())
    });
}

impl NamePrompt {
    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).value().trim().to_string();
        if !queries::name_ok(&self.scope, &name, self.current.as_deref()) {
            self.invalid = true;
            cx.notify();
            return;
        }
        window.close_dialog(cx);
        (self.done)(name, window, cx);
    }
}

impl Render for NamePrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("query.query_name").to_string()))
            .child(Input::new(&self.input).id("query-name-input"))
            .when(self.invalid, |v| {
                v.child(
                    div()
                        .text_size(tokens::font::SIZE_SMALL.rems())
                        .text_color(p.danger)
                        .child(t!("query.name_invalid").to_string()),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("query-name-cancel")
                            .label(t!("query.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("query-name-ok")
                            .primary()
                            .label(t!("query.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.ok(window, cx))),
                    ),
            )
    }
}

impl QueryEditor {
    /// ⌘S. An untitled query asks for a name and saves where the toolbar points; without a
    /// connection it becomes an external file instead. `close` closes the tab once it is saved.
    pub(crate) fn save_query(&mut self, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.source.clone() {
            QuerySource::Saved(o) => {
                if self.write(&queries::path(&o.scope, &o.name), window, cx) {
                    Workspace::global(cx).update(cx, |w, cx| w.queries_changed(cx));
                    self.after_save(close, window, cx);
                }
            }
            QuerySource::External(path) => {
                if self.write(&path, window, cx) {
                    self.after_save(close, window, cx);
                }
            }
            QuerySource::Untitled(_) => match self.target.scope() {
                Some(scope) => self.save_as_navicat(scope, close, window, cx),
                None => self.save_as_external(true, close, window, cx),
            },
        }
    }

    /// ⇧⌘S: a new name in the current location; the tab moves to the new query.
    pub(crate) fn save_query_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope = match &self.source {
            QuerySource::Saved(o) => Some(o.scope.clone()),
            _ => self.target.scope(),
        };
        match scope {
            Some(scope) => self.save_as_navicat(scope, false, window, cx),
            None => self.save_as_external(true, false, window, cx),
        }
    }

    /// Asks for a name and saves into `scope` as a saved query.
    pub(crate) fn save_as_navicat(
        &mut self,
        scope: Scope,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        let initial = match &self.source {
            QuerySource::Untitled(_) => String::new(),
            _ => self.display_name().trim_end_matches(".sql").to_string(),
        };
        let target = scope.clone();
        ask_name(scope, initial, None, window, cx, move |name, window, cx| {
            let object = ObjectRef {
                scope: target.clone(),
                kind: ObjectKind::Query,
                name,
            };
            let _ = this.update(cx, |e, cx| {
                if e.write(&queries::path(&object.scope, &object.name), window, cx) {
                    e.source = QuerySource::Saved(object.clone());
                    e.target.connection = Some(object.scope.connection.clone());
                    e.target.database = Some(object.scope.database.clone());
                    e.target.schema = object.scope.schema.clone();
                    Workspace::global(cx).update(cx, |w, cx| w.queries_changed(cx));
                    e.after_save(close, window, cx);
                }
            });
        });
    }

    /// The system save panel. `rebind` makes the tab edit the new file afterwards.
    pub(crate) fn save_as_external(
        &mut self,
        rebind: bool,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let suggested = format!("{}.sql", self.display_name().trim_end_matches(".sql"));
        let dir = match &self.source {
            QuerySource::External(p) => p.parent().map(PathBuf::from),
            _ => None,
        }
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_default();
        let picked = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let _ = this.update_in(cx, |e, window, cx| {
                if e.write(&path, window, cx) && rebind {
                    e.source = QuerySource::External(path);
                    e.after_save(close, window, cx);
                }
            });
        })
        .detach();
    }

    /// Writes the text; a failure is shown and leaves the editor as it was.
    fn write(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self.text(cx);
        match navidog_core::paths::write_atomic(path, text.as_bytes()) {
            Ok(()) => {
                self.file_mtime = queries::modified(path);
                true
            }
            Err(e) => {
                let message = e.to_string();
                window.open_alert_dialog(cx, move |alert, _, _| {
                    alert
                        .icon(lucide("circle-x"))
                        .title(t!("query.save_failed").to_string())
                        .description(message.clone())
                });
                false
            }
        }
    }

    fn after_save(&mut self, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.dirty = false;
        crate::recovery::forget(&self.id);
        cx.notify();
        if close {
            self.close_now(window, cx);
        }
    }

    /// The file behind the tab, if any.
    pub(crate) fn file_path(&self) -> Option<PathBuf> {
        match &self.source {
            QuerySource::Saved(o) => Some(queries::path(&o.scope, &o.name)),
            QuerySource::External(p) => Some(p.clone()),
            QuerySource::Untitled(_) => None,
        }
    }

    /// On activation: the file changed on disk since it was loaded or saved (30 §7).
    pub(crate) fn check_external_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.file_path() else { return };
        let now = queries::modified(&path);
        if now.is_none() || now == self.file_mtime || self.file_mtime.is_none() {
            return;
        }
        self.file_mtime = now;
        let this = cx.entity().downgrade();
        let name = self.display_name();
        let dirty = self.dirty;
        window.open_alert_dialog(cx, move |alert, _, _| {
            let reload = this.clone();
            alert
                .icon(lucide("info"))
                .title(t!("query.changed_outside", name = name).to_string())
                .show_cancel(true)
                .ok_text(t!("query.reload").to_string())
                .cancel_text(t!("query.cancel").to_string())
                // Reloading drops unsaved edits, so then Enter cancels
                .keyboard(!dirty)
                .on_ok(move |_, window, cx| {
                    let _ = reload.update(cx, |e, cx| e.reload_file(window, cx));
                    true
                })
        });
    }

    fn reload_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.file_path() else { return };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        self.editor
            .update(cx, |e, cx| e.set_value(text, window, cx));
        self.file_mtime = queries::modified(&path);
        self.dirty = false;
        cx.notify();
    }
}
