//! Find and replace on the loaded page (F-DATA-007).

use super::*;

impl TableViewer {
    pub(super) fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = true;
        self.replace_mode = replace;
        self.find_input.update(cx, |s, cx| s.focus(window, cx));
        self.sync_highlight(cx);
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = false;
        self.sync_highlight(cx);
        let handle = self.grid.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(super) fn sync_highlight(&mut self, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        let on = self.show_find && self.find_highlight && !self.find_columns && !query.is_empty();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.set_find(on.then(|| (query.into(), case)), cx);
        });
    }

    pub(super) fn find_step(&mut self, after: bool, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        if query.is_empty() {
            self.sync_highlight(cx);
            return;
        }
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        if self.find_columns {
            let hit = self
                .grid
                .read(cx)
                .model(cx)
                .find_column_name(&query, head.1, after, case);
            if let Some(c) = hit {
                self.grid.update(cx, |g, cx| {
                    g.update_model(cx, |m| m.select_column(c, false))
                });
            }
        } else if let Some(pos) = self
            .grid
            .read(cx)
            .model(cx)
            .find_cell(&query, head, after, case)
        {
            self.grid.update(cx, |g, cx| g.go_to(pos, cx));
        }
        self.sync_highlight(cx);
    }

    pub(super) fn replace_current(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        let head = self.grid.read(cx).model(cx).head();
        let replaced = self.grid.update(cx, |g, cx| {
            let mut ok = false;
            g.update_model(cx, |m| {
                ok = m.replace_cell(head, &query, &replacement, case)
            });
            ok
        });
        self.find_step(replaced, cx);
    }

    pub(super) fn replace_every(&mut self, cx: &mut Context<Self>) {
        if self.find_columns {
            return;
        }
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        let case = self.find_case;
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                m.replace_all(&query, &replacement, case);
            });
        });
        self.sync_highlight(cx);
    }

    pub(super) fn render_find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let toggle = |id: &'static str, on: bool, label: String| {
            Button::new(id).xsmall().ghost().selected(on).label(label)
        };
        h_flex()
            .h(tokens::viewer::FILTER_HEIGHT.rems())
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(p.separator)
            .child(
                Button::new("find-close")
                    .xsmall()
                    .ghost()
                    .icon(lucide("x"))
                    .tooltip(t!("viewer.close_find").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
            .child({
                let label = if self.find_columns {
                    t!("viewer.find_columns").to_string()
                } else {
                    t!("viewer.find_data").to_string()
                };
                toggle("find-columns", self.find_columns, label).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.find_columns = !this.find_columns;
                        if this.find_columns {
                            this.replace_mode = false;
                        }
                        this.find_step(false, cx);
                    },
                ))
            })
            .child(Input::new(&self.find_input).xsmall().flex_1())
            .child(
                Button::new("find-next")
                    .xsmall()
                    .ghost()
                    .label(t!("viewer.find_next").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))),
            )
            .child(
                toggle(
                    "find-replace",
                    self.replace_mode,
                    t!("viewer.replace").to_string(),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.replace_mode = !this.replace_mode;
                    if this.replace_mode {
                        this.find_columns = false;
                        this.replace_input.update(cx, |s, cx| s.focus(window, cx));
                    }
                    cx.notify();
                })),
            )
            .when(self.replace_mode && !self.find_columns, |bar| {
                bar.child(Input::new(&self.replace_input).xsmall().flex_1())
                    .child(
                        Button::new("replace-one")
                            .xsmall()
                            .ghost()
                            .label(t!("viewer.replace").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.replace_current(cx))),
                    )
                    .child(
                        Button::new("replace-all")
                            .xsmall()
                            .ghost()
                            .label(t!("viewer.replace_all").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.replace_every(cx))),
                    )
            })
            .child(
                toggle(
                    "find-case",
                    self.find_case,
                    t!("viewer.find_case").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_case = !this.find_case;
                    this.sync_highlight(cx);
                })),
            )
            .child(
                toggle(
                    "find-highlight",
                    self.find_highlight,
                    t!("viewer.find_highlight").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_highlight = !this.find_highlight;
                    this.sync_highlight(cx);
                })),
            )
            .child(
                toggle(
                    "find-incremental",
                    self.find_incremental,
                    t!("viewer.find_incremental").to_string(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.find_incremental = !this.find_incremental;
                    cx.notify();
                })),
            )
    }
}
