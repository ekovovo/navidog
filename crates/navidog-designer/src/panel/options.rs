use super::*;
use gpui_kit::component::checkbox::Checkbox;
use navidog_db::table_design::{MYSQL_OPTIONS, POSTGRES_OPTIONS, edit_table_option, table_option};

pub(super) fn keys(d: Dialect) -> &'static [&'static str] {
    match d {
        Dialect::MySql => MYSQL_OPTIONS,
        Dialect::Postgres => POSTGRES_OPTIONS,
        Dialect::Sqlite => &[],
    }
}
impl TableDesigner {
    pub(super) fn sync_table_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = true;
        for (key, input) in &self.table_option_inputs {
            let value = if *key == "owner" {
                self.draft.owner.clone()
            } else if *key == "table_comment" && self.dialect == Dialect::Postgres {
                self.draft.comment.clone()
            } else {
                table_option(&self.draft.options, key, self.dialect).unwrap_or_default()
            };
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.syncing = false;
    }
    pub(super) fn table_option_changed(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.loaded {
            return;
        }
        let key = keys(self.dialect)[ix];
        let value = self.table_option_inputs[key].read(cx).value().to_string();
        let result = if key == "table_comment" && self.dialect == Dialect::Postgres {
            if value.contains('\0') {
                Err(DbError::msg(t!("designer.invalid_comment").to_string()))
            } else {
                self.draft.comment = value;
                Ok(self.draft.options.clone())
            }
        } else if key == "owner" {
            if value.contains('\0') {
                Err(DbError::msg(t!("designer.invalid_owner").to_string()))
            } else {
                self.draft.owner = value;
                Ok(self.draft.options.clone())
            }
        } else {
            edit_table_option(&self.draft.options, key, &value, self.dialect)
        };
        self.apply_option(result, 1000 + ix, window, cx);
    }
    fn apply_option(
        &mut self,
        result: DbResult<String>,
        error_key: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(sql) => {
                self.draft.options = sql.clone();
                self.syncing = true;
                self.options
                    .update(cx, |s, cx| s.set_value(sql, window, cx));
                self.syncing = false;
                self.invalid_inputs.remove(&error_key);
                self.preview = None;
            }
            Err(error) => {
                self.invalid_inputs.insert(error_key, error.to_string());
            }
        }
        self.error = self.invalid_inputs.values().next().cloned();
        cx.notify();
    }
    fn toggle_table_option(
        &mut self,
        key: &str,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.loaded {
            return;
        }
        if key == "unlogged" {
            self.draft.unlogged = checked;
            self.preview = None;
            cx.notify();
            return;
        }
        let result = edit_table_option(
            &self.draft.options,
            key,
            if checked { "true" } else { "false" },
            self.dialect,
        );
        self.apply_option(result, 2000, window, cx);
    }
    pub(super) fn render_table_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy || !self.loaded;
        v_flex()
            .id("designer-options-form")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .children(keys(self.dialect).iter().enumerate().map(|(ix, key)| {
                let parse_error = *key != "owner"
                    && !(*key == "table_comment" && self.dialect == Dialect::Postgres)
                    && table_option(&self.draft.options, key, self.dialect).is_err();
                let values: Vec<String> = match *key {
                    "engine" => vec!["InnoDB", "MyISAM", "MEMORY", "ARCHIVE"],
                    "charset" => vec!["utf8mb4", "utf8mb3", "latin1", "ascii", "binary"],
                    "row_format" => vec![
                        "DEFAULT",
                        "DYNAMIC",
                        "COMPACT",
                        "REDUNDANT",
                        "COMPRESSED",
                        "FIXED",
                    ],
                    "stats_auto_recalc" | "stats_persistent" => vec!["DEFAULT", "0", "1"],
                    "checksum" | "delay_key_write" => vec!["0", "1"],
                    "pack_keys" => vec!["DEFAULT", "0", "1"],
                    "insert_method" => vec!["NO", "FIRST", "LAST"],
                    "compression" => vec!["", "ZLIB", "LZ4", "NONE"],
                    "encryption" => vec!["Y", "N"],
                    "autovacuum_enabled" => vec!["", "true", "false"],
                    _ => vec![],
                }
                .into_iter()
                .map(String::from)
                .collect();
                let has_choices = !values.is_empty();
                FormRow::new(
                    t!(format!("designer.option_{key}")).to_string(),
                    h_flex()
                        .w(tokens::form::FIELD_WIDTH.rems())
                        .max_w_full()
                        .min_w_0()
                        .child(
                            Input::new(&self.table_option_inputs[key])
                                .id(SharedString::from(format!("designer-option-{key}")))
                                .small()
                                .disabled(disabled || parse_error)
                                .flex_1(),
                        )
                        .when(has_choices, |row| {
                            row.child(super::view::choices(
                                cx.entity().downgrade(),
                                100 + ix,
                                values,
                                disabled || parse_error,
                            ))
                        }),
                )
            }))
            .when(self.dialect == Dialect::Postgres, |v| {
                v.child(FormRow::bare(
                    Checkbox::new("designer-option-unlogged")
                        .label(t!("designer.option_unlogged").to_string())
                        .checked(self.draft.unlogged)
                        .disabled(disabled)
                        .on_click(cx.listener(|this, value, w, cx| {
                            this.toggle_table_option("unlogged", *value, w, cx)
                        })),
                ))
            })
            .when(self.dialect == Dialect::Sqlite, |v| {
                v.children(["without_rowid", "strict"].into_iter().map(|key| {
                    let value = table_option(&self.draft.options, key, self.dialect);
                    FormRow::bare(
                        Checkbox::new(SharedString::from(format!("designer-option-{key}")))
                            .label(t!(format!("designer.option_{key}")).to_string())
                            .checked(value.as_deref().is_ok_and(|v| v == "true"))
                            .disabled(disabled || value.is_err())
                            .on_click(cx.listener(move |this, value, w, cx| {
                                this.toggle_table_option(key, *value, w, cx)
                            })),
                    )
                }))
            })
            .child(
                gpui_kit::div()
                    .text_color(palette(cx).label_secondary)
                    .child(
                        t!(if self.dialect == Dialect::MySql {
                            "designer.table_options_mysql_help"
                        } else {
                            "designer.table_options_help"
                        })
                        .to_string(),
                    ),
            )
            .child(
                Button::new("designer-options-advanced")
                    .ghost()
                    .small()
                    .label(t!("designer.advanced").to_string())
                    .on_click(
                        cx.listener(|this, _, w, cx| this.switch_page(Page::Advanced, w, cx)),
                    ),
            )
    }
}
