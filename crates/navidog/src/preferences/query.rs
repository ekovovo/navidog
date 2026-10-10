//! Preferences pages for the query editor (30-query-editor.md §9, 31-query-results.md §6):
//! Editor, Code Completion, Auto Recovery, and the log options of the Advanced page.

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Styled as _, Window,
    div,
};
use navidog_core::settings::Settings;
use navidog_ui::skin::{form_note, truncated};
use navidog_ui::theme::{SYNTAX_KEYS, syntax_color, to_hex};
use navidog_ui::{palette, tokens};
use rust_i18n::t;

use super::Preferences;

/// Autosave intervals offered (F-PREF-008); `None` is "Never".
pub(super) const AUTOSAVE: [Option<u32>; 6] =
    [Some(15), Some(30), Some(60), Some(180), Some(300), None];

const PREVIEW_SQL: &str = "-- Preview\nSELECT a.actor_id, 'name' AS label, 42 + 1.5\nFROM actor a\nWHERE a.first_name LIKE 'P%';";

pub(super) struct QueryPrefs {
    highlight_limit: Entity<InputState>,
    tab_width: Entity<InputState>,
    editor_font: Entity<InputState>,
    colors: Vec<(&'static str, Entity<ColorPickerState>)>,
    preview: Entity<EditorState>,
}

/// A number field applied on Return or blur; an invalid value goes back to the saved one.
fn number_input(
    value: String,
    apply: fn(&mut Settings, f32) -> bool,
    saved: fn(&Settings) -> String,
    window: &mut Window,
    cx: &mut Context<Preferences>,
) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
    cx.subscribe_in(
        &input,
        window,
        move |_, input, e: &InputEvent, window, cx| {
            if !matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                return;
            }
            let parsed = input.read(cx).value().trim().parse::<f32>().ok();
            let mut ok = false;
            if let Some(v) = parsed {
                let mut s = Settings::global(cx).clone();
                if apply(&mut s, v) {
                    Settings::update(cx, |g| *g = s);
                    ok = true;
                    cx.refresh_windows();
                }
            }
            if !ok {
                let text = saved(Settings::global(cx));
                input.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        },
    )
    .detach();
    input
}

impl QueryPrefs {
    pub fn new(window: &mut Window, cx: &mut Context<Preferences>) -> Self {
        let s = Settings::global(cx).clone();
        let highlight_limit = number_input(
            s.highlight_limit_mb.to_string(),
            |s, v| {
                let ok = (1.0..=1024.0).contains(&v);
                if ok {
                    s.highlight_limit_mb = v as u32;
                }
                ok
            },
            |s| s.highlight_limit_mb.to_string(),
            window,
            cx,
        );
        let tab_width = number_input(
            s.tab_width.to_string(),
            |s, v| {
                let ok = (1.0..=16.0).contains(&v);
                if ok {
                    s.tab_width = v as u8;
                }
                ok
            },
            |s| s.tab_width.to_string(),
            window,
            cx,
        );
        let editor_font = number_input(
            s.editor_font_pt.to_string(),
            |s, v| {
                let ok =
                    (tokens::query::EDITOR_FONT_MIN..=tokens::query::EDITOR_FONT_MAX).contains(&v);
                if ok {
                    s.editor_font_pt = v;
                }
                ok
            },
            |s| s.editor_font_pt.to_string(),
            window,
            cx,
        );
        let colors = SYNTAX_KEYS
            .iter()
            .map(|key| {
                let color = syntax_color(key, cx).unwrap_or(palette(cx).label);
                let state = cx.new(|cx| ColorPickerState::new(window, cx).default_value(color));
                let key: &'static str = key;
                cx.subscribe_in(
                    &state,
                    window,
                    move |_, _, e: &ColorPickerEvent, window, cx| {
                        let ColorPickerEvent::Change(Some(c)) = e else {
                            return;
                        };
                        let hex = to_hex(*c);
                        let dark = cx.theme().is_dark();
                        Settings::update(cx, |s| {
                            let map = if dark {
                                &mut s.syntax_colors.dark
                            } else {
                                &mut s.syntax_colors.light
                            };
                            map.insert(key.to_string(), hex);
                        });
                        crate::apply_theme(Some(window), cx);
                    },
                )
                .detach();
                (key, state)
            })
            .collect();
        let preview = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
        });
        preview.update(cx, |e, cx| {
            e.set_value(PREVIEW_SQL, window, cx);
            e.set_readonly(true, cx);
        });
        Self {
            highlight_limit,
            tab_width,
            editor_font,
            colors,
            preview,
        }
    }

    /// The pickers show the current appearance's colors, after a reset or an appearance change.
    fn sync_colors(&self, window: &mut Window, cx: &mut App) {
        for (key, state) in &self.colors {
            if let Some(c) = syntax_color(key, cx)
                && state.read(cx).value() != Some(c)
            {
                state.update(cx, |s, cx| s.set_value(c, window, cx));
            }
        }
    }
}

fn check(id: &'static str, label: String, on: bool, set: fn(&mut Settings, bool)) -> Checkbox {
    Checkbox::new(id)
        .label(label)
        .checked(on)
        .on_click(move |v: &bool, _, cx| {
            let v = *v;
            Settings::update(cx, |s| set(s, v));
            cx.refresh_windows();
        })
}

impl Preferences {
    /// Editor (F-PREF-004).
    pub(super) fn render_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let q = &self.query;
        q.sync_colors(window, cx);
        let label = |key: &str| {
            match key {
                "text" => t!("prefs.color_text"),
                "keyword" => t!("prefs.color_keyword"),
                "comment" => t!("prefs.color_comment"),
                "string" => t!("prefs.color_string"),
                "number" => t!("prefs.color_number"),
                _ => t!("prefs.color_background"),
            }
            .to_string()
        };
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::setting_row(
                t!("prefs.highlight_limit").to_string(),
                h_flex()
                    .gap_2()
                    .child(Input::new(&q.highlight_limit).w(tokens::form::FIELD_WIDTH_SHORT.rems()))
                    .child("MB"),
            ))
            .child(Self::section(t!("prefs.display").to_string()))
            .child(check(
                "show-line-numbers",
                t!("prefs.show_line_numbers").to_string(),
                s.show_line_numbers,
                |s, v| s.show_line_numbers = v,
            ))
            .child(check(
                "code-folding",
                t!("prefs.code_folding").to_string(),
                s.code_folding,
                |s, v| s.code_folding = v,
            ))
            .child(check(
                "bracket-highlight",
                t!("prefs.bracket_highlight").to_string(),
                s.bracket_highlight,
                |s, v| s.bracket_highlight = v,
            ))
            .child(check(
                "syntax-highlight",
                t!("prefs.syntax_highlight").to_string(),
                s.syntax_highlight,
                |s, v| s.syntax_highlight = v,
            ))
            .child(Self::section(t!("prefs.text_layout").to_string()))
            .child(check(
                "insert-spaces",
                t!("prefs.insert_spaces").to_string(),
                s.insert_spaces,
                |s, v| s.insert_spaces = v,
            ))
            .child(Self::setting_row(
                format!("{}:", t!("prefs.tab_width")),
                Input::new(&q.tab_width).w(tokens::form::FIELD_WIDTH_SHORT.rems()),
            ))
            .child(check(
                "word-wrap",
                t!("prefs.word_wrap").to_string(),
                s.word_wrap,
                |s, v| s.word_wrap = v,
            ))
            .child(Self::section(t!("prefs.font_colors").to_string()))
            .child(check(
                "use-default-editor-font",
                t!("prefs.use_default_font").to_string(),
                s.use_default_editor_font,
                |s, v| s.use_default_editor_font = v,
            ))
            .child(Self::setting_row(
                format!("{}:", t!("prefs.font_size")),
                Input::new(&q.editor_font)
                    .w(tokens::form::FIELD_WIDTH_SHORT.rems())
                    .disabled(s.use_default_editor_font),
            ))
            .child(Self::section(t!("prefs.syntax_colors").to_string()))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_3()
                    .children(q.colors.iter().map(|(key, state)| {
                        h_flex()
                            .gap_1()
                            .child(ColorPicker::new(state).small())
                            .child(label(key))
                    })),
            )
            .child(
                h_flex().child(
                    Button::new("syntax-reset")
                        .small()
                        .label(t!("prefs.reset_colors").to_string())
                        .on_click(|_, window, cx| {
                            let dark = cx.theme().is_dark();
                            Settings::update(cx, |s| {
                                if dark {
                                    s.syntax_colors.dark.clear()
                                } else {
                                    s.syntax_colors.light.clear()
                                }
                            });
                            crate::apply_theme(Some(window), cx);
                        }),
                ),
            )
            .child(form_note(t!("prefs.preview").to_string(), cx))
            .child(
                div()
                    .h(tokens::viewer::CELL_EDITOR_HEIGHT.rems())
                    .child(Editor::new(&q.preview).readonly(true).size_full()),
            )
    }

    /// Code completion (F-PREF-006).
    pub(super) fn render_completion(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(
                h_flex()
                    .gap_3()
                    .child(check(
                        "code-completion",
                        t!("prefs.code_completion").to_string(),
                        s.code_completion,
                        |s, v| s.code_completion = v,
                    ))
                    .child(
                        Button::new("clear-outline")
                            .small()
                            .label(t!("prefs.clear_outline").to_string())
                            .on_click(|_, _, cx| {
                                navidog_db::outline::OutlineCache::global(cx)
                                    .update(cx, |c, cx| c.clear_all(cx))
                            }),
                    ),
            )
            .child(check(
                "auto-update-outline",
                t!("prefs.auto_update_outline").to_string(),
                s.auto_update_outline,
                |s, v| s.auto_update_outline = v,
            ))
            .child(check(
                "outline-system",
                t!("prefs.outline_system").to_string(),
                s.outline_system_objects,
                |s, v| s.outline_system_objects = v,
            ))
            .child(check(
                "auto-select-first",
                t!("prefs.auto_select_first").to_string(),
                s.auto_select_first,
                |s, v| s.auto_select_first = v,
            ))
    }

    /// Auto recovery (F-PREF-008).
    pub(super) fn render_recovery(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let label = |v: Option<u32>| {
            match v {
                Some(15) => t!("prefs.every_15s"),
                Some(30) => t!("prefs.every_30s"),
                Some(60) => t!("prefs.every_minute"),
                Some(180) => t!("prefs.every_3m"),
                Some(300) => t!("prefs.every_5m"),
                _ => t!("prefs.never"),
            }
            .to_string()
        };
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::section(t!("prefs.autosave_queries").to_string()))
            .child(
                AUTOSAVE
                    .iter()
                    .enumerate()
                    .fold(
                        RadioGroup::vertical("autosave")
                            .selected_index(AUTOSAVE.iter().position(|v| *v == s.autosave_secs)),
                        |g, (ix, v)| g.child(Radio::new(("autosave-option", ix)).label(label(*v))),
                    )
                    .on_click(|ix: &usize, _, cx| {
                        let v = AUTOSAVE[*ix];
                        Settings::update(cx, |s| s.autosave_secs = v);
                    }),
            )
    }

    /// Advanced: where the logs go, and whether the app log records debug messages.
    pub(super) fn render_logs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Settings::global(cx).clone();
        let p = palette(cx);
        let dir = s.log_dir();
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(Self::section(t!("prefs.log_folder").to_string()))
            .child(
                truncated("log-folder", dir.display().to_string())
                    .w_full()
                    .text_color(p.label_secondary),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("log-choose")
                            .label(t!("prefs.choose").to_string())
                            .on_click(|_, window, cx| {
                                let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                                    files: false,
                                    directories: true,
                                    multiple: false,
                                    prompt: None,
                                });
                                window
                                    .spawn(cx, async move |cx| {
                                        let Ok(Ok(Some(paths))) = picked.await else {
                                            return;
                                        };
                                        let Some(dir) = paths.into_iter().next() else {
                                            return;
                                        };
                                        let _ = cx.update(|_, cx| {
                                            Settings::update(cx, |s| {
                                                s.log_dir = Some(dir.display().to_string())
                                            });
                                            // QueryExec.log moves now; the app log at the next start
                                            navidog_db::log::set_dir(dir);
                                            cx.refresh_windows();
                                        });
                                    })
                                    .detach();
                            }),
                    )
                    .child(
                        Button::new("log-reveal")
                            .label(t!("prefs.reveal").to_string())
                            .on_click(move |_, _, cx| cx.reveal_path(&dir)),
                    ),
            )
            .child(check(
                "diagnostic-log",
                t!("prefs.diagnostic_log").to_string(),
                s.diagnostic_log,
                |s, v| s.diagnostic_log = v,
            ))
            .child(form_note(t!("prefs.diagnostic_hint").to_string(), cx))
    }
}
