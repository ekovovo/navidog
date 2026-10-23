use super::*;

const PROPERTIES: [ColumnProperty; 11] = [
    ColumnProperty::Type,
    ColumnProperty::Length,
    ColumnProperty::Scale,
    ColumnProperty::Comment,
    ColumnProperty::Default,
    ColumnProperty::Collation,
    ColumnProperty::Charset,
    ColumnProperty::GeneratedExpression,
    ColumnProperty::GeneratedStorage,
    ColumnProperty::OnUpdate,
    ColumnProperty::Identity,
];
pub(super) const PROPERTY_IDS: [&str; 11] = [
    "designer-field-type",
    "designer-field-length",
    "designer-field-scale",
    "designer-field-comment",
    "designer-field-default",
    "designer-field-collation",
    "designer-field-charset",
    "designer-field-expression",
    "designer-field-storage",
    "designer-field-on-update",
    "designer-field-identity",
];
pub(super) const REFERENCE_SCHEMA: usize = 4;
pub(super) const REFERENCE_TABLE: usize = 5;
pub(super) const REFERENCE_FIELDS: usize = 6;
pub(super) const ENTRY_KEYS: [&str; 10] = [
    "name",
    "fields",
    "method",
    "comment",
    "reference_schema",
    "reference_table",
    "reference_fields",
    "on_delete",
    "on_update",
    "expression",
];

impl TableDesigner {
    pub(super) fn entries(&self) -> Vec<(bool, usize, String, Option<DesignEntry>)> {
        self.draft
            .constraints
            .iter()
            .enumerate()
            .map(|(ix, s)| (false, ix, s.clone(), parse_entry(s)))
            .chain(
                self.draft
                    .indexes
                    .iter()
                    .enumerate()
                    .map(|(ix, s)| (true, ix, s.clone(), parse_entry(s))),
            )
            .filter(|(_, _, _, e)| match self.page {
                Page::Indexes => e.as_ref().is_some_and(|e| {
                    matches!(
                        e.kind,
                        EntryKind::Index | EntryKind::Unique | EntryKind::Primary
                    )
                }),
                Page::ForeignKeys => e.as_ref().is_some_and(|e| e.kind == EntryKind::ForeignKey),
                Page::Checks => e.as_ref().is_some_and(|e| e.kind == EntryKind::Check),
                _ => false,
            })
            .collect()
    }
    pub(super) fn switch_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if page == Page::Preview {
            self.show_preview(window, cx);
            return;
        }
        self.flush(cx);
        self.page = page;
        if page == Page::Options {
            self.sync_table_options(window, cx);
        }
        if page == Page::Fields {
            self.sync_column(window, cx);
        }
        self.active_cell = None;
        self.entry_selected = None;
        if !self.entries().is_empty() {
            self.entry_selected = Some(0);
            self.sync_entry(window, cx);
        }
        self.refresh_grid(cx);
        cx.notify();
    }
    pub(super) fn refresh_grid(&mut self, cx: &mut Context<Self>) {
        let (headers, widths, rows, selected, editing) = if self.page == Page::Fields {
            let headers = vec![
                "name",
                "data_type",
                "length",
                "scale",
                "not_null",
                "virtual",
                "key",
                "comment",
            ];
            use tokens::designer::*;
            let widths = vec![
                NAME_WIDTH,
                TYPE_WIDTH,
                ARG_WIDTH,
                ARG_WIDTH,
                FLAG_WIDTH,
                FLAG_WIDTH,
                FLAG_WIDTH,
                COMMENT_WIDTH,
            ];
            let rows = self
                .draft
                .columns
                .iter()
                .map(|c| {
                    let p = column_properties(&c.definition);
                    vec![
                        c.name.clone(),
                        p.data_type,
                        p.length,
                        p.scale,
                        p.not_null.to_string(),
                        p.generated.to_string(),
                        self.is_primary(&c.name).to_string(),
                        if self.dialect == Dialect::Postgres {
                            c.comment.clone()
                        } else {
                            p.comment
                        },
                    ]
                })
                .collect();
            let row = self
                .draft
                .columns
                .iter()
                .position(|c| Some(c.id) == self.selected);
            let editing = row.zip(self.active_cell).and_then(|(r, c)| {
                let (input, id) = match c {
                    0 => (self.name.clone(), "designer-field-name"),
                    1 => (self.properties[0].clone(), PROPERTY_IDS[0]),
                    2 => (self.properties[1].clone(), PROPERTY_IDS[1]),
                    3 => (self.properties[2].clone(), PROPERTY_IDS[2]),
                    7 => (self.properties[3].clone(), PROPERTY_IDS[3]),
                    _ => return None,
                };
                Some((r, c, input, id))
            });
            (headers, widths, rows, row, editing)
        } else {
            let headers = match self.page {
                Page::Indexes => vec!["name", "fields", "index_type", "method", "comment"],
                Page::ForeignKeys => vec![
                    "name",
                    "fields",
                    "reference_schema",
                    "reference_table",
                    "reference_fields",
                    "on_delete",
                    "on_update",
                ],
                Page::Checks => vec!["name", "expression"],
                _ => vec![],
            };
            let widths = headers
                .iter()
                .map(|_| tokens::designer::NAME_WIDTH)
                .collect();
            let rows = self
                .entries()
                .iter()
                .map(|(_, _, _, e)| {
                    let e = e.as_ref().unwrap();
                    match self.page {
                        Page::Indexes => vec![
                            e.name.clone(),
                            e.fields.clone(),
                            if e.kind == EntryKind::Primary {
                                "PRIMARY"
                            } else if e.unique || e.kind == EntryKind::Unique {
                                "UNIQUE"
                            } else {
                                "NORMAL"
                            }
                            .into(),
                            e.method.clone(),
                            e.comment.clone(),
                        ],
                        Page::ForeignKeys => vec![
                            e.name.clone(),
                            e.fields.clone(),
                            e.reference_schema.clone(),
                            e.reference_table.clone(),
                            e.reference_fields.clone(),
                            e.on_delete.clone(),
                            e.on_update.clone(),
                        ],
                        _ => vec![e.name.clone(), e.expression.clone()],
                    }
                })
                .collect();
            (headers, widths, rows, self.entry_selected, None)
        };
        let fields = self.page == Page::Fields;
        let disabled = self.busy || !self.loaded;
        self.table.update(cx, |t, cx| {
            let d = t.delegate_mut();
            d.headers = headers;
            d.widths = widths;
            d.rows = rows;
            d.selected = selected;
            d.fields = fields;
            d.disabled = disabled;
            d.editing = editing;
            t.refresh(cx);
        });
    }
    pub(super) fn select_cell(
        &mut self,
        row: usize,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if self.busy || !self.loaded {
            return;
        }
        if self.page == Page::Fields {
            let Some(id) = self.draft.columns.get(row).map(|c| c.id) else {
                return;
            };
            // Clicking the active editor must not reset its value or caret.
            if self.selected == Some(id) && self.active_cell == Some(col) {
                return;
            }
            self.select(id, window, cx);
            self.active_cell = Some(col);
            self.refresh_grid(cx);
            let input = match col {
                0 => Some(self.name.clone()),
                1 => Some(self.properties[0].clone()),
                2 => Some(self.properties[1].clone()),
                3 => Some(self.properties[2].clone()),
                7 if self.dialect != Dialect::Sqlite => Some(self.properties[3].clone()),
                _ => None,
            };
            if let Some(input) = input {
                input.update(cx, |s, cx| s.focus(window, cx));
            }
        } else {
            self.entry_selected = Some(row);
            self.sync_entry(window, cx);
            self.refresh_grid(cx);
        }
        cx.notify();
    }
    pub(super) fn property_changed(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.loaded {
            return;
        }
        let value = self.properties[ix].read(cx).value().to_string();
        let Some(c) = self
            .draft
            .columns
            .iter_mut()
            .find(|c| Some(c.id) == self.selected)
        else {
            return;
        };
        if ix == 3 && self.dialect == Dialect::Postgres {
            if value.contains('\0') {
                self.invalid_inputs
                    .insert(ix, t!("designer.invalid_comment").to_string());
            } else {
                c.comment = value;
                self.invalid_inputs.remove(&ix);
                self.preview = None;
            }
            self.error = self.invalid_inputs.values().next().cloned();
            self.refresh_grid(cx);
            cx.notify();
            return;
        }
        match edit_column(&c.definition, PROPERTIES[ix], &value, self.dialect) {
            Ok(sql) => {
                c.definition = sql.clone();
                self.syncing = true;
                self.definition
                    .update(cx, |s, cx| s.set_value(sql, window, cx));
                self.syncing = false;
                self.preview = None;
                self.error = None;
                self.invalid_inputs.remove(&ix);
                self.error = self.invalid_inputs.values().next().cloned();
                self.refresh_grid(cx);
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.invalid_inputs.insert(ix, e.to_string());
            }
        };
        cx.notify();
    }
    pub(super) fn is_primary(&self, name: &str) -> bool {
        self.draft
            .columns
            .iter()
            .find(|c| c.name == name)
            .is_some_and(|c| column_properties(&c.definition).primary_key)
            || self
                .draft
                .constraints
                .iter()
                .filter_map(|s| parse_entry(s))
                .any(|e| {
                    e.kind == EntryKind::Primary
                        && split_field_list(&e.fields)
                            .unwrap_or_default()
                            .iter()
                            .any(|s| s.trim().trim_matches(['`', '"', '[', ']']) == name)
                })
    }
    pub(super) fn toggle_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Fields {
            return;
        }
        if let Some(row) = self
            .draft
            .columns
            .iter()
            .position(|c| Some(c.id) == self.selected)
        {
            let checked = !self.is_primary(&self.draft.columns[row].name);
            self.toggle_cell(row, 6, checked, window, cx);
        }
    }
    pub(super) fn toggle_cell(
        &mut self,
        row: usize,
        col: usize,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if self.busy || !self.loaded {
            return;
        }
        let Some(c) = self.draft.columns.get(row).cloned() else {
            return;
        };
        self.select(c.id, window, cx);
        if col == 6 {
            if column_properties(&c.definition).primary_key {
                if !checked {
                    match edit_column(
                        &c.definition,
                        ColumnProperty::PrimaryKey,
                        "false",
                        self.dialect,
                    ) {
                        Ok(sql) => self.draft.columns[row].definition = sql,
                        Err(e) => self.error = Some(e.to_string()),
                    };
                    self.preview = None;
                    self.sync_column(window, cx);
                    cx.notify();
                }
                return;
            }
            let primary = self
                .draft
                .constraints
                .iter()
                .position(|s| parse_entry(s).is_some_and(|e| e.kind == EntryKind::Primary));
            let mut entry = primary
                .and_then(|ix| parse_entry(&self.draft.constraints[ix]))
                .unwrap_or_else(|| DesignEntry::new(EntryKind::Primary));
            let mut fields = split_field_list(&entry.fields).unwrap_or_default();
            fields.retain(|s| s.trim_matches(['`', '"', '[', ']']) != c.name);
            if checked {
                fields.push(self.dialect.quote_ident(&c.name));
            }
            entry.fields = fields.join(", ");
            if let Some(ix) = primary {
                if fields.is_empty() {
                    self.draft.constraints.remove(ix);
                } else {
                    self.draft.constraints[ix] = entry
                        .sql(self.dialect, &self.scope, &self.draft.name, false)
                        .unwrap();
                }
            } else if !fields.is_empty() {
                self.draft.constraints.push(
                    entry
                        .sql(self.dialect, &self.scope, &self.draft.name, false)
                        .unwrap(),
                );
            }
            self.sync_definitions(window, cx);
        } else if col == 5 {
            match edit_column(
                &c.definition,
                ColumnProperty::Generated,
                if checked { "true" } else { "false" },
                self.dialect,
            ) {
                Ok(sql) => self.draft.columns[row].definition = sql,
                Err(e) => self.error = Some(e.to_string()),
            }
        } else if col == 4 {
            match edit_column(
                &c.definition,
                ColumnProperty::NotNull,
                if checked { "true" } else { "false" },
                self.dialect,
            ) {
                Ok(sql) => self.draft.columns[row].definition = sql,
                Err(e) => {
                    self.error = Some(e.to_string());
                }
            }
        }
        self.preview = None;
        self.sync_column(window, cx);
        cx.notify();
    }
    pub(super) fn toggle_attribute(
        &mut self,
        property: ColumnProperty,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if self.busy || !self.loaded {
            return;
        }
        if property == ColumnProperty::AutoIncrement && checked && self.dialect == Dialect::Sqlite {
            let Some(column) = self
                .draft
                .columns
                .iter()
                .find(|c| Some(c.id) == self.selected)
                .cloned()
            else {
                return;
            };
            if !column_properties(&column.definition)
                .data_type
                .eq_ignore_ascii_case("INTEGER")
            {
                self.error = Some(t!("designer.sqlite_auto_integer").to_string());
                cx.notify();
                return;
            }
            if !column_properties(&column.definition).primary_key {
                let primary = self
                    .draft
                    .constraints
                    .iter()
                    .position(|s| parse_entry(s).is_some_and(|e| e.kind == EntryKind::Primary));
                let single = primary
                    .and_then(|ix| parse_entry(&self.draft.constraints[ix]))
                    .is_some_and(|e| {
                        split_field_list(&e.fields).unwrap_or_default()
                            == vec![self.dialect.quote_ident(&column.name)]
                            || e.fields.trim() == column.name
                    });
                if !single {
                    self.error = Some(t!("designer.sqlite_auto_integer").to_string());
                    cx.notify();
                    return;
                }
                let sql = edit_column(
                    &column.definition,
                    ColumnProperty::PrimaryKey,
                    "true",
                    self.dialect,
                )
                .unwrap();
                self.draft
                    .columns
                    .iter_mut()
                    .find(|c| c.id == column.id)
                    .unwrap()
                    .definition = sql;
                self.draft.constraints.remove(primary.unwrap());
                self.sync_definitions(window, cx);
            }
        }
        let Some(c) = self
            .draft
            .columns
            .iter_mut()
            .find(|c| Some(c.id) == self.selected)
        else {
            return;
        };
        match edit_column(
            &c.definition,
            property,
            if checked { "true" } else { "false" },
            self.dialect,
        ) {
            Ok(s) => c.definition = s,
            Err(e) => {
                self.error = Some(e.to_string());
            }
        };
        self.preview = None;
        self.sync_column(window, cx);
        cx.notify();
    }
    fn sync_definitions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = true;
        self.constraints.update(cx, |s, cx| {
            s.set_value(self.draft.constraints.join("\n"), window, cx)
        });
        self.indexes.update(cx, |s, cx| {
            s.set_value(
                self.draft
                    .indexes
                    .iter()
                    .map(|s| format!("{s};"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                window,
                cx,
            )
        });
        self.syncing = false;
    }
    pub(super) fn sync_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let e = self
            .entry_selected
            .and_then(|ix| self.entries().get(ix).and_then(|e| e.3.clone()))
            .unwrap_or_else(|| DesignEntry::new(EntryKind::Advanced));
        self.syncing = true;
        for (input, value) in self.entry_inputs.iter().zip([
            e.name,
            e.fields,
            e.method,
            e.comment,
            e.reference_schema,
            e.reference_table,
            e.reference_fields,
            e.on_delete,
            e.on_update,
            e.expression,
        ]) {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.syncing = false;
        if self.page == Page::ForeignKeys {
            self.load_references(window, cx);
        }
    }
    pub(super) fn entry_changed(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::ForeignKeys && [REFERENCE_SCHEMA, REFERENCE_TABLE].contains(&ix) {
            self.syncing = true;
            for dependent in (ix + 1)..=REFERENCE_FIELDS {
                self.entry_inputs[dependent].update(cx, |s, cx| s.set_value("", window, cx));
            }
            self.syncing = false;
            self.load_references(window, cx);
        }
        self.write_entry(None, window, cx);
    }
    pub(super) fn write_entry(
        &mut self,
        unique: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.loaded {
            return;
        }
        let Some((standalone, ix, _, Some(mut e))) = self
            .entry_selected
            .and_then(|r| self.entries().get(r).cloned())
        else {
            return;
        };
        let v: Vec<String> = self
            .entry_inputs
            .iter()
            .map(|i| i.read(cx).value().to_string())
            .collect();
        e.name = v[0].clone();
        e.fields = v[1].clone();
        e.method = v[2].clone();
        e.comment = v[3].clone();
        e.reference_schema = v[REFERENCE_SCHEMA].clone();
        e.reference_table = v[REFERENCE_TABLE].clone();
        e.reference_fields = v[REFERENCE_FIELDS].clone();
        e.on_delete = v[7].clone();
        e.on_update = v[8].clone();
        e.expression = v[9].clone();
        if let Some(unique) = unique {
            e.unique = unique;
        }
        match e.sql(self.dialect, &self.scope, &self.draft.name, standalone) {
            Ok(sql) => {
                if standalone {
                    self.draft.indexes[ix] = sql;
                } else {
                    self.draft.constraints[ix] = sql;
                }
                self.error = None;
                self.invalid_inputs.remove(&100);
                self.error = self.invalid_inputs.values().next().cloned();
                self.preview = None;
                self.sync_definitions(window, cx);
                self.refresh_grid(cx);
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.invalid_inputs.insert(100, e.to_string());
            }
        }
        cx.notify();
    }
    pub(super) fn add_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if self.busy || !self.loaded {
            return;
        }
        let kind = match self.page {
            Page::Indexes => EntryKind::Index,
            Page::ForeignKeys => EntryKind::ForeignKey,
            Page::Checks => EntryKind::Check,
            _ => return,
        };
        let mut e = DesignEntry::new(kind);
        e.name = format!(
            "{}{}",
            match kind {
                EntryKind::Index => "idx_",
                EntryKind::ForeignKey => "fk_",
                _ => "chk_",
            },
            self.draft.constraints.len() + self.draft.indexes.len() + 1
        );
        e.fields = self
            .draft
            .columns
            .first()
            .map(|c| self.dialect.quote_ident(&c.name))
            .unwrap_or_default();
        e.reference_table = "parent_table".into();
        e.reference_fields = self.dialect.quote_ident("id");
        e.expression = if kind == EntryKind::Check {
            "1 = 1".into()
        } else {
            String::new()
        };
        let standalone = kind == EntryKind::Index && self.dialect != Dialect::MySql;
        if let Ok(sql) = e.sql(self.dialect, &self.scope, &self.draft.name, standalone) {
            if standalone {
                self.draft.indexes.push(sql);
            } else {
                self.draft.constraints.push(sql);
            }
            self.entry_selected = self.entries().len().checked_sub(1);
            self.sync_definitions(window, cx);
            self.sync_entry(window, cx);
            self.preview = None;
            self.refresh_grid(cx);
            cx.notify();
        }
    }
    pub(super) fn delete_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.invalid_inputs.is_empty() {
            return;
        }

        if self.busy || !self.loaded {
            return;
        }
        if let Some((standalone, ix, _, _)) = self
            .entry_selected
            .and_then(|r| self.entries().get(r).cloned())
        {
            if standalone {
                self.draft.indexes.remove(ix);
            } else {
                self.draft.constraints.remove(ix);
            }
            self.entry_selected = self.entries().len().checked_sub(1);
            self.sync_definitions(window, cx);
            self.sync_entry(window, cx);
            self.preview = None;
            self.refresh_grid(cx);
            cx.notify();
        }
    }
}

impl TableDesigner {
    pub(super) fn choose_value(
        &mut self,
        action: &ChooseValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || !self.loaded {
            return;
        }
        if action.target >= 100 {
            let ix = action.target - 100;
            let Some(key) = super::options::keys(self.dialect).get(ix) else {
                return;
            };
            let Some(input) = self.table_option_inputs.get(key).cloned() else {
                return;
            };
            self.syncing = true;
            input.update(cx, |s, cx| s.set_value(action.value.clone(), window, cx));
            self.syncing = false;
            self.table_option_changed(ix, window, cx);
        } else if action.target < 10 || action.target == 90 {
            let ix = if action.target == 90 {
                10
            } else {
                action.target
            };
            let Some(input) = self.properties.get(ix).cloned() else {
                return;
            };
            self.syncing = true;
            input.update(cx, |s, cx| s.set_value(action.value.clone(), window, cx));
            self.syncing = false;
            self.property_changed(ix, window, cx);
        } else {
            let ix = action.target - 10;
            let Some(input) = self.entry_inputs.get(ix).cloned() else {
                return;
            };
            let value = if ix == 1 || ix == REFERENCE_FIELDS {
                let field = self.dialect.quote_ident(&action.value);
                let old = input.read(cx).value().to_string();
                let fields = split_field_list(&old).unwrap_or_default();
                if fields
                    .iter()
                    .any(|s| s.trim().trim_matches(['`', '"', '[', ']']) == action.value)
                {
                    fields
                        .iter()
                        .filter(|s| s.trim().trim_matches(['`', '"', '[', ']']) != action.value)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                } else if old.is_empty() {
                    field
                } else {
                    format!("{old}, {field}")
                }
            } else {
                action.value.clone()
            };
            self.syncing = true;
            input.update(cx, |s, cx| s.set_value(value, window, cx));
            self.syncing = false;
            self.entry_changed(ix, window, cx);
        }
    }
}
