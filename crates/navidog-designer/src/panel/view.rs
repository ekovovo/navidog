use super::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use structured::{ENTRY_KEYS, PROPERTY_IDS};

impl TableDesigner {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy || !self.loaded;
        let fields = self.page == Page::Fields;
        let selected = if fields {
            self.selected.is_some()
        } else {
            matches!(self.page, Page::Indexes | Page::ForeignKeys | Page::Checks)
                && self.entry_selected.is_some()
        };
        let preview = self.page == Page::Preview;
        h_flex()
            .min_h(tokens::object_toolbar::HEIGHT.rems())
            .flex_wrap()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(palette(cx).separator)
            .when(preview, |bar| {
                bar.child(
                    Button::new("designer-execute")
                        .small()
                        .icon(lucide("play"))
                        .label(t!("designer.execute").to_string())
                        .disabled(
                            disabled
                                || self.needs_reload
                                || self.preview.as_ref().is_none_or(Vec::is_empty),
                        )
                        .on_click(cx.listener(|this, _, w, cx| this.request(true, w, cx))),
                )
            })
            .when(!preview, |bar| {
                bar.child(
                    Button::new("designer-save")
                        .ghost()
                        .small()
                        .icon(lucide("save"))
                        .label(t!("designer.save").to_string())
                        .disabled(disabled || self.needs_reload)
                        .on_click(cx.listener(|this, _, w, cx| this.show_preview(w, cx))),
                )
            })
            .child(
                Button::new("designer-add")
                    .ghost()
                    .small()
                    .icon(lucide("plus"))
                    .label(
                        t!(if fields {
                            "designer.add"
                        } else {
                            "designer.add_object"
                        })
                        .to_string(),
                    )
                    .disabled(
                        disabled
                            || !matches!(
                                self.page,
                                Page::Fields | Page::Indexes | Page::ForeignKeys | Page::Checks
                            ),
                    )
                    .on_click(cx.listener(|this, _, w, cx| this.add(false, w, cx))),
            )
            .child(
                Button::new("designer-delete")
                    .ghost()
                    .small()
                    .icon(lucide("minus"))
                    .label(
                        t!(if fields {
                            "designer.delete"
                        } else {
                            "designer.delete_object"
                        })
                        .to_string(),
                    )
                    .disabled(disabled || !selected)
                    .on_click(cx.listener(|this, _, w, cx| this.delete(w, cx))),
            )
            .when(fields, |bar| {
                bar.child(
                    Button::new("designer-duplicate")
                        .ghost()
                        .small()
                        .icon(lucide("copy"))
                        .tooltip(t!("designer.duplicate").to_string())
                        .accessibility_label(t!("designer.duplicate").to_string())
                        .disabled(disabled || !selected)
                        .on_click(cx.listener(|this, _, w, cx| this.add(true, w, cx))),
                )
                .child(
                    Button::new("designer-primary")
                        .ghost()
                        .small()
                        .icon(lucide("key-round"))
                        .label(t!("designer.primary").to_string())
                        .disabled(disabled || !selected)
                        .on_click(cx.listener(|this, _, w, cx| this.toggle_primary(w, cx))),
                )
                .child(
                    Button::new("designer-up")
                        .ghost()
                        .small()
                        .icon(lucide("arrow-up"))
                        .tooltip(t!("designer.up").to_string())
                        .accessibility_label(t!("designer.up").to_string())
                        .disabled(disabled || !selected)
                        .on_click(cx.listener(|this, _, w, cx| this.move_field(true, w, cx))),
                )
                .child(
                    Button::new("designer-down")
                        .ghost()
                        .small()
                        .icon(lucide("arrow-down"))
                        .tooltip(t!("designer.down").to_string())
                        .accessibility_label(t!("designer.down").to_string())
                        .disabled(disabled || !selected)
                        .on_click(cx.listener(|this, _, w, cx| this.move_field(false, w, cx))),
                )
            })
            .when(self.baseline.is_some() || self.needs_reload, |bar| {
                bar.child(
                    Button::new("designer-reload-toolbar")
                        .ghost()
                        .small()
                        .icon(lucide("refresh-cw"))
                        .tooltip(t!("designer.reload").to_string())
                        .accessibility_label(t!("designer.reload").to_string())
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, w, cx| this.reload(w, cx))),
                )
            })
    }
    fn property_row(
        &self,
        ix: usize,
        key: &str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        FormRow::new(
            t!(format!("designer.{key}")).to_string(),
            h_flex()
                .w(tokens::form::FIELD_WIDTH.rems())
                .max_w_full()
                .min_w_0()
                .child(
                    Input::new(&self.properties[ix])
                        .id(PROPERTY_IDS[ix])
                        .small()
                        .disabled(disabled)
                        .flex_1(),
                )
                .when(ix == 8 || ix == 10, |row| {
                    row.child(choices(
                        cx.entity().downgrade(),
                        if ix == 10 { 90 } else { ix },
                        if ix == 10 {
                            vec!["".into(), "ALWAYS".into(), "BY DEFAULT".into()]
                        } else if self.dialect == Dialect::Postgres {
                            vec!["STORED".into()]
                        } else {
                            vec!["VIRTUAL".into(), "STORED".into()]
                        },
                        disabled,
                    ))
                }),
        )
    }
    fn render_field_properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self
            .draft
            .columns
            .iter()
            .find(|c| Some(c.id) == self.selected);
        let p = c
            .map(|c| column_properties(&c.definition))
            .unwrap_or_default();
        let disabled = self.busy || !self.loaded || c.is_none();
        let numeric = navidog_util::sql_type::type_kind(&p.data_type)
            == navidog_util::sql_type::TypeKind::Number;
        let text = navidog_util::sql_type::type_kind(&p.data_type)
            == navidog_util::sql_type::TypeKind::Text;
        v_flex()
            .id("designer-properties")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .child(self.property_row(
                4,
                "default_value",
                disabled || p.generated || !p.identity.is_empty(),
                cx,
            ))
            .child(self.property_row(5, "collation", disabled, cx))
            .when(text && self.dialect == Dialect::MySql, |v| {
                v.child(self.property_row(6, "charset", disabled, cx))
            })
            .when(numeric && self.dialect != Dialect::Postgres, |v| {
                v.child(FormRow::bare(
                    Checkbox::new("designer-auto-increment")
                        .label(t!("designer.auto_increment").to_string())
                        .checked(p.auto_increment)
                        .disabled(disabled)
                        .on_click(cx.listener(|this, value, w, cx| {
                            this.toggle_attribute(ColumnProperty::AutoIncrement, *value, w, cx)
                        })),
                ))
            })
            .when(numeric && self.dialect == Dialect::MySql, |v| {
                v.child(FormRow::bare(
                    Checkbox::new("designer-unsigned")
                        .label(t!("designer.unsigned").to_string())
                        .checked(p.unsigned)
                        .disabled(disabled || p.zerofill)
                        .on_click(cx.listener(|this, value, w, cx| {
                            this.toggle_attribute(ColumnProperty::Unsigned, *value, w, cx)
                        })),
                ))
            })
            .when(self.dialect == Dialect::MySql && numeric, |v| {
                v.child(FormRow::bare(
                    Checkbox::new("designer-zerofill")
                        .label(t!("designer.zerofill").to_string())
                        .checked(p.zerofill)
                        .disabled(disabled)
                        .on_click(cx.listener(|this, value, w, cx| {
                            this.toggle_attribute(ColumnProperty::Zerofill, *value, w, cx)
                        })),
                ))
            })
            .when(
                self.dialect == Dialect::MySql
                    && ["TIMESTAMP", "DATETIME"]
                        .iter()
                        .any(|ty| p.data_type.to_ascii_uppercase().contains(ty)),
                |v| v.child(self.property_row(9, "on_update_expression", disabled, cx)),
            )
            .when(self.dialect == Dialect::Postgres && numeric, |v| {
                v.child(self.property_row(10, "identity_mode", disabled || p.generated, cx))
            })
            .when(p.generated, |v| {
                v.child(self.property_row(7, "expression", disabled, cx))
                    .child(self.property_row(8, "generated_storage", disabled, cx))
            })
            .when(p.generated, |v| {
                v.child(
                    gpui_kit::div()
                        .text_color(palette(cx).label_secondary)
                        .child(t!("designer.generated_help").to_string()),
                )
            })
    }
    fn render_entry_properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entry = self
            .entry_selected
            .and_then(|i| self.entries().get(i).and_then(|e| e.3.clone()));
        let disabled = self.busy || !self.loaded || entry.is_none();
        let indexes = match self.page {
            Page::Indexes => vec![0, 1, 2, 3, 9],
            Page::ForeignKeys => vec![0, 1, 4, 5, 6, 7, 8],
            _ => vec![0, 9],
        };
        v_flex()
            .id("designer-entry-properties")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .children(
                indexes
                    .into_iter()
                    .filter(|ix| {
                        !(self.page == Page::Indexes
                            && [2, 3, 9].contains(ix)
                            && entry.as_ref().is_some_and(|e| e.kind != EntryKind::Index))
                            && !(self.page == Page::ForeignKeys
                                && *ix == 4
                                && self.dialect == Dialect::Sqlite)
                    })
                    .map(|ix| {
                        let control_disabled = disabled
                            || (ix == 3 && self.dialect != Dialect::MySql)
                            || (ix == 2 && self.dialect == Dialect::Sqlite)
                            || (ix == 9
                                && self.page == Page::Indexes
                                && self.dialect == Dialect::MySql);
                        FormRow::new(
                            t!(format!("designer.{}", ENTRY_KEYS[ix])).to_string(),
                            h_flex()
                                .w(tokens::form::FIELD_WIDTH.rems())
                                .max_w_full()
                                .min_w_0()
                                .child(
                                    Input::new(&self.entry_inputs[ix])
                                        .id(SharedString::from(format!(
                                            "designer-entry-{}",
                                            ENTRY_KEYS[ix]
                                        )))
                                        .small()
                                        .disabled(control_disabled)
                                        .flex_1(),
                                )
                                .when(
                                    [1, 2, 7, 8].contains(&ix)
                                        || (self.page == Page::ForeignKeys
                                            && [4, 5, 6].contains(&ix)),
                                    |row| {
                                        let values = match ix {
                                            4..=6 => self.reference_values(ix, cx),
                                            1 => self
                                                .draft
                                                .columns
                                                .iter()
                                                .map(|c| c.name.clone())
                                                .collect(),
                                            2 => (if self.dialect == Dialect::MySql {
                                                vec!["BTREE", "HASH"]
                                            } else {
                                                vec!["BTREE", "HASH", "GIN", "GIST", "BRIN"]
                                            })
                                            .into_iter()
                                            .map(String::from)
                                            .collect(),
                                            _ => vec![
                                                "NO ACTION",
                                                "RESTRICT",
                                                "CASCADE",
                                                "SET NULL",
                                                "SET DEFAULT",
                                            ]
                                            .into_iter()
                                            .map(String::from)
                                            .collect(),
                                        };
                                        row.child(choices(
                                            cx.entity().downgrade(),
                                            ix + 10,
                                            values.clone(),
                                            control_disabled
                                                || (self.page == Page::ForeignKeys
                                                    && [4, 5, 6].contains(&ix)
                                                    && (self.references.loading
                                                        || values.is_empty())),
                                        ))
                                    },
                                ),
                        )
                    }),
            )
            .when(
                self.page == Page::Indexes
                    && entry.as_ref().is_some_and(|e| e.kind == EntryKind::Index),
                |v| {
                    v.child(FormRow::bare(
                        Checkbox::new("designer-index-unique")
                            .label(t!("designer.unique").to_string())
                            .checked(entry.as_ref().is_some_and(|e| e.unique))
                            .disabled(disabled)
                            .on_click(cx.listener(|this, value, w, cx| {
                                this.write_entry(Some(*value), w, cx)
                            })),
                    ))
                },
            )
            .when(self.page == Page::ForeignKeys, |v| {
                v.child(
                    Button::new("designer-reference-refresh")
                        .ghost()
                        .small()
                        .label(t!("designer.refresh_references").to_string())
                        .disabled(disabled || self.references.loading)
                        .on_click(cx.listener(|this, _, w, cx| this.load_references(w, cx))),
                )
                .when(self.references.loading, |v| {
                    v.child(t!("designer.loading_references").to_string())
                })
                .when_some(self.references.error.clone(), |v, error| {
                    v.child(gpui_kit::div().text_color(palette(cx).danger).child(error))
                })
                .child(
                    gpui_kit::div()
                        .text_color(palette(cx).label_secondary)
                        .child(t!("designer.actions_help").to_string()),
                )
            })
    }
    fn render_grid_page(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        let owner = cx.entity();
        v_resizable(if self.page == Page::Fields {
            "designer-fields-split"
        } else {
            "designer-entry-split"
        })
        .on_resize(move |state, _, cx| {
            if let Some(size) = state.read(cx).sizes().get(1).copied() {
                owner.update(cx, |this, _| this.property_height = Some(size));
            }
        })
        .child(
            resizable_panel()
                .size_range(tokens::designer::GRID_MIN_HEIGHT.to_pixels(rem)..gpui_kit::Pixels::MAX)
                .child(
                    v_flex().size_full().min_w_0().child(
                        DataTable::new(&self.table)
                            .with_size(gpui_kit::component::Size::Size(
                                tokens::designer::ROW_HEIGHT.to_pixels(rem),
                            ))
                            .header_row_height(tokens::designer::HEADER_HEIGHT.to_pixels(rem))
                            .bordered(false)
                            .stripe(false),
                    ),
                ),
        )
        .child(
            resizable_panel()
                .size(
                    self.property_height
                        .unwrap_or_else(|| tokens::designer::PROPERTY_HEIGHT.to_pixels(rem)),
                )
                .size_range(
                    tokens::designer::PROPERTY_MIN_HEIGHT.to_pixels(rem)..gpui_kit::Pixels::MAX,
                )
                .child(if self.page == Page::Fields {
                    self.render_field_properties(cx).into_any_element()
                } else {
                    self.render_entry_properties(cx).into_any_element()
                }),
        )
    }
}
impl Render for TableDesigner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let disabled = self.busy || !self.loaded;
        v_flex()
            .id("table-designer")
            .key_context("TableDesigner")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .bg(p.control_bg)
            .text_color(p.label)
            .on_action(cx.listener(|this, _: &Save, w, cx| this.show_preview(w, cx)))
            .on_action(cx.listener(|this, _: &AddField, w, cx| this.add(false, w, cx)))
            .on_action(cx.listener(|this, _: &DuplicateField, w, cx| this.add(true, w, cx)))
            .on_action(cx.listener(|this, _: &DeleteField, w, cx| this.delete(w, cx)))
            .on_action(cx.listener(|this, _: &MoveFieldUp, w, cx| this.move_field(true, w, cx)))
            .on_action(cx.listener(|this, _: &MoveFieldDown, w, cx| this.move_field(false, w, cx)))
            .on_action(
                cx.listener(|this, action: &ChooseValue, w, cx| this.choose_value(action, w, cx)),
            )
            .on_action(cx.listener(|this, _: &TogglePrimary, w, cx| this.toggle_primary(w, cx)))
            .child(self.render_toolbar(cx))
            .when(self.baseline.is_none(), |v| {
                v.child(
                    h_flex().px_2().py_1().child(
                        FormRow::new(
                            t!("designer.table_name").to_string(),
                            Input::new(&self.table_name)
                                .id("designer-table-name")
                                .small()
                                .disabled(disabled),
                        )
                        .hug_label(),
                    ),
                )
            })
            .child(
                h_flex()
                    .px_2()
                    .gap_1()
                    .flex_wrap()
                    .border_b_1()
                    .border_color(p.separator)
                    .children(
                        [
                            (Page::Fields, "fields"),
                            (Page::Indexes, "indexes"),
                            (Page::ForeignKeys, "foreign_keys"),
                            (Page::Checks, "checks"),
                            (Page::Options, "options"),
                            (Page::Preview, "preview"),
                            (Page::Advanced, "advanced"),
                        ]
                        .into_iter()
                        .map(|(page, key)| {
                            Button::new(SharedString::from(format!("designer-page-{key}")))
                                .ghost()
                                .small()
                                .label(t!(format!("designer.{key}")).to_string())
                                .selected(self.page == page)
                                .on_click(
                                    cx.listener(move |this, _, w, cx| {
                                        this.switch_page(page, w, cx)
                                    }),
                                )
                        }),
                    ),
            )
            .when(self.busy, |v| {
                v.child(
                    gpui_kit::div()
                        .px_2()
                        .child(t!("designer.loading").to_string()),
                )
            })
            .when_some(self.error.clone(), |v, error| {
                v.child(gpui_kit::div().px_2().text_color(p.danger).child(error))
            })
            .when(
                self.draft
                    .constraints
                    .iter()
                    .chain(&self.draft.indexes)
                    .any(|s| parse_entry(s).is_none()),
                |v| {
                    v.child(
                        h_flex()
                            .px_2()
                            .gap_2()
                            .child(
                                gpui_kit::div()
                                    .text_color(p.label_secondary)
                                    .child(t!("designer.preserved_help").to_string()),
                            )
                            .child(
                                Button::new("designer-open-advanced")
                                    .ghost()
                                    .small()
                                    .label(t!("designer.advanced").to_string())
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.switch_page(Page::Advanced, w, cx)
                                    })),
                            ),
                    )
                },
            )
            .when(self.needs_reload, |v| {
                v.child(
                    gpui_kit::div()
                        .px_2()
                        .text_color(p.danger)
                        .child(t!("designer.reload_required").to_string()),
                )
            })
            .when(self.needs_reload || !self.loaded, |v| {
                v.child(
                    Button::new("designer-reload")
                        .label(t!("designer.reload").to_string())
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, w, cx| this.load(w, cx))),
                )
            })
            .child(
                gpui_kit::div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(match self.page {
                        Page::Fields | Page::Indexes | Page::ForeignKeys | Page::Checks => {
                            self.render_grid_page(window, cx).into_any_element()
                        }
                        Page::Preview => Editor::new(&self.sql).size_full().into_any_element(),
                        Page::Options => self.render_table_options(cx).into_any_element(),
                        Page::Advanced => v_flex()
                            .size_full()
                            .min_h_0()
                            .child(
                                gpui_kit::div()
                                    .p_2()
                                    .text_color(p.label_secondary)
                                    .child(t!("designer.advanced_help").to_string()),
                            )
                            .child(
                                Input::new(&self.definition)
                                    .id("designer-field-definition")
                                    .disabled(disabled || self.selected.is_none()),
                            )
                            .child(
                                gpui_kit::div()
                                    .px_2()
                                    .child(t!("designer.constraints").to_string()),
                            )
                            .child(
                                Editor::new(&self.constraints)
                                    .disabled(disabled)
                                    .flex_1()
                                    .min_h_0(),
                            )
                            .child(
                                gpui_kit::div()
                                    .px_2()
                                    .child(t!("designer.indexes").to_string()),
                            )
                            .child(
                                Editor::new(&self.indexes)
                                    .disabled(disabled)
                                    .flex_1()
                                    .min_h_0(),
                            )
                            .child(
                                gpui_kit::div()
                                    .px_2()
                                    .child(t!("designer.options").to_string()),
                            )
                            .child(
                                Editor::new(&self.options)
                                    .disabled(disabled)
                                    .flex_1()
                                    .min_h_0(),
                            )
                            .into_any_element(),
                    }),
            )
    }
}

pub(super) fn choices(
    owner: WeakEntity<TableDesigner>,
    target: usize,
    values: Vec<String>,
    disabled: bool,
) -> Button {
    Button::new(SharedString::from(format!("designer-choices-{target}")))
        .ghost()
        .small()
        .icon(lucide("chevron-down"))
        .tooltip(t!("designer.choose").to_string())
        .accessibility_label(t!("designer.choose").to_string())
        .disabled(disabled)
        .on_click(move |event, window, cx| {
            let Ok(focus) = owner.read_with(cx, |this, _| this.focus.clone()) else {
                return;
            };
            window.focus(&focus, cx);
            let menu = values.iter().fold(
                gpui_kit::component::native_menu::NativeMenu::new(),
                |menu, value| {
                    menu.menu(
                        value.clone(),
                        Box::new(ChooseValue {
                            target,
                            value: value.clone(),
                        }),
                    )
                },
            );
            menu.show(event.position(), window, cx);
        })
}
