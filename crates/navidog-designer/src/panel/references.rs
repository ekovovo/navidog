use super::structured::{REFERENCE_FIELDS, REFERENCE_SCHEMA, REFERENCE_TABLE};
use super::*;
use navidog_core::object::ObjectKind;

#[derive(Default)]
pub(super) struct ReferenceChoices {
    pub scopes: Vec<String>,
    pub tables: Vec<String>,
    pub fields: Vec<String>,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
}

type ReferenceSnapshot = (Vec<String>, Vec<String>, Vec<String>);

impl ReferenceChoices {
    fn finish(&mut self, generation: u64, result: DbResult<ReferenceSnapshot>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        match result {
            Ok((scopes, tables, fields)) => {
                self.scopes = scopes;
                self.tables = tables;
                self.fields = fields;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        true
    }
}

impl TableDesigner {
    pub(super) fn reference_values(&self, ix: usize, cx: &App) -> Vec<String> {
        match ix {
            REFERENCE_SCHEMA => self.references.scopes.clone(),
            REFERENCE_TABLE => {
                let mut tables = self.references.tables.clone();
                if self.reference_is_self(cx)
                    && !self.draft.name.is_empty()
                    && !tables.contains(&self.draft.name)
                {
                    tables.push(self.draft.name.clone());
                }
                tables
            }
            REFERENCE_FIELDS
                if self.reference_is_self(cx)
                    && self.entry_inputs[REFERENCE_TABLE].read(cx).value() == self.draft.name =>
            {
                self.draft.columns.iter().map(|c| c.name.clone()).collect()
            }
            REFERENCE_FIELDS => self.references.fields.clone(),
            _ => Vec::new(),
        }
    }

    fn reference_is_self(&self, cx: &App) -> bool {
        let scope = self.entry_inputs[REFERENCE_SCHEMA].read(cx).value();
        scope.is_empty()
            || scope
                == match self.dialect {
                    Dialect::Postgres => self.scope.schema.as_deref().unwrap_or("public"),
                    _ => &self.scope.database,
                }
    }

    pub(super) fn load_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.references.generation += 1;
        let generation = self.references.generation;
        self.reference_task.take();
        self.references.scopes.clear();
        self.references.tables.clear();
        self.references.fields.clear();
        self.references.error = None;
        self.references.loading = false;
        let mut scope = self.scope.clone();
        let selected_scope = self.entry_inputs[REFERENCE_SCHEMA]
            .read(cx)
            .value()
            .to_string();
        let table = self.entry_inputs[REFERENCE_TABLE]
            .read(cx)
            .value()
            .to_string();
        if !selected_scope.is_empty() {
            match self.dialect {
                Dialect::MySql => scope.database = selected_scope,
                Dialect::Postgres => scope.schema = Some(selected_scope),
                Dialect::Sqlite => {}
            }
        }
        let Some(job) = Sessions::global(cx).read(cx).run_rel(
            &self.scope.connection,
            Some(&self.scope.database),
            move |r| {
                Box::pin(async move {
                    let scopes = match r.dialect() {
                        Dialect::MySql => r.list_databases().await?,
                        Dialect::Postgres => r.list_schemas().await?,
                        Dialect::Sqlite => Vec::new(),
                    };
                    let tables: Vec<_> = r
                        .list_objects(&scope, ObjectKind::Table)
                        .await?
                        .into_iter()
                        .map(|t| t.name)
                        .collect();
                    let fields = if tables.contains(&table) {
                        r.table_columns(&scope, &table)
                            .await?
                            .into_iter()
                            .map(|c| c.name)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    Ok((scopes, tables, fields))
                })
            },
        ) else {
            self.references.error = Some(t!("designer.disconnected").to_string());
            cx.notify();
            return;
        };
        self.references.loading = true;
        self.reference_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = job
                .await
                .map_err(|e| DbError::msg(e.to_string()))
                .and_then(|r| r);
            let _ = this.update_in(cx, |this, _, cx| {
                if !this.references.finish(generation, result) {
                    return;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_success_and_failure_cannot_replace_new_selection() {
        let mut choices = ReferenceChoices {
            generation: 2,
            loading: true,
            ..Default::default()
        };
        assert!(!choices.finish(
            1,
            Ok((vec![], vec!["old".into()], vec!["old_field".into()]))
        ));
        assert!(choices.loading);
        assert!(choices.tables.is_empty());
        assert!(!choices.finish(1, Err(DbError::msg("old failure"))));
        assert!(choices.error.is_none());
        assert!(choices.finish(
            2,
            Ok((vec![], vec!["new".into()], vec!["new_field".into()]))
        ));
        assert!(!choices.loading);
        assert_eq!(choices.fields, vec!["new_field"]);
    }
}
