//! Exclusion, trigger, and partition forms. They write the same draft the SQL plan reads.
use super::*;
use navidog_schema::table_design::DesignEntry;

impl TableDesigner {
    pub(super) fn render_more(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let disabled = self.busy || !self.loaded;
        let p = palette(cx);
        let (name_key, detail_key, extra_key, body_key) = match self.page {
            Page::Exclusions => ("name", "method", "where_clause", "elements"),
            Page::Triggers => ("name", "timing", "events", "definition"),
            _ => ("name", "partition_method", "bounds", "partition_key"),
        };
        v_flex()
            .id("designer-more")
            .size_full()
            .min_h_0()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .children(self.more_rows(cx))
            .child(
                self.more_field(
                    name_key,
                    Input::new(&self.more_name)
                        .id("designer-more-name")
                        .disabled(disabled),
                    disabled,
                ),
            )
            .child(
                self.more_field(
                    detail_key,
                    Input::new(&self.more_detail)
                        .id("designer-more-detail")
                        .disabled(disabled),
                    disabled,
                ),
            )
            .child(
                self.more_field(
                    extra_key,
                    Input::new(&self.more_extra)
                        .id("designer-more-extra")
                        .disabled(disabled),
                    disabled,
                ),
            )
            .child(
                gpui_kit::div()
                    .text_color(p.label_secondary)
                    .child(t!(format!("designer.{body_key}")).to_string()),
            )
            .child(
                Editor::new(&self.more_body)
                    .disabled(disabled)
                    .h(tokens::designer::PROPERTY_MIN_HEIGHT.rems()),
            )
            .child(
                Button::new("designer-more-apply")
                    .small()
                    .label(t!("designer.add_object").to_string())
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.apply_more(window, cx))),
            )
            .into_any_element()
    }

    fn more_field(&self, key: &str, input: impl IntoElement, _disabled: bool) -> impl IntoElement {
        FormRow::new(t!(format!("designer.{key}")).to_string(), input).hug_label()
    }

    fn more_rows(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let items = self.more_items();
        items
            .into_iter()
            .enumerate()
            .map(|(ix, label)| {
                h_flex()
                    .gap_2()
                    .child(truncated(
                        SharedString::from(format!("designer-more-row-{ix}")),
                        label,
                    ))
                    .child(
                        Button::new(SharedString::from(format!("designer-more-delete-{ix}")))
                            .ghost()
                            .small()
                            .icon(lucide("minus"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_more(ix, window, cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn more_items(&self) -> Vec<String> {
        match self.page {
            Page::Exclusions => self
                .draft
                .constraints
                .iter()
                .filter(|sql| {
                    parse_entry(sql, self.dialect.as_ref())
                        .is_some_and(|e| e.kind == EntryKind::Exclude)
                })
                .cloned()
                .collect(),
            Page::Triggers => self.draft.triggers.clone().unwrap_or_else(|| {
                self.baseline
                    .as_ref()
                    .map(|old| old.triggers.clone())
                    .unwrap_or_default()
            }),
            Page::Partitions => std::iter::once(self.draft.partition.clone())
                .filter(|s| !s.is_empty())
                .chain(self.draft.partition_children.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn apply_more(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.more_name.read(cx).value().to_string();
        let detail = self.more_detail.read(cx).value().to_string();
        let extra = self.more_extra.read(cx).value().to_string();
        let body = self.more_body.read(cx).value().to_string();
        let result = match self.page {
            Page::Exclusions => self.add_exclusion(&name, &detail, &extra, &body),
            Page::Triggers => self.add_trigger(&name, &detail, &extra, &body),
            Page::Partitions => self.add_partition(&name, &detail, &extra, &body),
            _ => Ok(()),
        };
        match result {
            Ok(()) => {
                self.error = None;
                self.preview = None;
                if self.page == Page::Exclusions {
                    self.write_constraints(window, cx);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// `flush` rebuilds constraints from the editor, so an exclusion edit has to land there.
    fn write_constraints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.draft.constraints.join("\n");
        self.syncing = true;
        self.constraints
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.syncing = false;
    }

    fn add_exclusion(
        &mut self,
        name: &str,
        method: &str,
        predicate: &str,
        elements: &str,
    ) -> DbResult<()> {
        let mut entry = DesignEntry::new(EntryKind::Exclude);
        entry.name = name.to_string();
        entry.method = method.to_string();
        entry.expression = predicate.to_string();
        entry.fields = elements.trim().to_string();
        let sql = entry.sql(self.dialect.as_ref(), &self.scope, &self.draft.name, false)?;
        self.draft.constraints.push(sql);
        Ok(())
    }

    fn add_trigger(&mut self, name: &str, timing: &str, event: &str, body: &str) -> DbResult<()> {
        if name.is_empty() || timing.is_empty() || event.is_empty() || body.trim().is_empty() {
            return Err(DbError::msg(
                "A trigger needs a name, timing, event, and body.",
            ));
        }
        let table = self.dialect.qualified(&self.scope, &self.draft.name);
        let quoted = self.dialect.quote_ident(name);
        let sql = if self.dialect.features().enriches_from_catalog {
            format!("CREATE TRIGGER {quoted} {timing} {event} ON {table} FOR EACH ROW {body}")
        } else if self.dialect.features().rebuilds_tables {
            sqlite_trigger_statement(&quoted, timing, event, &table, body)
        } else {
            format!("CREATE TRIGGER {quoted} {timing} {event} ON {table} FOR EACH ROW {body}")
        };
        let mut triggers = self.draft.triggers.clone().unwrap_or_else(|| {
            self.baseline
                .as_ref()
                .map(|old| old.triggers.clone())
                .unwrap_or_default()
        });
        triggers.push(sql);
        self.draft.triggers = Some(triggers);
        Ok(())
    }

    fn add_partition(&mut self, name: &str, method: &str, bounds: &str, key: &str) -> DbResult<()> {
        if !self.dialect.features().supports_partitions {
            return Err(DbError::msg("This database cannot partition tables."));
        }
        if method.is_empty() || key.trim().is_empty() {
            return Err(DbError::msg("A partition needs a method and a key."));
        }
        if self.dialect.features().inline_table_options {
            self.draft.partition =
                mysql_partition(method, key, name, bounds, self.dialect.as_ref());
            return Ok(());
        }
        if !self.draft.partition.is_empty()
            && self.baseline.as_ref().is_some_and(|old| {
                !old.design.partition.is_empty()
                    && old.design.partition != format!("PARTITION BY {method} ({key})")
            })
        {
            return Err(DbError::msg(
                "Changing the partition method of an existing table is not supported.",
            ));
        }
        self.draft.partition = format!("PARTITION BY {method} ({key})");
        if !name.is_empty() {
            let child = format!(
                "CREATE TABLE {} PARTITION OF {} FOR VALUES {}",
                self.dialect.quote_ident(name),
                self.dialect.qualified(&self.scope, &self.draft.name),
                bounds.trim()
            );
            self.draft.partition_children.push(child);
        }
        Ok(())
    }

    fn delete_more(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Exclusions => {
                let mut seen = 0;
                self.draft.constraints.retain(|sql| {
                    let exclude = parse_entry(sql, self.dialect.as_ref())
                        .is_some_and(|e| e.kind == EntryKind::Exclude);
                    if !exclude {
                        return true;
                    }
                    let keep = seen != ix;
                    seen += 1;
                    keep
                });
                self.write_constraints(window, cx);
            }
            Page::Triggers => {
                let mut triggers = self.draft.triggers.clone().unwrap_or_else(|| {
                    self.baseline
                        .as_ref()
                        .map(|old| old.triggers.clone())
                        .unwrap_or_default()
                });
                if ix < triggers.len() {
                    triggers.remove(ix);
                }
                self.draft.triggers = Some(triggers);
            }
            Page::Partitions => {
                if ix == 0 && !self.draft.partition.is_empty() {
                    self.draft.partition.clear();
                } else {
                    let child = ix - usize::from(!self.draft.partition.is_empty());
                    if child < self.draft.partition_children.len() {
                        self.draft.partition_children.remove(child);
                    }
                }
            }
            _ => {}
        }
        self.preview = None;
        cx.notify();
    }
}

/// SQLite wants one `;` before `END`. A body that already ends with `;` must not become `;; END`.
/// A trailing `--` comment would swallow `; END` on the same line, so that comment stays after
/// the terminator and `END` moves to the next line.
fn sqlite_trigger_statement(
    quoted: &str,
    timing: &str,
    event: &str,
    table: &str,
    body: &str,
) -> String {
    let (code, comment) = split_trailing_line_comment(body);
    let code = code.trim().trim_end_matches(';').trim();
    let head = format!("CREATE TRIGGER {quoted} {timing} {event} ON {table} BEGIN {code};");
    if comment.is_empty() {
        format!("{head} END")
    } else {
        format!("{head} {comment}\nEND")
    }
}

/// `(code, trailing line comments)`. The comment text starts at `--`.
fn split_trailing_line_comment(body: &str) -> (&str, &str) {
    let spans = navidog_sql::span::spans(body, navidog_sql::span::Lex::SQLITE);
    let mut comment_at = None;
    for span in spans.iter().rev() {
        if span.kind != navidog_sql::span::SpanKind::LineComment {
            break;
        }
        comment_at = Some(span.range.start);
    }
    match comment_at {
        Some(at) => (&body[..at], body[at..].trim_end()),
        None => (body, ""),
    }
}

fn mysql_partition(
    method: &str,
    key: &str,
    name: &str,
    bounds: &str,
    dialect: &dyn Dialect,
) -> String {
    let bounds = bounds.trim();
    let name = name.trim();
    let tail = if name.is_empty() {
        if bounds.is_empty() {
            String::new()
        } else {
            format!(" {bounds}")
        }
    } else {
        let values = if bounds.is_empty() {
            String::new()
        } else {
            format!(" {bounds}")
        };
        format!(" (PARTITION {}{values})", dialect.quote_ident(name))
    };
    format!("PARTITION BY {method}({key}){tail}")
}

#[cfg(test)]
mod tests {
    use super::mysql_partition;

    #[test]
    fn mysql_partition_keeps_the_partition_name() {
        let dialect = navidog_postgres::PostgresDialect;
        assert_eq!(
            mysql_partition("RANGE", "id", "p0", "VALUES LESS THAN (10)", &dialect),
            "PARTITION BY RANGE(id) (PARTITION \"p0\" VALUES LESS THAN (10))"
        );
        assert_eq!(
            mysql_partition("HASH", "id", "", "PARTITIONS 4", &dialect),
            "PARTITION BY HASH(id) PARTITIONS 4"
        );
    }

    #[test]
    fn sqlite_trigger_body_keeps_one_terminator() {
        let sql = super::sqlite_trigger_statement(
            "\"audit\"",
            "AFTER",
            "INSERT",
            "\"main\".\"t\"",
            "INSERT INTO a VALUES (1);\nINSERT INTO a VALUES (2);",
        );
        assert!(
            sql.ends_with("BEGIN INSERT INTO a VALUES (1);\nINSERT INTO a VALUES (2); END"),
            "{sql}"
        );
        assert!(!sql.contains(";;"), "{sql}");
        let bare = super::sqlite_trigger_statement(
            "\"audit\"",
            "AFTER",
            "INSERT",
            "\"t\"",
            "INSERT INTO a VALUES (1)",
        );
        assert!(
            bare.ends_with("BEGIN INSERT INTO a VALUES (1); END"),
            "{bare}"
        );
        for body in [
            "INSERT INTO audit VALUES (1); -- audit",
            "INSERT INTO audit VALUES (1) -- audit",
        ] {
            let sql =
                super::sqlite_trigger_statement("\"audit\"", "AFTER", "INSERT", "\"t\"", body);
            assert!(
                sql.contains("BEGIN INSERT INTO audit VALUES (1); -- audit\nEND"),
                "{body} -> {sql}"
            );
            assert!(!sql.contains(";;"), "{sql}");
        }
    }
}
