//! Snippet placeholders (30-query-editor.md §5): `${text}` in a snippet becomes a tab stop.
//! Tab and ⇧Tab move between them; leaving the last one, Esc, or a caret outside all of them ends
//! the jumps. The stops are range decorations, so they follow edits.

use std::ops::Range;

use gpui_kit::component::input::{
    RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::{Context, Window};
use navidog_ui::palette;

use crate::editor::QueryEditor;

pub(crate) struct TabStops {
    collection: RangeDecorationCollection,
    current: usize,
}

/// The snippet text with its `${…}` markers removed, and the ranges of the placeholder texts.
/// PostgreSQL's `$1` and `$$` have no braces and stay as they are.
pub(crate) fn expand(code: &str) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(code.len());
    let mut ranges = Vec::new();
    let mut rest = code;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start + 2..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = &rest[start + 2..start + 2 + len];
        let at = out.len();
        out.push_str(name);
        ranges.push(at..at + name.len());
        rest = &rest[start + 2 + len + 1..];
    }
    out.push_str(rest);
    (out, ranges)
}

/// Wraps the selected text of `code` as a placeholder ("Add Placeholder" in the snippet editor).
pub(crate) fn add_placeholder(code: &str, selection: Range<usize>) -> String {
    if selection.is_empty() || selection.end > code.len() {
        return code.to_string();
    }
    format!(
        "{}${{{}}}{}",
        &code[..selection.start],
        &code[selection.clone()],
        &code[selection.end..]
    )
}

/// Turns every `${text}` back into `text` ("Remove Placeholder").
pub(crate) fn remove_placeholders(code: &str) -> String {
    expand(code).0
}

impl QueryEditor {
    pub(crate) fn insert_snippet(
        &mut self,
        code: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (text, ranges) = expand(code);
        let start = self.editor.read(cx).selected_range().start;
        self.editor.update(cx, |e, cx| {
            e.replace(text, window, cx);
            e.focus(window, cx);
        });
        self.start_tab_stops(
            ranges
                .into_iter()
                .map(|r| r.start + start..r.end + start)
                .collect(),
            cx,
        );
    }

    pub(crate) fn start_tab_stops(&mut self, ranges: Vec<Range<usize>>, cx: &mut Context<Self>) {
        self.end_tab_stops(cx);
        let Some(first) = ranges.first().cloned() else {
            return;
        };
        let color = palette(cx).success.opacity(0.25);
        let decorations = ranges
            .into_iter()
            .map(|r| {
                RangeDecoration::new(r)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();
        let collection = self.editor.update(cx, |e, cx| {
            e.create_range_decorations_collection(decorations, cx)
        });
        self.editor
            .update(cx, |e, cx| e.set_selected_range(first, cx));
        self.tab_stops = Some(TabStops {
            collection,
            current: 0,
        });
    }

    pub(crate) fn end_tab_stops(&mut self, cx: &mut Context<Self>) {
        if let Some(stops) = self.tab_stops.take() {
            stops.collection.dispose(cx);
        }
    }

    /// Tab / ⇧Tab while tab stops are active. Returns whether the key was used.
    pub(crate) fn step_tab_stop(
        &mut self,
        forward: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(stops) = self.tab_stops.as_ref() else {
            return false;
        };
        let ranges = stops.collection.get_ranges(cx);
        let caret = self.editor.read(cx).cursor();
        let inside = ranges.iter().any(|r| r.start <= caret && caret <= r.end);
        if !inside {
            self.end_tab_stops(cx);
            return false;
        }
        let next = if forward {
            stops.current + 1
        } else {
            match stops.current.checked_sub(1) {
                Some(n) => n,
                None => return true,
            }
        };
        let Some(range) = ranges.get(next).cloned() else {
            // Past the last placeholder: the jumps end and the caret stays put
            self.end_tab_stops(cx);
            return true;
        };
        if let Some(stops) = self.tab_stops.as_mut() {
            stops.current = next;
        }
        self.editor
            .update(cx, |e, cx| e.set_selected_range(range, cx));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_expand_to_their_text() {
        let (text, ranges) = expand("SELECT ${cols} FROM ${table} WHERE id = $1");
        assert_eq!(text, "SELECT cols FROM table WHERE id = $1");
        assert_eq!(ranges, [7..11, 17..22]);
        assert_eq!(&text[ranges[1].clone()], "table");
        assert_eq!(expand("a ${unclosed").0, "a ${unclosed");
    }

    #[test]
    fn placeholders_are_added_and_removed() {
        let code = "SELECT * FROM t";
        let with = add_placeholder(code, 14..15);
        assert_eq!(with, "SELECT * FROM ${t}");
        assert_eq!(remove_placeholders(&with), code);
        assert_eq!(add_placeholder(code, 3..3), code);
    }
}
