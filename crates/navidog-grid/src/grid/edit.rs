//! In-place cell editing and the row height dialog.

use super::*;

impl ResultGrid {
    pub(super) fn set_row_height(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.row_height.0.to_string();
        let grid = cx.entity().downgrade();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let view = cx.new(|_| RowHeight {
            input: input.clone(),
            grid,
        });
        window.open_dialog(cx, move |dialog, window, _| {
            confirm_with(dialog, &view, RowHeight::apply)
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .close_button(false)
                .child(view.clone())
        });
        // `open_dialog` focuses the dialog itself, so a focus call before that does not stick
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx));
        });
    }

    pub fn start_edit(&mut self, pos: Pos, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        let m = self.model(cx);
        if m.locked || m.columns.get(pos.1).is_some_and(|col| col.read_only) {
            return;
        }
        let kind = self
            .model(cx)
            .columns
            .get(pos.1)
            .and_then(|col| crate::date::temporal_kind(&col.type_name));
        let (raw, dialect) = {
            let table = self.table.read(cx);
            let d = table.delegate();
            (d.raw, d.dialect)
        };
        let text = self.model(cx).raw_text(pos.0, pos.1, raw, dialect);
        if let Some(kind) = kind {
            crate::date::open(pos, kind, &text, cx.entity().downgrade(), window, cx);
            return;
        }
        let numeric = self
            .model(cx)
            .columns
            .get(pos.1)
            .is_some_and(|col| type_kind(&col.type_name) == TypeKind::Number);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        if numeric {
            input.update(cx, |s, cx| s.set_text_align(TextAlign::Right, cx));
        }
        self.edit_sub = Some(cx.subscribe_in(
            &input,
            window,
            |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { .. } => this.commit_edit(window, cx),
                InputEvent::Blur => this.commit_edit(window, cx),
                _ => {}
            },
        ));
        input.update(cx, |s, cx| s.focus(window, cx));
        self.table.update(cx, |t, cx| {
            t.delegate_mut().editing = Some((pos, input));
            cx.notify();
        });
    }

    /// Ends in-place editing and writes back to the model; an unchanged value is not a change (a NULL
    /// that is still an empty string after editing stays NULL).
    pub(crate) fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(((r, c), input)) = self
            .table
            .update(cx, |t, _| t.delegate_mut().editing.take())
        else {
            return;
        };
        self.edit_sub = None;
        let text = input.read(cx).value().to_string();
        let (raw, dialect) = {
            let table = self.table.read(cx);
            let d = table.delegate();
            (d.raw, d.dialect)
        };
        let old = self.model(cx).raw_text(r, c, raw, dialect);
        if text != old {
            self.update_model(cx, |m| {
                let cell = Some(text.into());
                if raw {
                    m.set_raw(r, c, cell);
                } else {
                    m.set_value(r, c, cell);
                }
            });
        }
        self.focus_table(window, cx);
    }

    pub(super) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .table
            .update(cx, |t, _| t.delegate_mut().editing.take())
            .is_some()
        {
            self.edit_sub = None;
            self.focus_table(window, cx);
        } else {
            cx.propagate();
        }
    }

    pub(super) fn focus_table(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.table.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.table.update(cx, |_, cx| cx.notify());
    }
}

/// Row height dialog (F-DATA-009). The value is in points and applies to this grid only.
struct RowHeight {
    input: Entity<InputState>,
    grid: WeakEntity<ResultGrid>,
}

/// Accepted row heights in points. Below this the text does not fit; a tiny value would also lay
/// out thousands of rows per frame.
const ROW_HEIGHT_PT: std::ops::RangeInclusive<f32> = 8.0..=200.0;

impl RowHeight {
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().trim().parse::<f32>().ok();
        if let Some(n) = value.filter(|n| n.is_finite()) {
            let n = n.clamp(*ROW_HEIGHT_PT.start(), *ROW_HEIGHT_PT.end());
            let _ = self.grid.update(cx, |g, cx| {
                g.row_height = tokens::Pt(n);
                cx.notify();
            });
        }
        window.close_dialog(cx);
    }
}

impl Render for RowHeight {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(tokens::form::ROW_GAP.rems())
            .child(t!("grid.row_height").to_string())
            .child(Input::new(&self.input).w(tokens::form::FIELD_WIDTH_SHORT.rems()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("row-height-cancel")
                            .label(t!("grid.cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("row-height-ok")
                            .primary()
                            .label(t!("grid.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}
