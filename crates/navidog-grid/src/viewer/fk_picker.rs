//! Foreign key data selection (F-DATA-012): pick a value of the referenced column.

use super::*;

impl TableViewer {
    pub(super) fn open_fk_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, col, _, _)) = self.focused_cell(cx) else {
            return;
        };
        let name = self
            .grid
            .read(cx)
            .model(cx)
            .columns
            .get(col)
            .map(|column| column.name.to_string());
        let Some(name) = name else {
            return;
        };
        let Some(key) = self
            .foreign_keys
            .iter()
            .find(|key| key.column == name)
            .cloned()
        else {
            return;
        };
        let (Some(session), Some(dialect)) = (self.session.clone(), self.dialect) else {
            return;
        };
        let generation = self.session_generation;
        let viewer = cx.entity().downgrade();
        let picker = cx.new(|cx| {
            FkPicker::new(
                key,
                dialect,
                (session, generation),
                viewer,
                (row, col),
                window,
                cx,
            )
        });
        picker.update(cx, |picker, cx| picker.reload(cx));
        window.open_dialog(cx, move |dialog, window, _| {
            // Enter in the search field searches again and leaves the picker open
            confirm_with(dialog, &picker, |p, _, cx| p.reload(cx))
                .w(tokens::dialog::SMALL_WIDTH.to_pixels(window.rem_size()))
                .title(t!("grid.fk_pick").to_string())
                .child(picker.clone())
        });
    }
}

/// Referenced-column picker (F-DATA-012). One click writes the value into the cell that was focused
/// when the dialog opened.
struct FkPicker {
    key: navidog_db::ForeignKey,
    dialect: Dialect,
    session: SharedSession,
    generation: u64,
    viewer: WeakEntity<TableViewer>,
    cell: (usize, usize),
    search: Entity<InputState>,
    rows: Vec<Option<String>>,
    note: SharedString,
    all: bool,
}

impl FkPicker {
    fn new(
        key: navidog_db::ForeignKey,
        dialect: Dialect,
        (session, generation): (SharedSession, u64),
        viewer: WeakEntity<TableViewer>,
        cell: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("grid.fk_search").to_string()));
        Self {
            key,
            dialect,
            session,
            generation,
            viewer,
            cell,
            search,
            rows: Vec::new(),
            note: SharedString::default(),
            all: false,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let search = self.search.read(cx).value().to_string();
        let sql = navidog_db::referenced_values_sql(
            self.dialect,
            &self.key,
            &search,
            (!self.all).then_some(1000),
        );
        let session = self.session.clone();
        let job = run_on(session, move |rel| {
            Box::pin(async move { rel.execute(&sql).await })
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ =
                this.update(cx, |this, cx| {
                    if !this.viewer.upgrade().is_some_and(|viewer| {
                        viewer.read(cx).generation_matches(this.generation, cx)
                    }) {
                        return;
                    }
                    match result {
                        Ok(Ok(sets)) => {
                            this.rows = sets
                                .into_iter()
                                .next()
                                .map(|set| {
                                    set.rows
                                        .into_iter()
                                        .map(|row| row.into_iter().next().unwrap_or(None))
                                        .collect()
                                })
                                .unwrap_or_default();
                            this.note = if this.rows.is_empty() {
                                t!("grid.fk_empty").into()
                            } else {
                                SharedString::default()
                            };
                        }
                        Ok(Err(e)) => this.note = crate::db_text(&e).into(),
                        Err(e) => this.note = e.to_string().into(),
                    }
                    cx.notify();
                });
        })
        .detach();
    }
}

impl Render for FkPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .gap_2()
            .h(tokens::form::LIST_HEIGHT.rems())
            .child(
                h_flex()
                    .gap_1()
                    .child(Input::new(&self.search).flex_1())
                    .child(
                        Button::new("fk-all")
                            .xsmall()
                            .selected(self.all)
                            .label(if self.all {
                                t!("grid.fk_limit").to_string()
                            } else {
                                t!("grid.fk_all").to_string()
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.all = !this.all;
                                this.reload(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("fk-rows")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.rows.iter().enumerate().map(|(ix, value)| {
                        let stored = value.clone();
                        let label = stored.clone().unwrap_or_else(|| "NULL".into());
                        div()
                            .id(("fk-row", ix))
                            .h(tokens::form::ROW_HEIGHT.rems())
                            .px_1()
                            .min_w_0()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let value = stored.clone();
                                let (row, col) = this.cell;
                                let generation = this.generation;
                                let _ = this.viewer.update(cx, |viewer, cx| {
                                    if viewer.generation_matches(generation, cx) {
                                        viewer.write_cell(row, col, value.map(Into::into), cx);
                                    }
                                });
                                window.close_dialog(cx);
                            }))
                            .child(truncated(("fk-text", ix), label))
                    })),
            )
            .when(!self.note.is_empty(), |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(self.note.clone()),
                )
            })
    }
}
