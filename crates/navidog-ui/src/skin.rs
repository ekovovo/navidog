//! Skin components with the AppKit look. Behavior comes from GPUI elements and gpui-component
//! components; these only handle appearance and sizing.

use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};

use crate::tokens::{self, palette};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Single-line text as Navicat shows it: never wraps, overflow ends in an ellipsis, and the full
/// text appears on hover. Use it for every label whose content comes from data or user input.
// ponytail: the tooltip also shows when the text fits; measuring the shaped line would hide it then
pub fn truncated(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let text = text.into();
    let tip = text.clone();
    // GPUI's nowrap still lays out explicit line breaks as separate lines.
    let display = if text.contains(['\r', '\n']) {
        text.replace(['\r', '\n'], " ").into()
    } else {
        text
    };
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .line_clamp(1)
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .child(display)
}

/// Makes the dialog's Confirm (Enter in a single-line field, or on the dialog itself) call `ok` on
/// `view`, the same method its OK button calls. `ok` closes the dialog itself when it succeeds.
/// Do not also subscribe to the field's PressEnter: the input propagates Enter, so both would run.
pub fn confirm_with<V: 'static>(
    dialog: gpui_kit::component::dialog::Dialog,
    view: &gpui_kit::Entity<V>,
    ok: fn(&mut V, &mut Window, &mut gpui_kit::Context<V>),
) -> gpui_kit::component::dialog::Dialog {
    let view = view.clone();
    dialog.on_ok(move |_, window, cx| {
        view.update(cx, |v, cx| ok(v, window, cx));
        false
    })
}

/// Byte ranges of `query` in `text`, ignoring ASCII case (folding ASCII keeps byte offsets valid).
pub fn find_matches(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let (t, q) = (text.as_bytes(), query.as_bytes());
    let mut out = Vec::new();
    let mut i = 0;
    while !q.is_empty() && i + q.len() <= t.len() {
        if t[i..i + q.len()].eq_ignore_ascii_case(q) {
            out.push(i..i + q.len());
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

/// `text` with every match of `query` drawn on the find highlight.
pub fn highlighted(text: impl Into<SharedString>, query: &str, cx: &App) -> gpui_kit::StyledText {
    let text = text.into();
    let style = gpui_kit::HighlightStyle {
        background_color: Some(palette(cx).find_highlight),
        color: Some(gpui_kit::black()),
        ..Default::default()
    };
    let ranges = find_matches(&text, query);
    gpui_kit::StyledText::new(text).with_highlights(ranges.into_iter().map(|r| (r, style)))
}

/// Main toolbar item: icon on top, label below (Navicat's Expanded toolbar style).
#[derive(IntoElement)]
pub struct ToolbarItem {
    id: ElementId,
    icon: Icon,
    label: SharedString,
    icon_color: Option<Hsla>,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl ToolbarItem {
    pub fn new(id: impl Into<ElementId>, icon: Icon, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            icon_color: None,
            disabled: false,
            on_click: None,
        }
    }

    pub fn icon_color(mut self, color: Hsla) -> Self {
        self.icon_color = Some(color);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for ToolbarItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let fg = if self.disabled {
            p.disabled_text
        } else {
            p.label
        };
        let icon = self.icon.size(tokens::toolbar::ITEM_ICON.rems());
        let icon = match self.icon_color {
            Some(color) if !self.disabled => icon.text_color(color),
            _ => icon,
        };
        v_flex()
            .id(self.id)
            .items_center()
            .justify_center()
            .min_w(tokens::toolbar::ITEM_MIN_WIDTH.rems())
            .px_1()
            .gap_0p5()
            .rounded_md()
            .text_color(fg)
            .child(icon)
            .child(
                div()
                    .text_size(tokens::font::SIZE_SMALL.rems())
                    .line_height(tokens::font::LINE_SMALL.rems())
                    .child(self.label),
            )
            .when(!self.disabled, |this| {
                this.active(move |s| s.bg(p.fill_tertiary))
                    .when_some(self.on_click, |this, on_click| {
                        this.on_click(move |e, w, cx| on_click(e, w, cx))
                    })
            })
    }
}

/// Form row: right-aligned label + 6pt gap + control, matching the nib layout of Navicat's
/// connection form.
#[derive(IntoElement)]
pub struct FormRow {
    label: Option<SharedString>,
    control: AnyElement,
    compact: bool,
    hug: bool,
    label_width: tokens::Pt,
}

impl FormRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: Some(label.into()),
            control: control.into_any_element(),
            compact: false,
            hug: false,
            label_width: tokens::form::LABEL_COLUMN,
        }
    }

    /// A row without a label whose control is left-aligned with the controls above (e.g. a checkbox).
    pub fn bare(control: impl IntoElement) -> Self {
        Self {
            label: None,
            control: control.into_any_element(),
            compact: false,
            hug: false,
            label_width: tokens::form::LABEL_COLUMN,
        }
    }

    /// Size the label to its text instead of the connection form's label column.
    /// Small dialogs use this so a short label does not push the field to the right.
    pub fn hug_label(mut self) -> Self {
        self.hug = true;
        self
    }

    /// Compact row (22pt) used on the Advanced page.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

impl RenderOnce for FormRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let height = if self.compact {
            tokens::form::COMPACT_ROW_HEIGHT
        } else {
            tokens::form::ROW_HEIGHT
        };
        let hug = self.hug;
        h_flex()
            .min_h(height.rems())
            .gap(tokens::form::LABEL_GAP.rems())
            .when(hug, |row| row.w_full().min_w_0())
            // A hugged row with no label (a checkbox, an error) starts at the left edge.
            // The connection form keeps an empty column so those rows line up with the fields.
            .when(!hug || self.label.is_some(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .when(!hug, |d| d.w(self.label_width.rems()))
                        .text_right()
                        .text_color(p.label)
                        .line_height(tokens::form::LABEL_HEIGHT.rems())
                        .children(self.label.map(|l| SharedString::from(format!("{l}:")))),
                )
            })
            .child(div().flex_1().min_w_0().child(self.control))
    }
}

/// 11pt note text below a form.
pub fn form_note(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_size(tokens::font::SIZE_SMALL.rems())
        .line_height(tokens::form::NOTE_HEIGHT.rems())
        .text_color(palette(cx).label_secondary)
        .child(text.into())
}

/// Dialog heading (Semibold 13pt).
pub fn heading(text: impl Into<SharedString>) -> impl IntoElement {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

#[cfg(test)]
mod tests {
    use super::find_matches;

    #[test]
    fn matches_ignore_ascii_case_and_keep_utf8_offsets() {
        assert_eq!(
            find_matches("本地 MySQL mysql", "mysql"),
            vec![7..12, 13..18]
        );
        assert_eq!(find_matches("数据库", "据"), vec![3..6]);
        assert_eq!(find_matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(find_matches("abc", "").is_empty());
        assert!(find_matches("ab", "abc").is_empty());
    }
}
