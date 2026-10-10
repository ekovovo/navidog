//! "Input Parameters" dialog (31-query-results.md §3): one row per `[$name]`, with a raw mode
//! that skips the quotes.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use navidog_ui::skin::{confirm_with, heading, truncated};
use navidog_ui::tokens;
use rust_i18n::t;

type Done = Rc<dyn Fn(Vec<(String, String, bool)>, &mut Window, &mut App)>;

struct Row {
    name: String,
    input: Entity<InputState>,
    raw: bool,
}

struct Params {
    rows: Vec<Row>,
    done: Done,
}

/// Asks for the values; `done` gets (name, value, raw) for each parameter. Cancel runs nothing.
pub(crate) fn ask(
    prefilled: Vec<(String, String, bool)>,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Vec<(String, String, bool)>, &mut Window, &mut App) + 'static,
) {
    let rows = prefilled
        .into_iter()
        .map(|(name, value, raw)| Row {
            input: cx.new(|cx| InputState::new(window, cx).default_value(value)),
            name,
            raw,
        })
        .collect();
    let view = cx.new(|_| Params {
        rows,
        done: Rc::new(done),
    });
    if let Some(first) = view.read(cx).rows.first().map(|r| r.input.clone()) {
        first.update(cx, |s, cx| s.focus(window, cx));
    }
    window.open_dialog(cx, move |dialog, window, _| {
        confirm_with(
            dialog
                .w(tokens::query::PARAMS_WIDTH.to_pixels(window.rem_size()))
                .close_button(false),
            &view,
            Params::ok,
        )
        .child(view.clone())
    });
}

impl Params {
    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self
            .rows
            .iter()
            .map(|r| (r.name.clone(), r.input.read(cx).value().to_string(), r.raw))
            .collect();
        window.close_dialog(cx);
        (self.done)(values, window, cx);
    }
}

impl Render for Params {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = tokens::query::PARAMS_HEIGHT.to_pixels(window.rem_size());
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .max_h(height)
            .child(heading(t!("query.input_params").to_string()))
            .child(
                v_flex()
                    .id("params-rows")
                    .gap(tokens::form::ROW_GAP.rems())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(self.rows.iter().enumerate().map(|(ix, row)| {
                        h_flex()
                            .gap_2()
                            .child(
                                truncated(("param-name", ix), row.name.clone())
                                    .w(tokens::form::FIELD_WIDTH_SHORT.rems())
                                    .flex_none(),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&row.input).small()),
                            )
                            .child(
                                Checkbox::new(("param-raw", ix))
                                    .label(t!("query.raw_mode").to_string())
                                    .checked(row.raw)
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        if let Some(r) = this.rows.get_mut(ix) {
                                            r.raw = *checked;
                                        }
                                        cx.notify();
                                    })),
                            )
                    })),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("params-cancel")
                            .label(t!("query.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("params-ok")
                            .primary()
                            .label(t!("query.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.ok(window, cx))),
                    ),
            )
    }
}
