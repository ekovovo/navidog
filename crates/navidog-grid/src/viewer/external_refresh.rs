//! Refreshes safe table viewers after SQL executed by another panel.

use super::*;

impl TableViewer {
    /// Unlike an explicit page command, a database notification never commits local input.
    fn external_refresh_blocked(&self, cx: &App) -> bool {
        let grid = self.grid.read(cx);
        if self.in_transaction
            || self.transaction_pending
            || grid.is_editing(cx)
            || grid.model(cx).has_changes()
        {
            return true;
        }
        if let Some(row) = self.form_for {
            let dialect = self.dialect.unwrap_or(Dialect::Sqlite);
            if self.form_inputs.iter().enumerate().any(|(column, input)| {
                input.read(cx).value().as_ref()
                    != grid.model(cx).raw_text(row, column, self.raw_data, dialect)
            }) {
                return true;
            }
        }
        let editor_text = match self.editor {
            Some(Kind::Json) => Some(self.json_input.read(cx).value().to_string()),
            Some(Kind::Text | Kind::Hex | Kind::Web) => {
                Some(self.editor_input.read(cx).value().to_string())
            }
            _ => None,
        };
        editor_text.is_some_and(|text| text != self.editor_baseline)
    }

    pub(super) fn accept_catalog(
        &mut self,
        columns: Vec<ColumnInfo>,
        foreign_keys: Vec<navidog_db::ForeignKey>,
        cx: &mut Context<Self>,
    ) {
        self.columns = columns;
        let names = foreign_keys.iter().map(|key| key.column.clone()).collect();
        self.foreign_keys = foreign_keys;
        self.metadata_stale = false;
        self.grid
            .update(cx, |grid, cx| grid.set_foreign_keys(names, cx));
    }

    pub(super) fn refresh_after_external_sql(&mut self, cx: &mut Context<Self>) {
        self.metadata_stale = true;
        if self.busy {
            self.external_refresh_pending = true;
            return;
        }
        self.external_refresh_pending = false;
        if self.external_refresh_blocked(cx) {
            self.status = t!("viewer.external_refresh_pending").into();
            cx.notify();
            return;
        }
        let (Some(session), Some(dialect)) = (self.session.clone(), self.dialect) else {
            return;
        };
        let (scope, table) = (self.object.scope.clone(), self.object.name.clone());
        let filter = self.filter.read(cx).value().to_string();
        let order = self.order.read(cx).value().to_string();
        let sql = dialect.select_page(
            &scope,
            &table,
            &filter,
            &order,
            self.limit,
            self.page * self.limit.unwrap_or(0),
            self.show_rowid,
        );
        let count_sql = dialect.count(&scope, &table, &filter);
        self.status = sql.clone().into();
        self.set_busy(true, cx);
        cx.notify();
        // Keep metadata and rows together on the existing session. Reconnecting here would
        // discard transactions and other state owned by the table viewer.
        let job = run_on(session.clone(), move |rel| {
            Box::pin(async move {
                let columns = rel.table_columns(&scope, &table).await?;
                let foreign_keys = rel.foreign_keys(&scope, &table).await?;
                let page = rel.execute(&sql).await?;
                let count = rel.execute(&count_sql).await?;
                Ok((columns, foreign_keys, page, first_cell(&count)))
            })
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| crate::db_text(&e)));
            let pending_close = this
                .update(cx, |this, cx| {
                    // A connection reopen may have replaced the session while the job completed.
                    if !this
                        .session
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &session))
                    {
                        return false;
                    }
                    // Form and field-editor inputs can still receive text while the grid is locked.
                    if this.external_refresh_blocked(cx) {
                        this.set_busy(false, cx);
                        this.status = t!("viewer.external_refresh_pending").into();
                        cx.notify();
                        return this.close_after_commit;
                    }
                    match result {
                        Ok((columns, foreign_keys, page, total)) => {
                            this.accept_catalog(columns, foreign_keys, cx);
                            this.total = total;
                            this.apply_page(Ok(page), cx)
                        }
                        Err(error) => {
                            this.fail(error, cx);
                            this.close_after_commit
                        }
                    }
                })
                .unwrap_or(false);
            if pending_close {
                cx.update(|cx| TableViewer::resume_close(this.clone(), cx));
            }
        }));
    }
}
