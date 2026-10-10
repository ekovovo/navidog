//! The grid's context menu, Copy As, and the column and value a quick filter uses.

use super::*;

impl ResultGrid {
    pub(super) fn filter_target(&self, cx: &App) -> Option<(SharedString, String, bool)> {
        let m = self.model(cx);
        if m.columns.is_empty() || m.row_count() == 0 {
            return None;
        }
        let (r, c) = m.head();
        let name = m.columns.get(c)?.name.clone();
        match m.value(r, c) {
            Some(Some(text)) => Some((name, text.to_string(), false)),
            _ => Some((name, String::new(), true)),
        }
    }

    pub(super) fn context_menu(&self, cx: &App) -> NativeMenu {
        let copy_as = |key: &str| {
            Box::new(CopyAs {
                format: key.to_string().into(),
            })
        };
        let mut sub = NativeMenu::new();
        if self.target.is_some() {
            sub = sub
                .menu(t!("grid.copy_insert").to_string(), copy_as("insert"))
                .menu(t!("grid.copy_update").to_string(), copy_as("update"))
                .separator();
        }
        let sub = sub
            .menu(t!("grid.copy_tsv_data").to_string(), copy_as("tsv"))
            .menu(t!("grid.copy_tsv_names").to_string(), copy_as("names"))
            .menu(t!("grid.copy_tsv_both").to_string(), copy_as("both"));
        let quick = |column: SharedString, op: &str, value: &str| {
            Box::new(QuickFilter {
                column,
                op: op.to_string().into(),
                value: value.to_string().into(),
            }) as Box<dyn gpui_kit::Action>
        };
        let mut menu = NativeMenu::new()
            .menu(t!("grid.set_empty").to_string(), Box::new(SetEmpty))
            .menu(t!("grid.set_null").to_string(), Box::new(SetNull))
            .separator()
            .menu(t!("grid.copy").to_string(), Box::new(Copy))
            .submenu(t!("grid.copy_as").to_string(), sub)
            .menu(t!("grid.paste").to_string(), Box::new(Paste));
        if let Some((column, value, is_null)) = self.filter_target(cx) {
            let shown = menu_value(&value);
            menu = menu.separator();
            if is_null {
                menu = menu.menu(
                    t!("grid.filter_null", column = column).to_string(),
                    quick(column, "is_null", ""),
                );
            } else {
                menu = menu
                    .menu(
                        t!("grid.filter_eq", column = column, value = shown).to_string(),
                        quick(column.clone(), "eq", &value),
                    )
                    .menu(
                        t!("grid.filter_ne", column = column, value = shown).to_string(),
                        quick(column.clone(), "ne", &value),
                    )
                    .menu(
                        t!("grid.filter_contains", column = column, value = shown).to_string(),
                        quick(column, "contains", &value),
                    );
            }
        }
        if let Some((column, _, _)) = self.filter_target(cx)
            && self.foreign_keys.iter().any(|name| name == column.as_ref())
        {
            menu = menu.menu(t!("grid.fk_pick").to_string(), Box::new(PickForeignKey));
        }
        let mut menu = menu
            .separator()
            .menu(t!("grid.delete_rows").to_string(), Box::new(DeleteRows))
            .separator()
            .menu(t!("grid.row_height").to_string(), Box::new(SetRowHeight));
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.dialect == Dialect::Sqlite)
        {
            menu = menu.menu_with_check(
                t!("grid.rowid").to_string(),
                self.show_rowid,
                Box::new(ToggleRowId),
            );
        }
        menu.separator().menu_with_check(
            t!("grid.raw_data").to_string(),
            self.raw,
            Box::new(ToggleRawData),
        )
    }

    pub(super) fn copy_text(&self, format: &str, cx: &App) -> String {
        let m = self.model(cx);
        let delegate = self.table.read(cx).delegate();
        let raw = delegate.raw.then_some(delegate.dialect);
        match (format, &self.target) {
            ("insert", Some(t)) => t.dialect.insert_statements(
                &t.scope,
                &t.table,
                &m.selected_columns(),
                &m.selected_values(),
            ),
            ("update", Some(t)) => {
                let keys: Vec<String> = m
                    .columns
                    .iter()
                    .filter(|c| c.primary_key)
                    .map(|c| c.name.to_string())
                    .collect();
                t.dialect.update_statements(
                    &t.scope,
                    &t.table,
                    &m.selected_columns(),
                    &keys,
                    &m.selected_values(),
                )
            }
            ("names", _) => m.selected_tsv(true, false, None),
            ("both", _) => m.selected_tsv(true, true, raw),
            _ => m.selected_tsv(false, true, raw),
        }
    }

    pub(super) fn set_selected(&mut self, v: Cell, cx: &mut Context<Self>) {
        self.update_model(cx, |m| {
            let (rows, cols) = m.selection();
            for r in rows {
                if m.row_state(r) == RowState::Deleted {
                    continue;
                }
                for c in cols.clone() {
                    m.set_value(r, c, v.clone());
                }
            }
        });
    }
}

fn menu_value(value: &str) -> String {
    let mut chars = value.chars();
    let short: String = chars.by_ref().take(32).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}
