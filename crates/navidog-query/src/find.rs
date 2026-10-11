//! Find and replace bar of the query editor (30-query-editor.md §3): regular expressions, match
//! case and whole word on top of incremental search. Matches are fill decorations; the current
//! match is the selection. Search starts at the caret and does not wrap, like Navicat.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Input, InputEvent, InputState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    actions, div,
};
use navidog_ui::assets::lucide;
use navidog_ui::{palette, tokens};
use regex::{Regex, RegexBuilder};
use rust_i18n::t;

use crate::editor::QueryEditor;

actions!(
    query,
    [
        FindPrevious,
        ToggleFindRegex,
        ToggleFindCase,
        ToggleFindWholeWord,
        FindLiteral,
        FindReplace,
    ]
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FindOptions {
    pub regex: bool,
    pub case: bool,
    pub whole_word: bool,
}

/// The pattern for a query; `Err` when the regular expression does not compile.
pub(crate) fn pattern(query: &str, o: FindOptions) -> Result<Regex, regex::Error> {
    let mut source = if o.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if o.whole_word {
        source = format!(r"\b(?:{source})\b");
    }
    RegexBuilder::new(&source)
        .case_insensitive(!o.case)
        .multi_line(true)
        .build()
}

/// Non-empty matches of `query` in `text`.
pub(crate) fn find_all(
    text: &str,
    query: &str,
    o: FindOptions,
) -> Result<Vec<Range<usize>>, regex::Error> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    Ok(pattern(query, o)?
        .find_iter(text)
        .filter(|m| !m.is_empty())
        .map(|m| m.range())
        .collect())
}

/// The replacement for one match; with regular expressions `$1` and `${name}` expand.
pub(crate) fn replacement(re: &Regex, matched: &str, with: &str, o: FindOptions) -> String {
    if o.regex {
        re.replace(matched, with).into_owned()
    } else {
        with.to_string()
    }
}

/// `text` with every match replaced, and how many there were.
pub(crate) fn replace_all(
    text: &str,
    query: &str,
    with: &str,
    o: FindOptions,
) -> Result<(String, usize), regex::Error> {
    let re = pattern(query, o)?;
    let mut n = 0;
    let out = re.replace_all(text, |caps: &regex::Captures| {
        n += 1;
        if o.regex {
            let mut s = String::new();
            caps.expand(with, &mut s);
            s
        } else {
            with.to_string()
        }
    });
    Ok((out.into_owned(), n))
}

pub(crate) struct Find {
    pub open: bool,
    pub replace_mode: bool,
    pub query: Entity<InputState>,
    pub replacement: Entity<InputState>,
    pub options: FindOptions,
    pub error: bool,
    pub matches: Vec<Range<usize>>,
    decorations: Option<RangeDecorationCollection>,
    _subscriptions: Vec<Subscription>,
}

impl Find {
    pub fn new(window: &mut Window, cx: &mut Context<QueryEditor>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("query.find").to_string()));
        let replacement =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("query.replace").to_string()));
        let subs = vec![
            cx.subscribe_in(
                &query,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::Change => this.find_from_caret(window, cx),
                    InputEvent::PressEnter { shift, .. } => this.find_step(!*shift, window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &replacement,
                window,
                |this, _, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.replace_one(window, cx);
                    }
                },
            ),
        ];
        Self {
            open: false,
            replace_mode: false,
            query,
            replacement,
            options: FindOptions::default(),
            error: false,
            matches: Vec::new(),
            decorations: None,
            _subscriptions: subs,
        }
    }
}

impl QueryEditor {
    pub(crate) fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.editor.read(cx).selected_text().to_string();
        self.find.open = true;
        self.find.replace_mode = replace;
        if !selected.is_empty() && !selected.contains('\n') {
            self.find
                .query
                .update(cx, |s, cx| s.set_value(selected, window, cx));
        }
        self.find.query.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        self.refresh_matches(cx);
        cx.notify();
    }

    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = false;
        self.find.matches.clear();
        if let Some(d) = &self.find.decorations {
            d.clear(cx);
        }
        self.editor.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    /// Recomputes and highlights every match; a bad regular expression turns the field red.
    pub(crate) fn refresh_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.find.query.read(cx).value().to_string();
        let text = self.editor.read(cx).value().to_string();
        match find_all(&text, &query, self.find.options) {
            Ok(m) => {
                self.find.error = false;
                self.find.matches = m;
            }
            Err(_) => {
                self.find.error = true;
                self.find.matches.clear();
            }
        }
        let color = palette(cx).find_highlight.opacity(0.45);
        let decorations: Vec<RangeDecoration> = self
            .find
            .matches
            .iter()
            .map(|r| {
                RangeDecoration::new(r.clone())
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();
        match &self.find.decorations {
            Some(d) => d.set(decorations, cx),
            None => {
                let d = self.editor.update(cx, |e, cx| {
                    e.create_range_decorations_collection(decorations, cx)
                });
                self.find.decorations = Some(d);
            }
        }
        cx.notify();
    }

    /// Incremental search: the first match at or after the caret.
    fn find_from_caret(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_matches(cx);
        let from = self.editor.read(cx).selected_range().start;
        if let Some(m) = self.find.matches.iter().find(|m| m.start >= from).cloned() {
            self.editor.update(cx, |e, cx| e.set_selected_range(m, cx));
        }
    }

    /// Next or previous match from the selection; stops at the ends of the text.
    pub(crate) fn find_step(&mut self, forward: bool, _: &mut Window, cx: &mut Context<Self>) {
        if !self.find.open {
            return;
        }
        self.refresh_matches(cx);
        let sel = self.editor.read(cx).selected_range();
        let next = if forward {
            self.find
                .matches
                .iter()
                .find(|m| m.start >= sel.end && **m != sel)
                .cloned()
        } else {
            self.find
                .matches
                .iter()
                .rev()
                .find(|m| m.end <= sel.start)
                .cloned()
        };
        if let Some(m) = next {
            self.editor.update(cx, |e, cx| e.set_selected_range(m, cx));
        }
    }

    /// Replaces the current match and moves to the next one.
    fn replace_one(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.find.query.read(cx).value().to_string();
        let with = self.find.replacement.read(cx).value().to_string();
        let Ok(re) = pattern(&query, self.find.options) else {
            return;
        };
        self.refresh_matches(cx);
        let sel = self.editor.read(cx).selected_range();
        if !self.find.matches.contains(&sel) {
            self.find_step(true, window, cx);
            return;
        }
        let matched = self.editor.read(cx).selected_text().to_string();
        let new = replacement(&re, &matched, &with, self.find.options);
        self.editor.update(cx, |e, cx| e.replace(new, window, cx));
        self.find_step(true, window, cx);
    }

    /// Replaces every match in one undoable edit.
    fn replace_every(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.find.query.read(cx).value().to_string();
        let with = self.find.replacement.read(cx).value().to_string();
        let text = self.editor.read(cx).value().to_string();
        let Ok((new, n)) = replace_all(&text, &query, &with, self.find.options) else {
            return;
        };
        if n == 0 {
            return;
        }
        let len = text.len();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(0..len, cx);
            e.replace(new, window, cx);
        });
        self.refresh_matches(cx);
    }

    fn find_menu(&self) -> NativeMenu {
        NativeMenu::new()
            .menu_with_check(
                t!("query.find").to_string(),
                !self.find.replace_mode,
                Box::new(FindLiteral),
            )
            .menu_with_check(
                t!("query.replace").to_string(),
                self.find.replace_mode,
                Box::new(FindReplace),
            )
            .separator()
            .menu_with_check(
                t!("query.find_regex").to_string(),
                self.find.options.regex,
                Box::new(ToggleFindRegex),
            )
            .menu_with_check(
                t!("query.find_case").to_string(),
                self.find.options.case,
                Box::new(ToggleFindCase),
            )
            .menu_with_check(
                t!("query.find_whole_word").to_string(),
                self.find.options.whole_word,
                Box::new(ToggleFindWholeWord),
            )
    }

    pub(crate) fn toggle_find_option(
        &mut self,
        f: impl FnOnce(&mut FindOptions),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.find.options);
        self.find_from_caret(window, cx);
    }

    pub(crate) fn render_find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let count = self.find.matches.len();
        let mode = if self.find.replace_mode {
            t!("query.replace").to_string()
        } else {
            t!("query.find").to_string()
        };
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .items_start()
            .border_b_1()
            .border_color(p.separator)
            .child(
                h_flex()
                    .id("find-mode")
                    .flex_none()
                    .px_2()
                    .gap_1()
                    .h(tokens::query::FIND_CONTROL_HEIGHT.rems())
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(if self.find.error {
                        p.danger
                    } else {
                        p.separator
                    })
                    .bg(p.fill_tertiary)
                    .cursor_pointer()
                    .child(mode)
                    .child(
                        lucide("chevron-down")
                            .size(tokens::navigator::ROW_ICON.rems())
                            .text_color(p.label_secondary),
                    )
                    .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                        this.focus_for_menu(window, cx);
                        this.find_menu().show(e.position(), window, cx);
                    })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .rounded_md()
                                    .when(self.find.error, |d| d.border_1().border_color(p.danger))
                                    .child(Input::new(&self.find.query).small()),
                            )
                            .child(
                                Button::new("find-prev")
                                    .ghost()
                                    .small()
                                    .icon(lucide("chevron-left"))
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.find_step(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("find-next")
                                    .ghost()
                                    .small()
                                    .icon(lucide("chevron-right"))
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.find_step(true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("find-done")
                                    .small()
                                    .label(t!("query.find_done").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_find(window, cx)
                                    })),
                            ),
                    )
                    .when(self.find.replace_mode, |col| {
                        col.child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.find.replacement).small()),
                                )
                                .child(
                                    Button::new("find-replace-one")
                                        .small()
                                        .label(t!("query.replace").to_string())
                                        .disabled(count == 0)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.replace_one(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("find-replace-all")
                                        .small()
                                        .label(t!("query.replace_all").to_string())
                                        .disabled(count == 0)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.replace_every(window, cx)
                                        })),
                                ),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_shape_the_matches() {
        let text = "Select a; select selector; SELECTED";
        let plain = FindOptions::default();
        assert_eq!(find_all(text, "select", plain).unwrap().len(), 4);
        let case = FindOptions {
            case: true,
            ..plain
        };
        assert_eq!(find_all(text, "select", case).unwrap(), [10..16, 17..23]);
        let word = FindOptions {
            whole_word: true,
            ..plain
        };
        assert_eq!(find_all(text, "select", word).unwrap(), [0..6, 10..16]);
        let re = FindOptions {
            regex: true,
            ..plain
        };
        assert_eq!(find_all(text, r"sel\w+or", re).unwrap(), vec![17..25]);
        assert!(find_all(text, "(", re).is_err());
        // A literal search escapes regex syntax
        assert_eq!(find_all("a(b", "(", plain).unwrap(), vec![1..2]);
    }

    #[test]
    fn replace_all_expands_groups_in_regex_mode() {
        let re = FindOptions {
            regex: true,
            ..FindOptions::default()
        };
        let (out, n) = replace_all("id1 id22", r"id(\d+)", "n$1", re).unwrap();
        assert_eq!((out.as_str(), n), ("n1 n22", 2));
        let plain = FindOptions::default();
        let (out, n) = replace_all("a.b.c", ".", "$1", plain).unwrap();
        assert_eq!((out.as_str(), n), ("a$1b$1c", 2));
    }
}
