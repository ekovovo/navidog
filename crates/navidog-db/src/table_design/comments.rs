//! PostgreSQL comment data and migration of legacy supplemental COMMENT statements.
use super::*;
use std::collections::HashSet;

fn word<'a>(sql: &'a str, expected: &str) -> DbResult<&'a str> {
    let sql = sql.trim_start();
    let end = sql.find(char::is_whitespace).unwrap_or(sql.len());
    if !sql[..end].eq_ignore_ascii_case(expected) {
        return Err(DbError::msg("Unsupported legacy comment definition."));
    }
    Ok(sql[end..].trim_start())
}
fn literal(sql: &str) -> DbResult<String> {
    if sql.eq_ignore_ascii_case("NULL") {
        return Ok(String::new());
    }
    let (escape, sql) = if sql.starts_with(['E', 'e']) {
        (true, &sql[1..])
    } else {
        (false, sql)
    };
    let body = sql
        .strip_prefix('\'')
        .ok_or_else(|| DbError::msg("Expected a comment string or NULL."))?;
    let mut chars = body.chars().peekable();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                value.push('\'');
            } else if chars.next().is_none() {
                return Ok(value);
            } else {
                break;
            }
        } else if escape && c == '\\' {
            let escaped = match chars.next() {
                Some('\\') => '\\',
                Some('\'') => '\'',
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('b') => '\u{8}',
                Some('f') => '\u{c}',
                _ => {
                    return Err(DbError::msg(
                        "Unsupported legacy comment escape; edit it in Advanced.",
                    ));
                }
            };
            value.push(escaped);
        } else {
            value.push(c);
        }
    }
    Err(DbError::msg("Invalid legacy comment string."))
}
fn decode(sql: &str) -> DbResult<(bool, Vec<String>, String)> {
    let sql = sql.trim().trim_end_matches(';').trim_end();
    parse::fragment(sql)?;
    let rest = word(word(sql, "COMMENT")?, "ON")?;
    let column = rest
        .split_whitespace()
        .next()
        .is_some_and(|v| v.eq_ignore_ascii_case("COLUMN"));
    let mut rest = word(rest, if column { "COLUMN" } else { "TABLE" })?;
    let mut names = Vec::new();
    loop {
        let quoted = rest.starts_with(['"', '`', '[']);
        let (name, tail) = parse::ident(rest)?;
        if quoted {
            names.push(name);
        } else {
            names.extend(name.split('.').map(str::to_ascii_lowercase));
        }
        rest = tail;
        if let Some(tail) = rest.strip_prefix('.') {
            rest = tail.trim_start();
        } else {
            break;
        }
    }
    let value = literal(word(rest, "IS")?)?;
    Ok((column, names, value))
}
fn is_comment(sql: &str) -> bool {
    sql.split_whitespace()
        .next()
        .is_some_and(|v| v.eq_ignore_ascii_case("COMMENT"))
}
impl TableDesign {
    /// Converts old COMMENT statements to comment data without changing the input on failure.
    pub fn migrate_comments(&mut self, scope: &Scope) -> DbResult<()> {
        let mut next = self.clone();
        let mut seen = HashSet::new();
        let mut indexes = Vec::new();
        for sql in &self.indexes {
            if !is_comment(sql) {
                indexes.push(sql.clone());
                continue;
            }
            let (column, mut names, value) = decode(sql)?;
            let field = if column {
                Some(
                    names
                        .pop()
                        .ok_or_else(|| DbError::msg("Missing comment field."))?,
                )
            } else {
                None
            };
            let table = names
                .pop()
                .ok_or_else(|| DbError::msg("Missing comment table."))?;
            if table != self.name
                || names.len() > 1
                || names
                    .first()
                    .is_some_and(|s| s != scope.schema.as_deref().unwrap_or("public"))
            {
                return Err(DbError::msg(
                    "A legacy comment targets another table; resolve it in Advanced.",
                ));
            }
            let target = if let Some(field) = field {
                let c = next
                    .columns
                    .iter_mut()
                    .find(|c| c.name == field || c.original.as_deref() == Some(&field))
                    .ok_or_else(|| {
                        DbError::msg(
                            "A legacy comment targets a missing field; resolve it in Advanced.",
                        )
                    })?;
                if !c.comment.is_empty() {
                    return Err(DbError::msg(
                        "Duplicate column comment; resolve it in Advanced.",
                    ));
                }
                c.comment = value;
                Some(c.id)
            } else {
                if !next.comment.is_empty() {
                    return Err(DbError::msg(
                        "Duplicate table comment; resolve it in Advanced.",
                    ));
                }
                next.comment = value;
                None
            };
            if !seen.insert(target) {
                return Err(DbError::msg(
                    "Duplicate legacy comment; resolve it in Advanced.",
                ));
            }
        }
        next.indexes = indexes;
        *self = next;
        Ok(())
    }
}
fn value(comment: &str) -> String {
    if comment.is_empty() {
        "NULL".into()
    } else {
        format!("E'{}'", comment.replace('\\', "\\\\").replace('\'', "''"))
    }
}
pub(super) fn statements(
    scope: &Scope,
    before: Option<&TableDesign>,
    after: &TableDesign,
) -> Vec<String> {
    let d = Dialect::Postgres;
    let table = d.qualified(scope, &after.name);
    let mut out = Vec::new();
    if before.map(|b| b.comment.as_str()).unwrap_or("") != after.comment {
        out.push(format!(
            "COMMENT ON TABLE {table} IS {}",
            value(&after.comment)
        ));
    }
    for c in &after.columns {
        let old = c
            .original
            .as_ref()
            .and_then(|name| before.and_then(|b| b.columns.iter().find(|c| &c.name == name)))
            .map(|c| c.comment.as_str())
            .unwrap_or("");
        if old != c.comment {
            out.push(format!(
                "COMMENT ON COLUMN {table}.{} IS {}",
                d.quote_ident(&c.name),
                value(&c.comment)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope::new("test", "db", Some("schema.with.dot".into()))
    }
    fn baseline() -> TableSnapshot {
        let mut design = TableDesign::new();
        design.name = "table\"name".into();
        design.comment = "old table".into();
        design.columns[0].original = Some("id".into());
        design.columns[0].comment = "old column".into();
        TableSnapshot {
            design,
            ddl: String::new(),
            triggers: vec![],
        }
    }
    #[test]
    fn comments_follow_column_identity_and_are_removed_without_dropping_indexes() {
        let old = baseline();
        let mut next = old.design.clone();
        next.columns[0].name = "new.name".into();
        next.columns[0].comment = "中文 O'Brien \\ path; -- text\nnext".into();
        next.comment.clear();
        let sql = plan(Dialect::Postgres, &scope(), Some(&old), &next).unwrap();
        assert!(sql[0].contains("RENAME COLUMN"));
        assert_eq!(
            sql[1],
            "COMMENT ON TABLE \"schema.with.dot\".\"table\"\"name\" IS NULL"
        );
        assert!(sql[2].contains(".\"new.name\" IS E'中文 O''Brien \\\\ path; -- text\nnext'"));
        let mut unchanged = old.design.clone();
        unchanged.columns[0].name = "renamed".into();
        assert_eq!(
            plan(Dialect::Postgres, &scope(), Some(&old), &unchanged)
                .unwrap()
                .len(),
            1
        );
        let mut deleted = old.design.clone();
        deleted.columns.remove(0);
        deleted
            .columns
            .push(DesignColumn::new(9, "replacement", "INTEGER"));
        let sql = plan(Dialect::Postgres, &scope(), Some(&old), &deleted).unwrap();
        assert!(!sql.iter().any(|s| s.starts_with("COMMENT ON COLUMN")));
    }
    #[test]
    fn legacy_comments_migrate_atomically_and_reject_ambiguous_targets() {
        let mut old = baseline().design;
        old.comment.clear();
        old.columns[0].comment.clear();
        old.indexes=vec!["COMMENT ON TABLE \"schema.with.dot\".\"table\"\"name\" IS '旧表'".into(),"COMMENT ON COLUMN \"schema.with.dot\".\"table\"\"name\".\"id\" IS E'O''Brien \\\\ path'".into()];
        old.migrate_comments(&scope()).unwrap();
        assert!(old.indexes.is_empty());
        assert_eq!(old.comment, "旧表");
        assert_eq!(old.columns[0].comment, "O'Brien \\ path");
        let mut bad = baseline().design;
        bad.indexes.push("COMMENT ON TABLE elsewhere IS 'x'".into());
        let original = bad.clone();
        assert!(bad.migrate_comments(&scope()).is_err());
        assert_eq!(bad, original);
        let mut duplicate = TableDesign::new();
        duplicate.name = "t".into();
        duplicate.indexes = vec![
            "COMMENT ON TABLE t IS NULL".into(),
            "COMMENT ON TABLE t IS ''".into(),
        ];
        assert!(
            duplicate
                .migrate_comments(&Scope::new("test", "db", None))
                .is_err()
        );
    }
    #[test]
    fn new_table_comments_precede_owner_transfer_and_reject_nul() {
        let mut design = baseline().design;
        design.owner = "new_owner".into();
        let sql = plan(Dialect::Postgres, &scope(), None, &design).unwrap();
        assert!(sql[1].starts_with("COMMENT ON TABLE"));
        assert!(sql[2].starts_with("COMMENT ON COLUMN"));
        assert!(sql.last().unwrap().contains("OWNER TO"));
        design.comment = "bad\0comment".into();
        assert!(plan(Dialect::Postgres, &scope(), None, &design).is_err());
    }
}
