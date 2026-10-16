//! Shared chrome of the table and query find bars: mode button, previous / next / done, and the
//! optional replace row. Each caller keeps its own menu and search.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};

use crate::assets::lucide;
use crate::tokens::{self, palette};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The second row of a find bar: replacement field, replace, replace all.
pub struct ReplaceRow {
    input: AnyElement,
    one_id: ElementId,
    all_id: ElementId,
    one_label: SharedString,
    all_label: SharedString,
    disabled: bool,
    on_one: Option<ClickHandler>,
    on_all: Option<ClickHandler>,
}

impl ReplaceRow {
    pub fn new(
        one_id: impl Into<ElementId>,
        all_id: impl Into<ElementId>,
        input: impl IntoElement,
        one_label: impl Into<SharedString>,
        all_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            input: input.into_any_element(),
            one_id: one_id.into(),
            all_id: all_id.into(),
            one_label: one_label.into(),
            all_label: all_label.into(),
            disabled: false,
            on_one: None,
            on_all: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_one(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_one = Some(Box::new(f));
        self
    }

    pub fn on_all(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_all = Some(Box::new(f));
        self
    }
}

/// Mode dropdown plus the find field and its buttons. `invalid` paints the mode button in the
/// danger color (a regular expression that does not compile).
#[derive(IntoElement)]
pub struct FindBar {
    mode_label: SharedString,
    invalid: bool,
    on_mode: Option<ClickHandler>,
    find: AnyElement,
    nav_disabled: bool,
    done_label: SharedString,
    on_prev: Option<ClickHandler>,
    on_next: Option<ClickHandler>,
    on_done: Option<ClickHandler>,
    replace: Option<ReplaceRow>,
}

impl FindBar {
    pub fn new(
        mode_label: impl Into<SharedString>,
        find: impl IntoElement,
        done_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            mode_label: mode_label.into(),
            invalid: false,
            on_mode: None,
            find: find.into_any_element(),
            nav_disabled: false,
            done_label: done_label.into(),
            on_prev: None,
            on_next: None,
            on_done: None,
            replace: None,
        }
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// Disables previous and next. Done stays enabled so the bar can always be closed.
    pub fn nav_disabled(mut self, disabled: bool) -> Self {
        self.nav_disabled = disabled;
        self
    }

    pub fn on_mode(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_mode = Some(Box::new(f));
        self
    }

    pub fn on_prev(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_prev = Some(Box::new(f));
        self
    }

    pub fn on_next(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_next = Some(Box::new(f));
        self
    }

    pub fn on_done(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_done = Some(Box::new(f));
        self
    }

    pub fn replace(mut self, row: Option<ReplaceRow>) -> Self {
        self.replace = row;
        self
    }
}

fn click(
    button: gpui_kit::component::button::Button,
    handler: Option<ClickHandler>,
) -> gpui_kit::component::button::Button {
    button.when_some(handler, |button, handler| {
        button.on_click(move |e, window, cx| handler(e, window, cx))
    })
}

impl RenderOnce for FindBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let mode_border = if self.invalid { p.danger } else { p.separator };
        let nav_disabled = self.nav_disabled;
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
                    .border_color(mode_border)
                    .bg(p.fill_tertiary)
                    .cursor_pointer()
                    .child(self.mode_label)
                    .child(
                        lucide("chevron-down")
                            .size(tokens::navigator::ROW_ICON.rems())
                            .text_color(p.label_secondary),
                    )
                    .when_some(self.on_mode, |this, on_mode| {
                        this.on_click(move |e, window, cx| on_mode(e, window, cx))
                    }),
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
                            .child(self.find)
                            .child(click(
                                Button::new("find-prev")
                                    .small()
                                    .ghost()
                                    .icon(lucide("chevron-left"))
                                    .disabled(nav_disabled),
                                self.on_prev,
                            ))
                            .child(click(
                                Button::new("find-next")
                                    .small()
                                    .ghost()
                                    .icon(lucide("chevron-right"))
                                    .disabled(nav_disabled),
                                self.on_next,
                            ))
                            .child(click(
                                Button::new("find-done").small().label(self.done_label),
                                self.on_done,
                            )),
                    )
                    .when_some(self.replace, |col, row| {
                        col.child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(row.input)
                                .child(click(
                                    Button::new(row.one_id)
                                        .small()
                                        .label(row.one_label)
                                        .disabled(row.disabled),
                                    row.on_one,
                                ))
                                .child(click(
                                    Button::new(row.all_id)
                                        .small()
                                        .label(row.all_label)
                                        .disabled(row.disabled),
                                    row.on_all,
                                )),
                        )
                    }),
            )
    }
}
