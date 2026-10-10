//! Auto recovery (30-query-editor.md §8): unsaved query editors are written to
//! `autosave/<editor id>.json` on an interval. Files left behind by a crash are offered back in
//! the "Recover" dialog at startup. On a normal quit the unsaved editors are written once more
//! and marked, so their tabs come back with the layout without asking.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use navidog_core::settings::Settings;
use navidog_ui::skin::{heading, truncated};
use navidog_ui::{palette, tokens};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::editor::{QueryEditor, QuerySource, Target};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Autosave {
    pub id: String,
    pub title: String,
    pub source: QuerySource,
    pub target: Target,
    pub text: String,
    pub saved_at: String,
    /// Written on a normal quit; the tab returns with the layout instead of the dialog
    #[serde(default)]
    pub on_quit: bool,
}

fn dir() -> PathBuf {
    navidog_core::paths::config_dir().join("autosave")
}

fn path(id: &str) -> PathBuf {
    dir().join(format!("{id}.json"))
}

pub(crate) fn write(entry: &Autosave) {
    let Ok(bytes) = serde_json::to_vec_pretty(entry) else {
        return;
    };
    if let Err(e) = navidog_core::paths::write_atomic(&path(&entry.id), &bytes) {
        tracing::warn!("autosave failed: {e}");
    }
}

#[cfg(test)]
pub(crate) fn read(id: &str) -> Option<Autosave> {
    serde_json::from_slice(&std::fs::read(path(id)).ok()?).ok()
}

/// Saved, closed without saving, or quit cleanly with no changes: the file goes.
pub(crate) fn forget(id: &str) {
    let _ = std::fs::remove_file(path(id));
}

pub(crate) fn all() -> Vec<Autosave> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut out: Vec<Autosave> = entries
        .flatten()
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a, b| a.saved_at.cmp(&b.saved_at));
    out
}

impl QueryEditor {
    pub(crate) fn autosave_entry(&self, on_quit: bool, cx: &App) -> Autosave {
        Autosave {
            id: self.id.clone(),
            title: self.display_name(),
            source: self.source.clone(),
            target: self.target.clone(),
            text: self.text(cx),
            saved_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            on_quit,
        }
    }

    /// Restores text from an autosave (dock restore after a quit, or the "Recover" dialog).
    pub(crate) fn take_autosave(
        &mut self,
        entry: &Autosave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = entry.text.clone();
        self.editor
            .update(cx, |e, cx| e.set_value(text, window, cx));
        self.dirty = true;
        cx.notify();
    }
}

/// The autosave loop; reads the interval each round, so preference changes apply.
pub(crate) fn init(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let secs = cx.update(|cx| Settings::global(cx).autosave_secs);
            cx.background_executor()
                .timer(Duration::from_secs(secs.unwrap_or(15).max(5) as u64))
                .await;
            if secs.is_none() {
                continue;
            }
            cx.update(|cx| {
                for editor in crate::save::open_editors(cx) {
                    let e = editor.read(cx);
                    if e.dirty {
                        write(&e.autosave_entry(false, cx));
                    }
                }
            });
        }
    })
    .detach();
}

/// Normal quit: unsaved editors are written and marked; the others leave nothing behind.
pub fn flush_on_quit(cx: &mut App) {
    for editor in crate::save::open_editors(cx) {
        let e = editor.read(cx);
        if e.dirty {
            write(&e.autosave_entry(true, cx));
        } else {
            forget(&e.id);
        }
    }
}

type Open = std::rc::Rc<dyn Fn(Entity<QueryEditor>, &mut Window, &mut App)>;

/// At startup, after the layout is restored: editors back from a normal quit take their text
/// silently; anything else left in `autosave/` is offered in the "Recover" dialog. `open` adds
/// a recovered editor to the window.
pub fn check(
    window: &mut Window,
    cx: &mut App,
    open: impl Fn(Entity<QueryEditor>, &mut Window, &mut App) + 'static,
) {
    let editors = crate::save::open_editors(cx);
    let mut offered = Vec::new();
    for entry in all() {
        let restored = editors
            .iter()
            .find(|e| matches_entry(e.read(cx), &entry))
            .cloned();
        match restored {
            Some(editor) if entry.on_quit => {
                editor.update(cx, |e, cx| e.take_autosave(&entry, window, cx));
            }
            _ => offered.push(entry),
        }
    }
    if offered.is_empty() {
        return;
    }
    let checked = vec![true; offered.len()];
    let view = cx.new(|_| Recover {
        entries: offered,
        checked,
        open: std::rc::Rc::new(open),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(tokens::query::RECOVERY_WIDTH.to_pixels(window.rem_size()))
            .close_button(false)
            .keyboard(false)
            .child(view.clone())
    });
}

/// The same editor: its id, or the same saved query when it was restored under a new id.
fn matches_entry(e: &QueryEditor, entry: &Autosave) -> bool {
    e.id == entry.id || (matches!(entry.source, QuerySource::Saved(_)) && e.source == entry.source)
}

struct Recover {
    entries: Vec<Autosave>,
    checked: Vec<bool>,
    open: Open,
}

impl Recover {
    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let editors = crate::save::open_editors(cx);
        for (entry, on) in self.entries.iter().zip(&self.checked) {
            if !on {
                forget(&entry.id);
                continue;
            }
            match editors.iter().find(|e| matches_entry(e.read(cx), entry)) {
                Some(editor) => editor.update(cx, |e, cx| e.take_autosave(entry, window, cx)),
                None => {
                    let source = match &entry.source {
                        // The saved query may be gone; then the text comes back untitled
                        QuerySource::Saved(o)
                            if !navidog_core::queries::path(&o.scope, &o.name).exists() =>
                        {
                            QuerySource::Untitled(crate::editor::next_untitled())
                        }
                        other => other.clone(),
                    };
                    let editor = QueryEditor::restored(
                        entry.id.clone(),
                        source,
                        entry.target.clone(),
                        entry.text.clone(),
                        window,
                        cx,
                    );
                    (self.open)(editor, window, cx);
                }
            }
        }
    }

    fn location(entry: &Autosave, cx: &App) -> String {
        let conn = entry
            .target
            .connection
            .as_ref()
            .and_then(|id| {
                navidog_core::store::ConnectionStore::global(cx)
                    .read(cx)
                    .get(id)
            })
            .map(|c| c.name.clone());
        let parts: Vec<String> = [
            conn,
            entry.target.database.clone(),
            entry.target.schema.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if parts.is_empty() {
            entry.title.clone()
        } else {
            format!("{} ({})", entry.title, parts.join(" / "))
        }
    }
}

impl Render for Recover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let height = tokens::query::RECOVERY_HEIGHT.to_pixels(window.rem_size());
        v_flex()
            .h(height * 0.8)
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("query.recover_title").to_string()))
            .child(
                h_flex()
                    .h(tokens::list::HEADER_HEIGHT.rems())
                    .border_b_1()
                    .border_color(p.separator)
                    .child(div().w(tokens::form::LIST_BUTTON_WIDTH.rems()))
                    .child(
                        div()
                            .w(tokens::list::NAME_COLUMN_WIDTH.rems())
                            .px_2()
                            .child(t!("query.recover_time").to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .child(t!("query.recover_name").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .id("recover-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(self.entries.iter().enumerate().map(|(ix, entry)| {
                        h_flex()
                            .h(tokens::list::ROW_HEIGHT.rems())
                            .when(ix % 2 == 1, |d| d.bg(p.row_alt))
                            .child(
                                div().w(tokens::form::LIST_BUTTON_WIDTH.rems()).child(
                                    Checkbox::new(("recover-check", ix))
                                        .checked(self.checked[ix])
                                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                                            this.checked[ix] = *on;
                                            cx.notify();
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .w(tokens::list::NAME_COLUMN_WIDTH.rems())
                                    .px_2()
                                    .child(entry.saved_at.clone()),
                            )
                            .child(
                                truncated(("recover-name", ix), Self::location(entry, cx))
                                    .flex_1()
                                    .px_2(),
                            )
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("recover-all")
                            .label(t!("query.select_all").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.checked.iter_mut().for_each(|c| *c = true);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("recover-none")
                            .label(t!("query.deselect_all").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.checked.iter_mut().for_each(|c| *c = false);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("recover-cancel")
                            .label(t!("query.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("recover-ok")
                            .primary()
                            .label(t!("query.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.ok(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_through_their_files() {
        // The same folder the editor tests use: they all run in one process
        let home = std::env::temp_dir().join(format!("navidog-query-test-{}", std::process::id()));
        // SAFETY: every test of this crate sets the same value
        unsafe { std::env::set_var("NAVIDOG_HOME", &home) };
        let entry = Autosave {
            id: "e1".into(),
            title: "Untitled".into(),
            source: QuerySource::Untitled(1),
            target: Target::default(),
            text: "SELECT 1".into(),
            saved_at: "2026-10-11 00:00:00".into(),
            on_quit: false,
        };
        write(&entry);
        assert_eq!(read("e1"), Some(entry.clone()));
        assert!(all().contains(&entry));
        forget("e1");
        assert_eq!(read("e1"), None);
    }
}
