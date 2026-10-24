//! Form view (F-DATA-002): one record at a time, field by field, on the loaded page.

use super::*;

impl TableViewer {
    pub(super) fn flush_form(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.form_for else {
            return;
        };
        let values: Vec<String> = self
            .form_inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        for (c, text) in values.into_iter().enumerate() {
            self.write_form_cell(row, c, text, cx);
        }
    }

    pub(super) fn write_form_cell(
        &mut self,
        row: usize,
        c: usize,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let raw = self.raw_data;
        let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                let old = m.raw_text(row, c, raw, dialect);
                if text != old {
                    let cell = Some(text.into());
                    if raw {
                        m.set_raw(row, c, cell);
                    } else {
                        m.set_value(row, c, cell);
                    }
                }
            });
        });
    }

    pub(super) fn sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.form {
            return;
        }
        let (row, n) = {
            let model = self.grid.read(cx).model(cx);
            (model.head().0, model.columns.len())
        };
        if self.form_for == Some(row) && self.form_inputs.len() == n {
            return;
        }
        self.form_inputs.clear();
        for c in 0..n {
            let raw = self.raw_data;
            let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
            let value = self.grid.read(cx).model(cx).raw_text(row, c, raw, dialect);
            let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
            let row_at = row;
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let text = input.read(cx).value().to_string();
                        this.write_form_cell(row_at, c, text, cx);
                    }
                },
            )
            .detach();
            self.form_inputs.push(input);
        }
        self.form_for = Some(row);
    }

    pub(super) fn form_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        // Flushing while locked would silently drop the draft before changing records.
        if self.busy {
            return;
        }
        self.flush_form(cx);
        let (row, n) = {
            let model = self.grid.read(cx).model(cx);
            (model.head().0, model.row_count())
        };
        if n == 0 {
            return;
        }
        let next = (row as isize + delta).clamp(0, n as isize - 1) as usize;
        if next == row {
            return;
        }
        self.grid.update(cx, |g, cx| g.move_to_row(next, cx));
        self.form_for = None;
    }

    pub(super) fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<_> = self
            .grid
            .read(cx)
            .model(cx)
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect();
        v_flex()
            .id("form-view")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap(tokens::form::ROW_GAP.rems())
            .children(
                self.form_inputs
                    .iter()
                    .zip(names)
                    .map(|(input, name)| FormRow::new(name, Input::new(input).flex_1())),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("form-prev")
                            .disabled(self.busy)
                            .ghost()
                            .small()
                            .label(t!("viewer.prev_record").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.form_step(-1, cx))),
                    )
                    .child(
                        Button::new("form-next")
                            .disabled(self.busy)
                            .ghost()
                            .small()
                            .label(t!("viewer.next_record").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.form_step(1, cx))),
                    ),
            )
    }
}
