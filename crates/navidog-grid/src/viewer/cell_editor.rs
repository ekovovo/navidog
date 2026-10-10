//! Cell editor under the grid (F-DATA-011): text, hex, image, web, JSON, enum and set.

use super::*;

impl TableViewer {
    pub(super) fn focused_cell(&self, cx: &App) -> Option<(usize, usize, String, String)> {
        let model = self.grid.read(cx).model(cx);
        if model.columns.is_empty() || model.row_count() == 0 {
            return None;
        }
        let (r, c) = model.head();
        let type_name = model.columns.get(c)?.type_name.to_string();
        let text = match model.value(r, c) {
            Some(Some(s)) => s.to_string(),
            _ => String::new(),
        };
        Some((r, c, text, type_name))
    }

    pub(super) fn write_at(
        &mut self,
        row: usize,
        col: usize,
        text: String,
        cx: &mut Context<Self>,
    ) {
        self.grid.update(cx, |g, cx| {
            g.update_model(cx, |m| {
                let current = match m.value(row, col) {
                    Some(Some(s)) => Some(s.to_string()),
                    _ => None,
                };
                if (text.is_empty() && current.is_none())
                    || current.as_deref() == Some(text.as_str())
                {
                    return;
                }
                m.set_value(row, col, Some(text.into()));
            });
        });
    }

    pub(super) fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.editor else {
            return;
        };
        let Some((r, c, text, _)) = self.focused_cell(cx) else {
            return;
        };
        if self.editor_cell == Some((r, c)) {
            return;
        }
        self.editor_cell = Some((r, c));
        self.editor_note = SharedString::default();
        let shown = match kind {
            Kind::Hex => encode_hex(&text),
            Kind::Json => pretty_json(&text).unwrap_or(text.clone()),
            _ => text.clone(),
        };
        if kind == Kind::Json {
            self.json_input
                .update(cx, |s, cx| s.set_value(shown, window, cx));
        } else {
            self.editor_input
                .update(cx, |s, cx| s.set_value(shown, window, cx));
        }
        self.image = editor::sniff_image(text.as_bytes())
            .map(|format| Arc::new(Image::from_bytes(format, text.into_bytes())));
    }

    pub(super) fn open_editor(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor == Some(kind) {
            return;
        }
        self.editor = Some(kind);
        self.editor_cell = None;
        self.sync_editor(window, cx);
        cx.notify();
    }

    pub(super) fn apply_editor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((r, c, _, _)) = self.focused_cell(cx) else {
            return;
        };
        let raw = if self.editor == Some(Kind::Json) {
            self.json_input.read(cx).value().to_string()
        } else {
            self.editor_input.read(cx).value().to_string()
        };
        let text = match self.editor {
            Some(Kind::Hex) => match decode_hex(&raw) {
                Some(text) => text,
                None => {
                    self.editor_note = t!("viewer.editor_hex_invalid").into();
                    cx.notify();
                    return;
                }
            },
            Some(Kind::Text | Kind::Web | Kind::Json) => raw,
            _ => return,
        };
        self.editor_note = SharedString::default();
        self.write_at(r, c, text, cx);
        self.editor_cell = Some((r, c));
    }

    pub(super) fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.json_input.read(cx).value().to_string();
        match pretty_json(&raw) {
            Some(pretty) => {
                self.editor_note = SharedString::default();
                self.json_input
                    .update(cx, |s, cx| s.set_value(pretty, window, cx));
            }
            None => self.editor_note = t!("viewer.editor_json_invalid").into(),
        }
        cx.notify();
    }

    pub(super) fn load_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let bytes = std::fs::read(&path).ok();
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(bytes) = bytes else {
                    this.editor_note = t!("viewer.editor_image_read").into();
                    cx.notify();
                    return;
                };
                if let Some(format) = editor::sniff_image(&bytes) {
                    this.image = Some(Arc::new(Image::from_bytes(format, bytes.clone())));
                }
                match String::from_utf8(bytes) {
                    Ok(text) => {
                        if let Some((r, c, _, _)) = this.focused_cell(cx) {
                            this.write_at(r, c, text.clone(), cx);
                            this.editor_cell = Some((r, c));
                            this.editor_input
                                .update(cx, |s, cx| s.set_value(text, window, cx));
                        }
                        this.editor_note = SharedString::default();
                    }
                    Err(_) => this.editor_note = t!("viewer.editor_image_binary").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bytes = if let Some(image) = &self.image {
            image.bytes.clone()
        } else {
            self.focused_cell(cx)
                .map(|(_, _, text, _)| text.into_bytes())
                .unwrap_or_default()
        };
        let rx = cx.prompt_for_new_path(Path::new("."), Some("cell.bin"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            if std::fs::write(&path, bytes).is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.editor_note = t!("viewer.editor_image_write").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn clear_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.image = None;
        if let Some((r, c, _, _)) = self.focused_cell(cx) {
            self.write_at(r, c, String::new(), cx);
            self.editor_cell = Some((r, c));
            self.editor_input
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.editor_note = SharedString::default();
        cx.notify();
    }

    pub(super) fn pick_choice(&mut self, value: String, cx: &mut Context<Self>) {
        let Some((r, c, current, type_name)) = self.focused_cell(cx) else {
            return;
        };
        let text = match choices(&type_name) {
            Some(Choices::Many(values)) => {
                let mut picked: Vec<String> = current
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && values.iter().any(|v| v == s))
                    .map(str::to_string)
                    .collect();
                if let Some(ix) = picked.iter().position(|v| v == &value) {
                    picked.remove(ix);
                } else {
                    picked.push(value);
                }
                picked.join(",")
            }
            _ => value,
        };
        self.write_at(r, c, text, cx);
    }

    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let kind = self.editor.unwrap_or(Kind::Text);
        let kind_button = |id: &'static str, on: bool, label: String, next: Kind| {
            Button::new(id)
                .xsmall()
                .selected(on)
                .label(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor(next, window, cx);
                }))
        };
        let (_, _, text, type_name) = self
            .focused_cell(cx)
            .unwrap_or_else(|| (0, 0, String::new(), String::new()));
        // Tabs and the button row sit in this chrome; the text area takes the rest and
        // scrolls inside itself. The pane can shrink so a short window keeps the bottom bar.
        let chrome = tokens::viewer::FILTER_HEIGHT.rems() * 2.;
        v_flex()
            .id("cell-editor")
            .w_full()
            .min_w_0()
            .h(tokens::viewer::CELL_EDITOR_HEIGHT.rems() + chrome)
            .min_h(chrome)
            .flex_shrink(1.)
            .overflow_hidden()
            .border_t_1()
            .border_color(p.separator)
            .px_2()
            .py_1()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(kind_button(
                        "editor-text",
                        kind == Kind::Text,
                        t!("viewer.editor_text").to_string(),
                        Kind::Text,
                    ))
                    .child(kind_button(
                        "editor-hex",
                        kind == Kind::Hex,
                        t!("viewer.editor_hex").to_string(),
                        Kind::Hex,
                    ))
                    .child(kind_button(
                        "editor-image",
                        kind == Kind::Image,
                        t!("viewer.editor_image").to_string(),
                        Kind::Image,
                    ))
                    .child(kind_button(
                        "editor-web",
                        kind == Kind::Web,
                        t!("viewer.editor_web").to_string(),
                        Kind::Web,
                    ))
                    .child(kind_button(
                        "editor-json",
                        kind == Kind::Json,
                        t!("viewer.editor_json").to_string(),
                        Kind::Json,
                    ))
                    .child(kind_button(
                        "editor-enum",
                        kind == Kind::Enum,
                        t!("viewer.editor_enum").to_string(),
                        Kind::Enum,
                    )),
            )
            .when(kind == Kind::Web, |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(t!("viewer.editor_web_note").to_string()),
                )
            })
            .when(
                matches!(kind, Kind::Text | Kind::Hex | Kind::Web | Kind::Json),
                |pane| {
                    pane.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            // raw-px: gpui-kit's focus ring is 3px outside the border; this slot clips it square
                            .p(gpui_kit::px(3.))
                            .when(kind == Kind::Json, |slot| {
                                slot.child(Editor::new(&self.json_input).size_full())
                            })
                            .when(kind != Kind::Json, |slot| {
                                slot.child(Textarea::new(&self.editor_input).size_full())
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("editor-update")
                                    .xsmall()
                                    .label(t!("viewer.editor_update").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply_editor(window, cx)
                                    })),
                            )
                            .when(kind == Kind::Json, |row| {
                                row.child(
                                    Button::new("editor-format")
                                        .xsmall()
                                        .label(t!("viewer.editor_format").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.format_json(window, cx)
                                        })),
                                )
                            }),
                    )
                },
            )
            .when(kind == Kind::Image, |pane| {
                pane.when_some(self.image.clone(), |pane, image| {
                    pane.child(
                        img(image)
                            .max_h(tokens::viewer::CELL_EDITOR_HEIGHT.rems())
                            .max_w(tokens::form::FIELD_WIDTH.rems()),
                    )
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("editor-load")
                                .xsmall()
                                .label(t!("viewer.editor_load").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.load_image(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("editor-save")
                                .xsmall()
                                .label(t!("viewer.editor_save").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_image(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("editor-clear")
                                .xsmall()
                                .label(t!("viewer.editor_clear").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.clear_image(window, cx)),
                                ),
                        ),
                )
            })
            .when(kind == Kind::Enum, |pane| match choices(&type_name) {
                Some(Choices::One(values)) => {
                    pane.child(self.choice_row(&values, &text, false, cx))
                }
                Some(Choices::Many(values)) => {
                    pane.child(self.choice_row(&values, &text, true, cx))
                }
                None => pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(t!("viewer.editor_enum_missing").to_string()),
                ),
            })
            .when(!self.editor_note.is_empty(), |pane| {
                pane.child(
                    div()
                        .text_xs()
                        .text_color(p.label_secondary)
                        .child(self.editor_note.clone()),
                )
            })
    }

    pub(super) fn choice_row(
        &self,
        values: &[String],
        current: &str,
        many: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let picked: Vec<&str> = if many {
            current
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![current]
        };
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(values.iter().enumerate().map(|(ix, value)| {
                let on = picked.iter().any(|p| *p == value);
                let value = value.clone();
                Checkbox::new(ElementId::Name(format!("editor-choice-{ix}").into()))
                    .xsmall()
                    .label(value.clone())
                    .checked(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_choice(value.clone(), cx);
                    }))
            }))
    }
}
