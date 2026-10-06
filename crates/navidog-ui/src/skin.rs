//! AppKit 外观的皮肤组件。行为沿用 GPUI 元素与 gpui-component 组件，只负责外观与尺寸。

use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};

use crate::tokens::{self, palette};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// 主工具栏条目：图标在上、文字在下（Navicat Expanded 样式工具栏）。
#[derive(IntoElement)]
pub struct ToolbarItem {
    id: ElementId,
    icon: Icon,
    label: SharedString,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl ToolbarItem {
    pub fn new(id: impl Into<ElementId>, icon: Icon, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            disabled: false,
            on_click: None,
        }
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
        v_flex()
            .id(self.id)
            .items_center()
            .justify_center()
            .min_w(tokens::toolbar::ITEM_MIN_WIDTH.rems())
            .px_1()
            .gap_0p5()
            .rounded_md()
            .text_color(fg)
            .child(self.icon.size(tokens::toolbar::ITEM_ICON.rems()))
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

/// 表单行：右对齐标签 + 6pt 间距 + 控件，对齐 Navicat 连接表单的 nib 布局。
#[derive(IntoElement)]
pub struct FormRow {
    label: Option<SharedString>,
    control: AnyElement,
    compact: bool,
    label_width: tokens::Pt,
}

impl FormRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: Some(label.into()),
            control: control.into_any_element(),
            compact: false,
            label_width: tokens::form::LABEL_COLUMN,
        }
    }

    /// 没有标签、控件与上方控件左对齐的行（例如复选框）。
    pub fn bare(control: impl IntoElement) -> Self {
        Self {
            label: None,
            control: control.into_any_element(),
            compact: false,
            label_width: tokens::form::LABEL_COLUMN,
        }
    }

    /// 高级页的紧凑行（22pt）。
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
        h_flex()
            .min_h(height.rems())
            .gap(tokens::form::LABEL_GAP.rems())
            .child(
                div()
                    .flex_none()
                    .w(self.label_width.rems())
                    .text_right()
                    .text_color(p.label)
                    .line_height(tokens::form::LABEL_HEIGHT.rems())
                    .children(self.label.map(|l| SharedString::from(format!("{l}:")))),
            )
            .child(div().flex_1().min_w_0().child(self.control))
    }
}

/// 表单下方的 11pt 说明文字。
pub fn form_note(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_size(tokens::font::SIZE_SMALL.rems())
        .line_height(tokens::form::NOTE_HEIGHT.rems())
        .text_color(palette(cx).label_secondary)
        .child(text.into())
}

/// 对话框标题（Semibold 13pt）。
pub fn heading(text: impl Into<SharedString>) -> impl IntoElement {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}
