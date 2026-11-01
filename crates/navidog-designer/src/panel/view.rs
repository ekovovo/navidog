use super::*;
use gpui_kit::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::relative;
use structured::{ENTRY_KEYS, PROPERTIES, PROPERTY_IDS};

/// Choice-menu id space of the identity row; see `render_property_row`.
const IDENTITY_CHOICE_IX: usize = 90;

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
                let spinning = self.reload_spin.spinning(self.reloading);
                bar.child(
                    navidog_ui::refresh_spin::refresh_button(
                        "designer-reload-toolbar",
                        spinning,
                        gpui_kit::component::Size::Small,
                    )
                    .ghost()
                    .tooltip(t!("designer.reload").to_string())
                    .accessibility_label(t!("designer.reload").to_string())
                    .disabled(self.busy && !spinning)
                    .on_click(cx.listener(|this, _, w, cx| {
                        if this.reload_spin.spinning(this.reloading) {
                            return;
                        }
                        this.reload(w, cx);
                    })),
                )
            })
    }
    fn line_control(
        &self,
        line: &complete::Line,
        id: &str,
        disabled: bool,
    ) -> gpui_kit::AnyElement {
        match line {
            complete::Line::Text(editor) => Input::new(editor)
                .id(id.to_string())
                .small()
                .disabled(disabled)
                .flex_1()
                .into_any_element(),
            complete::Line::Sql(editor) => navidog_ui::skin::single_line_sql(editor)
                .disabled(disabled)
                .flex_1()
                .min_w_0()
                .into_any_element(),
        }
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
                .child(self.line_control(&self.properties[ix], PROPERTY_IDS[ix], disabled))
                .when(
                    matches!(
                        PROPERTIES[ix],
                        ColumnProperty::GeneratedStorage | ColumnProperty::Identity
                    ),
                    |row| {
                        row.child(choices(
                            cx.entity().downgrade(),
                            // The identity menu gets its own id space so its choices cannot
                            // collide with the generated-storage menu.
                            if PROPERTIES[ix] == ColumnProperty::Identity {
                                IDENTITY_CHOICE_IX
                            } else {
                                ix
                            },
                            if PROPERTIES[ix] == ColumnProperty::Identity {
                                vec!["".into(), "ALWAYS".into(), "BY DEFAULT".into()]
                            } else if !self.dialect.features().supports_virtual_generated {
                                vec!["STORED".into()]
                            } else {
                                vec!["VIRTUAL".into(), "STORED".into()]
                            },
                            disabled,
                        ))
                    },
                ),
        )
    }
    fn render_field_properties(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self
            .draft
            .columns
            .iter()
            .find(|c| Some(c.id) == self.selected);
        let p = c
            .map(|c| column_properties(&c.definition, self.dialect.as_ref()))
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
            .when(
                text && self.dialect.features().supports_column_charset,
                |v| v.child(self.property_row(6, "charset", disabled, cx)),
            )
            .when(numeric && !self.dialect.features().supports_identity, |v| {
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
            .when(numeric && self.dialect.features().supports_unsigned, |v| {
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
            .when(self.dialect.features().supports_zerofill && numeric, |v| {
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
                self.dialect.features().supports_on_update
                    && ["TIMESTAMP", "DATETIME"]
                        .iter()
                        .any(|ty| p.data_type.to_ascii_uppercase().contains(ty)),
                |v| v.child(self.property_row(9, "on_update_expression", disabled, cx)),
            )
            .when(self.dialect.features().supports_identity && numeric, |v| {
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
                                && ENTRY_KEYS[*ix] == "reference_schema"
                                && !self.dialect.features().fk_reference_schema)
                    })
                    .map(|ix| {
                        let key = ENTRY_KEYS[ix];
                        let control_disabled = disabled
                            || (key == "comment"
                                && !self.dialect.features().supports_index_comment)
                            || (key == "method" && !self.dialect.features().supports_index_method)
                            || (key == "expression"
                                && self.page == Page::Indexes
                                && !self.dialect.features().supports_index_expression);
                        FormRow::new(
                            t!(format!("designer.{}", ENTRY_KEYS[ix])).to_string(),
                            h_flex()
                                .w(tokens::form::FIELD_WIDTH.rems())
                                .max_w_full()
                                .min_w_0()
                                .child(self.line_control(
                                    &self.entry_inputs[ix],
                                    &format!("designer-entry-{}", ENTRY_KEYS[ix]),
                                    control_disabled,
                                ))
                                .when(
                                    matches!(
                                        ENTRY_KEYS[ix],
                                        "fields" | "method" | "on_delete" | "on_update"
                                    ) || (self.page == Page::ForeignKeys
                                        && matches!(
                                            ENTRY_KEYS[ix],
                                            "reference_schema"
                                                | "reference_table"
                                                | "reference_fields"
                                        )),
                                    |row| {
                                        let values = match ENTRY_KEYS[ix] {
                                            "reference_schema" | "reference_table"
                                            | "reference_fields" => self.reference_values(ix, cx),
                                            "fields" => self
                                                .draft
                                                .columns
                                                .iter()
                                                .map(|c| c.name.clone())
                                                .collect(),
                                            "method" => self
                                                .dialect
                                                .index_methods()
                                                .iter()
                                                .map(|m| String::from(*m))
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
                                this.write_entry(Some(*value), None, w, cx)
                            })),
                    ))
                },
            )
            .when(self.page == Page::ForeignKeys, |v| {
                let spinning = self.references.spin.spinning(self.references.loading);
                v.child(
                    navidog_ui::refresh_spin::refresh_button(
                        "designer-reference-refresh",
                        spinning,
                        gpui_kit::component::Size::Small,
                    )
                    .ghost()
                    .label(t!("designer.refresh_references").to_string())
                    .disabled(disabled && !spinning)
                    .on_click(cx.listener(|this, _, w, cx| {
                        if this.references.spin.spinning(this.references.loading) {
                            return;
                        }
                        this.load_references(w, cx);
                    })),
                )
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
            .capture_action(cx.listener(|this, _: &Enter, w, cx| this.on_sql_enter(w, cx)))
            .capture_action(cx.listener(|this, _: &IndentInline, w, cx| this.on_sql_tab(w, cx)))
            .capture_action(cx.listener(|this, _: &MoveUp, w, cx| this.note_sql_menu_moved(w, cx)))
            .capture_action(
                cx.listener(|this, _: &MoveDown, w, cx| this.note_sql_menu_moved(w, cx)),
            )
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
            .child({
                let pages: Vec<(Page, &'static str)> = [
                    (Page::Fields, "fields"),
                    (Page::Indexes, "indexes"),
                    (Page::ForeignKeys, "foreign_keys"),
                    (Page::Checks, "checks"),
                    (Page::Exclusions, "exclusions"),
                    (Page::Triggers, "triggers"),
                    (Page::Partitions, "partitions"),
                    (Page::Options, "options"),
                    (Page::Preview, "preview"),
                    (Page::Advanced, "advanced"),
                ]
                .into_iter()
                .filter(|(page, _)| {
                    let features = self.dialect.features();
                    match page {
                        Page::Exclusions => features.supports_exclude,
                        Page::Partitions => features.supports_partitions,
                        _ => true,
                    }
                })
                .collect();
                let selected = pages
                    .iter()
                    .position(|(page, _)| *page == self.page)
                    .unwrap_or(0);
                let pages_for_bar = pages.clone();
                TabBar::new("designer-pages")
                    .segmented()
                    .small()
                    .w_full()
                    .selected_index(selected)
                    .on_click(cx.listener(move |this, ix: &usize, window, cx| {
                        if let Some((page, _)) = pages_for_bar.get(*ix) {
                            this.switch_page(*page, window, cx);
                        }
                    }))
                    .children(pages.into_iter().map(|(page, key)| {
                        let label = t!(format!("designer.{key}")).to_string();
                        Tab::new().child(
                            div()
                                .id(SharedString::from(format!("designer-page-{key}")))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.switch_page(page, window, cx);
                                }))
                                .child(label)
                                .test_support(),
                        )
                    }))
            })
            .when_some(self.error.clone(), |v, error| {
                v.child(gpui_kit::div().px_2().text_color(p.danger).child(error))
            })
            .when(
                self.draft
                    .constraints
                    .iter()
                    .chain(&self.draft.indexes)
                    .any(|s| parse_entry(s, self.dialect.as_ref()).is_none()),
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
            // While the first load is running, reload returns immediately. Offer it again only
            // after that load finishes without data, or when a save asks for a reload.
            .when(self.needs_reload || (!self.loaded && !self.busy), |v| {
                let spinning = self.reload_spin.spinning(self.reloading);
                v.child(
                    navidog_ui::refresh_spin::refresh_button(
                        "designer-reload",
                        spinning,
                        gpui_kit::component::Size::Medium,
                    )
                    .label(t!("designer.reload").to_string())
                    .disabled(self.busy && !spinning)
                    .on_click(cx.listener(|this, _, w, cx| {
                        if this.reload_spin.spinning(this.reloading) {
                            return;
                        }
                        this.reload(w, cx);
                    })),
                )
            })
            .child(gpui_kit::div().flex_1().min_h_0().min_w_0().w_full().child(
                if self.busy && !self.loaded {
                    loading_grid().into_any_element()
                } else {
                    match self.page {
                        Page::Fields | Page::Indexes | Page::ForeignKeys | Page::Checks => {
                            self.render_grid_page(window, cx).into_any_element()
                        }
                        Page::Exclusions | Page::Triggers | Page::Partitions => {
                            self.render_more(cx).into_any_element()
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
                                navidog_ui::skin::single_line_sql(&self.definition)
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
                    }
                },
            ))
    }
}

/// Column-shaped bars shown until the first table snapshot arrives.
fn loading_grid() -> gpui_kit::AnyElement {
    const COLS: [(tokens::Pt, f32); 3] = [
        (tokens::designer::NAME_WIDTH, 0.72),
        (tokens::designer::TYPE_WIDTH, 0.55),
        (tokens::designer::ARG_WIDTH, 0.6),
    ];
    v_flex()
        .size_full()
        .children((0..8).map(|row| {
            h_flex()
                .h(tokens::designer::ROW_HEIGHT.rems())
                .px_2()
                .gap_2()
                .items_center()
                .children(
                    COLS.into_iter()
                        .enumerate()
                        .map(|(col, (width, fraction))| {
                            let fraction = (fraction - ((row + col) % 3) as f32 * 0.08).max(0.35);
                            div().w(width.rems()).flex_none().child(
                                Skeleton::new()
                                    .h(tokens::font::LINE_BODY.rems())
                                    .w(relative(fraction))
                                    .rounded_sm(),
                            )
                        }),
                )
        }))
        .into_any_element()
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
