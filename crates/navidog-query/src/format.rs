//! Format menu, clipboard stack and bracket highlighting (30-query-editor.md §3). Each command
//! replaces its text in one edit, so one undo reverts it.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Input, InputState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _, Render,
    Styled as _, Window, actions,
};
use navidog_core::settings::Settings;
use navidog_db::dialect::Dialect;
use navidog_ui::skin::{FormRow, confirm_with, heading};
use navidog_ui::{palette, tokens};
use navidog_util::sql_format::{self, BeautifyOptions, CommentMode};
use rust_i18n::t;

use crate::editor::QueryEditor;

actions!(
    query,
    [
        CommentLines,
        UncommentLines,
        ToggleComment,
        UpperCase,
        LowerCase,
        Beautify,
        BeautifyOptionsDialog,
        Minify,
        PasteFromStack,
    ]
);

/// Text copied or cut in any query editor, newest first (30 §3).
#[derive(Default)]
pub(crate) struct ClipboardStack(pub Vec<String>);
impl Global for ClipboardStack {}

pub(crate) const STACK_SIZE: usize = 10;

impl ClipboardStack {
    pub fn push(&mut self, text: String) {
        if text.is_empty() || self.0.first() == Some(&text) {
            return;
        }
        self.0.insert(0, text);
        self.0.truncate(STACK_SIZE);
    }
}

/// The last paste from the stack, so pressing ⇧⌘V again swaps in the next item.
pub(crate) struct StackPaste {
    range: Range<usize>,
    index: usize,
}

impl QueryEditor {
    fn selection_and_text(&self, cx: &App) -> (String, Range<usize>) {
        let e = self.editor.read(cx);
        (e.value().to_string(), e.selected_range())
    }

    /// Replaces `range` with `new` and selects the result.
    fn replace_range(
        &mut self,
        range: Range<usize>,
        new: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end = range.start + new.len();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(range.clone(), cx);
            e.replace(new, window, cx);
            e.set_selected_range(range.start..end, cx);
        });
    }

    /// Commands on whole lines: the selected lines, or the caret's.
    fn on_lines(
        &mut self,
        f: impl FnOnce(&str) -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, sel) = self.selection_and_text(cx);
        let range = sql_format::line_range(&text, sel);
        let new = f(&text[range.clone()]);
        if new != text[range.clone()] {
            self.replace_range(range, new, window, cx);
        }
    }

    /// Commands on the selected text, or all of it when nothing is selected.
    fn on_selection(
        &mut self,
        f: impl FnOnce(&str) -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, sel) = self.selection_and_text(cx);
        let range = if sel.is_empty() { 0..text.len() } else { sel };
        let new = f(&text[range.clone()]);
        if new != text[range.clone()] {
            self.replace_range(range, new, window, cx);
        }
    }

    pub(crate) fn comment(
        &mut self,
        mode: CommentMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_lines(|s| sql_format::comment_lines(s, mode), window, cx);
    }

    pub(crate) fn change_case(&mut self, upper: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.on_selection(
            |s| {
                if upper {
                    s.to_uppercase()
                } else {
                    s.to_lowercase()
                }
            },
            window,
            cx,
        );
    }

    pub(crate) fn beautify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Settings::global(cx);
        let dialect = self.dialect(cx);
        let options = BeautifyOptions {
            indent_spaces: s.insert_spaces.then_some(s.tab_width.clamp(1, 16)),
            uppercase: s.beautify_uppercase,
            max_inline: s.beautify_max_inline,
            postgres: dialect == Some(Dialect::Postgres),
            mysql: dialect == Some(Dialect::MySql),
        };
        self.on_selection(|s| sql_format::beautify(s, &options), window, cx);
    }

    pub(crate) fn minify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mysql = self.dialect(cx) == Some(Dialect::MySql);
        self.on_selection(|s| sql_format::minify(s, mysql), window, cx);
    }

    /// Copy and Cut in the editor also push the selection onto the clipboard stack.
    pub(crate) fn remember_copy(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).selected_text().to_string();
        cx.default_global::<ClipboardStack>().push(text);
    }

    /// ⇧⌘V: pastes the newest item; pressed again right after, the next one replaces it.
    pub(crate) fn paste_from_stack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stack = cx.default_global::<ClipboardStack>().0.clone();
        if stack.is_empty() {
            return;
        }
        let (text, sel) = self.selection_and_text(cx);
        let again = self.stack_paste.as_ref().filter(|p| {
            sel == (p.range.end..p.range.end)
                && text.get(p.range.clone()) == stack.get(p.index).map(String::as_str)
        });
        let (range, index) = match again {
            Some(p) => (p.range.clone(), (p.index + 1) % stack.len()),
            None => (sel, 0),
        };
        let item = stack[index].clone();
        let end = range.start + item.len();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(range.clone(), cx);
            e.replace(item, window, cx);
            e.set_selected_range(end..end, cx);
        });
        self.stack_paste = Some(StackPaste {
            range: range.start..end,
            index,
        });
    }

    /// Frames the bracket at the caret and its partner (30 §3), when the preference is on.
    pub(crate) fn sync_brackets(&mut self, cx: &mut Context<Self>) {
        let enabled = Settings::global(cx).bracket_highlight;
        let e = self.editor.read(cx);
        let caret = e.cursor();
        let sel_empty = e.selected_range().is_empty();
        if self.bracket_caret == Some(caret) {
            return;
        }
        self.bracket_caret = Some(caret);
        let text = e.text();
        let near = |i: Option<usize>| {
            i.filter(|i| *i < text.len())
                .and_then(|i| text.get_char(i).ok())
                .is_some_and(|c| c == '(' || c == ')')
        };
        let pair = if enabled && sel_empty && (near(Some(caret)) || near(caret.checked_sub(1))) {
            // ponytail: scans the whole text when the caret touches a bracket; fine up to a few MB
            let value = e.value();
            let mysql = self.dialect(cx) == Some(Dialect::MySql);
            sql_format::bracket_at(&sql_format::bracket_pairs(&value, mysql), caret)
        } else {
            None
        };
        let decorations: Vec<RangeDecoration> = pair
            .map(|(a, b)| {
                [a, b]
                    .into_iter()
                    .map(|i| RangeDecoration::new(i..i + 1).with_style(RangeDecorationStyle::Frame))
                    .collect()
            })
            .unwrap_or_default();
        match &self.brackets {
            Some(d) => d.set(decorations, cx),
            None if !decorations.is_empty() => {
                let d: RangeDecorationCollection = self.editor.update(cx, |e, cx| {
                    e.create_range_decorations_collection(decorations, cx)
                });
                self.brackets = Some(d);
            }
            None => {}
        }
    }
}

/// "Beautify SQL Options" (356 × 143): short parentheses length and uppercase keywords.
struct BeautifyDialog {
    editor: gpui_kit::WeakEntity<QueryEditor>,
    max_inline: Entity<InputState>,
    uppercase: bool,
}

pub(crate) fn open_beautify_options(
    editor: gpui_kit::WeakEntity<QueryEditor>,
    window: &mut Window,
    cx: &mut App,
) {
    let s = Settings::global(cx);
    let (max, upper) = (s.beautify_max_inline, s.beautify_uppercase);
    let view = cx.new(|cx| BeautifyDialog {
        editor,
        max_inline: cx.new(|cx| InputState::new(window, cx).default_value(max.to_string())),
        uppercase: upper,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(
            dialog
                .w(tokens::query::BEAUTIFY_WIDTH.to_pixels(window.rem_size()))
                .close_button(false),
            &view,
            BeautifyDialog::beautify,
        )
        .child(view.clone())
    });
}

impl BeautifyDialog {
    fn save(&mut self, cx: &mut Context<Self>) {
        let max = self
            .max_inline
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(50);
        let upper = self.uppercase;
        Settings::update(cx, |s| {
            s.beautify_max_inline = max;
            s.beautify_uppercase = upper;
        });
    }

    fn beautify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
        window.close_dialog(cx);
        let _ = self.editor.update(cx, |e, cx| e.beautify(window, cx));
    }
}

impl Render for BeautifyDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = palette(cx);
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(heading(t!("query.beautify_options").to_string()))
            .child(
                FormRow::new(
                    t!("query.short_paren").to_string(),
                    Input::new(&self.max_inline)
                        .small()
                        .w(tokens::form::FIELD_WIDTH_SHORT.rems()),
                )
                .hug_label(),
            )
            .child(
                FormRow::bare(
                    Checkbox::new("beautify-uppercase")
                        .label(t!("query.uppercase_keywords").to_string())
                        .checked(self.uppercase)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.uppercase = *checked;
                            cx.notify();
                        })),
                )
                .hug_label(),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("beautify-default")
                            .label(t!("query.default").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.uppercase = true;
                                this.max_inline
                                    .update(cx, |s, cx| s.set_value("50", window, cx));
                                cx.notify();
                            })),
                    )
                    .child(gpui_kit::div().flex_1())
                    .child(
                        Button::new("beautify-save")
                            .label(t!("query.save").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save(cx);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("beautify-cancel")
                            .label(t!("query.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("beautify-run")
                            .primary()
                            .label(t!("query.beautify").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.beautify(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_keeps_ten_without_repeating_the_top() {
        let mut s = ClipboardStack::default();
        for i in 0..12 {
            s.push(i.to_string());
        }
        s.push("11".into());
        s.push(String::new());
        assert_eq!(s.0.len(), STACK_SIZE);
        assert_eq!(s.0[0], "11");
        assert_eq!(s.0[9], "2");
    }
}
