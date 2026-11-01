//! Structured index and constraint forms. Unknown syntax remains an untouched advanced entry.
use super::*;
use navidog_sql::span::{self as sql_span, SpanKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Index,
    ForeignKey,
    Check,
    Unique,
    /// PostgreSQL `EXCLUDE` constraint.
    Exclude,
    Primary,
    Advanced,
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DesignEntry {
    pub kind: EntryKind,
    pub name: String,
    pub fields: String,
    pub method: String,
    pub comment: String,
    pub reference_schema: String,
    pub reference_table: String,
    pub reference_fields: String,
    pub on_delete: String,
    pub on_update: String,
    pub expression: String,
    pub unique: bool,
}
impl DesignEntry {
    pub fn new(kind: EntryKind) -> Self {
        Self {
            kind,
            name: String::new(),
            fields: String::new(),
            method: String::new(),
            comment: String::new(),
            reference_schema: String::new(),
            reference_table: String::new(),
            reference_fields: String::new(),
            on_delete: String::new(),
            on_update: String::new(),
            expression: String::new(),
            unique: false,
        }
    }
    pub fn sql(
        &self,
        dialect: &dyn Dialect,
        scope: &Scope,
        table: &str,
        standalone: bool,
    ) -> DbResult<String> {
        for s in [
            &self.fields,
            &self.method,
            &self.expression,
            &self.reference_fields,
        ] {
            parse::fragment(s, dialect)?;
        }
        let q = |s: &str| dialect.quote_ident(s);
        let name = if self.name.is_empty() {
            String::new()
        } else {
            format!("CONSTRAINT {} ", q(&self.name))
        };
        Ok(match self.kind {
            EntryKind::Primary => format!("{name}PRIMARY KEY ({})", self.fields),
            EntryKind::Unique => format!("{name}UNIQUE ({})", self.fields),
            EntryKind::Check => format!("{name}CHECK ({})", self.expression),
            EntryKind::Exclude => {
                if self.fields.trim().is_empty() {
                    return Err(DbError::msg(
                        "An exclusion constraint needs at least one element.",
                    ));
                }
                let method = if self.method.trim().is_empty() {
                    "gist"
                } else {
                    self.method.trim()
                };
                if !method
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return Err(DbError::msg("Invalid index method."));
                }
                // The constraints editor stores one constraint per line. A break
                // inside the element list would be read back as another constraint.
                let fields = navidog_sql::completion::one_line(self.fields.trim());
                let expression = navidog_sql::completion::one_line(self.expression.trim());
                let mut sql = format!("{name}EXCLUDE USING {method} ({fields})");
                if !expression.is_empty() {
                    sql.push_str(&format!(" WHERE ({expression})"));
                }
                sql
            }
            EntryKind::ForeignKey => {
                let local = split_field_list(&self.fields, dialect)?;
                let referenced = split_field_list(&self.reference_fields, dialect)?;
                if self.reference_table.is_empty()
                    || local.is_empty()
                    || local.len() != referenced.len()
                {
                    return Err(DbError::msg(
                        "Foreign key requires a referenced table and matching field counts.",
                    ));
                }
                let target = if self.reference_schema.is_empty() {
                    q(&self.reference_table)
                } else {
                    format!("{}.{}", q(&self.reference_schema), q(&self.reference_table))
                };
                let mut sql = format!(
                    "{name}FOREIGN KEY ({}) REFERENCES {target} ({})",
                    self.fields, self.reference_fields
                );
                for (key, action) in [("DELETE", &self.on_delete), ("UPDATE", &self.on_update)] {
                    if !action.is_empty() {
                        if ![
                            "NO ACTION",
                            "RESTRICT",
                            "CASCADE",
                            "SET NULL",
                            "SET DEFAULT",
                        ]
                        .iter()
                        .any(|a| action.eq_ignore_ascii_case(a))
                        {
                            return Err(DbError::msg("Invalid foreign key action."));
                        }
                        sql.push_str(&format!(" ON {key} {}", action.to_ascii_uppercase()));
                    }
                }
                sql
            }
            EntryKind::Index => {
                let unique = if self.unique { "UNIQUE " } else { "" };
                let method = if self.method.is_empty() {
                    String::new()
                } else {
                    if !self
                        .method
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(DbError::msg("Invalid index method."));
                    }
                    format!(" USING {}", self.method)
                };
                let mut sql = if standalone {
                    format!(
                        "CREATE {unique}INDEX {} ON {}{method} ({})",
                        q(&self.name),
                        dialect.qualified(scope, table),
                        self.fields
                    )
                } else {
                    format!("{unique}KEY {}{method} ({})", q(&self.name), self.fields)
                };
                if !self.expression.trim().is_empty() {
                    sql.push_str(&format!(" WHERE {}", self.expression));
                }
                if !self.comment.is_empty() {
                    if !dialect.features().supports_index_comment {
                        return Err(DbError::msg(
                            "Index comments require a separate comment definition on this database.",
                        ));
                    }
                    sql.push_str(&format!(
                        " COMMENT '{}'",
                        self.comment.replace('\\', "\\\\").replace('\'', "''")
                    ));
                }
                sql
            }
            EntryKind::Advanced => {
                return Err(DbError::msg(
                    "This definition requires the advanced editor.",
                ));
            }
        })
    }
}
pub fn split_field_list(s: &str, d: &dyn Dialect) -> DbResult<Vec<String>> {
    let mut out = Vec::new();
    let mut start = 0;
    for end in parse::positions(s, b',', d)? {
        out.push(s[start..end].trim().to_string());
        start = end + 1;
    }
    if !s[start..].trim().is_empty() {
        out.push(s[start..].trim().to_string());
    }
    Ok(out)
}
fn take<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let s = s.trim_start();
    if s.get(..word.len())?.eq_ignore_ascii_case(word)
        && s.get(word.len()..word.len() + 1)
            .is_none_or(|c| c.chars().all(|c| c.is_whitespace() || c == '('))
    {
        Some(s[word.len()..].trim_start())
    } else {
        None
    }
}
/// The parenthesized group at the start of `s`: its inner text and the rest.
fn group<'a>(s: &'a str, d: &dyn Dialect) -> Option<(String, &'a str)> {
    let s = s.trim_start();
    if !s.starts_with('(') {
        return None;
    }
    let close = sql_span::spans(s, d.lex())
        .into_iter()
        .find(|sp| sp.kind == SpanKind::CloseParen && sp.depth == 1)?
        .range
        .start;
    Some((s[1..close].trim().into(), s[close + 1..].trim()))
}

fn qualified(s: &str) -> Option<(String, String, &str)> {
    let (a, rest) = parse::ident(s).ok()?;
    if let Some(rest) = rest.strip_prefix('.') {
        let (b, rest) = parse::ident(rest).ok()?;
        Some((a, b, rest))
    } else if !s.trim_start().starts_with(['"', '`', '[']) && a.contains('.') {
        let (schema, table) = a.split_once('.')?;
        if table.contains('.') {
            return None;
        }
        Some((schema.into(), table.into(), rest))
    } else {
        Some((String::new(), a, rest))
    }
}
/// Returns None unless all syntax can be represented by the form.
pub fn parse_entry(s: &str, d: &dyn Dialect) -> Option<DesignEntry> {
    let mut rest = s.trim().trim_end_matches(';').trim();
    let mut e = DesignEntry::new(EntryKind::Advanced);
    if let Some(r) = take(rest, "CONSTRAINT") {
        let (name, r) = parse::ident(r).ok()?;
        e.name = name;
        rest = r;
    }
    let standalone = take(rest, "CREATE").is_some();
    if standalone {
        rest = take(rest, "CREATE")?;
    }
    if let Some(r) = take(rest, "UNIQUE") {
        e.unique = true;
        rest = r;
    }
    if let Some(r) = take(rest, "PRIMARY KEY") {
        e.kind = EntryKind::Primary;
        rest = r;
    } else if let Some(r) = take(rest, "FOREIGN KEY") {
        e.kind = EntryKind::ForeignKey;
        rest = r;
    } else if let Some(r) = take(rest, "CHECK") {
        e.kind = EntryKind::Check;
        let (expr, r) = group(r, d)?;
        e.expression = expr;
        return r.is_empty().then_some(e);
    } else if let Some(r) = take(rest, "EXCLUDE") {
        e.kind = EntryKind::Exclude;
        rest = r;
        if let Some(r) = take(rest, "USING") {
            let (method, r) = parse::ident(r).ok()?;
            e.method = method;
            rest = r;
        }
        let (fields, r) = group(rest, d)?;
        e.fields = fields;
        rest = r;
        if let Some(r) = take(rest, "WHERE") {
            let (expr, r) = group(r, d)?;
            e.expression = expr;
            rest = r;
        }
        return rest.is_empty().then_some(e);
    } else if let Some(r) = take(rest, "INDEX").or_else(|| take(rest, "KEY")) {
        e.kind = EntryKind::Index;
        rest = r;
    } else if e.unique {
        e.kind = EntryKind::Unique;
    } else {
        return None;
    }
    if !rest.starts_with('(') && !rest.to_ascii_uppercase().starts_with("USING ") {
        let (name, r) = parse::ident(rest).ok()?;
        e.name = name;
        rest = r;
    }
    if standalone {
        let r = take(rest, "ON")?;
        let (_, _, r) = qualified(r)?;
        rest = r;
    }
    if let Some(r) = take(rest, "USING") {
        let (m, r) = parse::ident(r).ok()?;
        e.method = m;
        rest = r;
    }
    let (fields, r) = group(rest, d)?;
    e.fields = fields;
    rest = r;
    if e.kind == EntryKind::ForeignKey {
        let r = take(rest, "REFERENCES")?;
        let (schema, table, r) = qualified(r)?;
        e.reference_schema = schema;
        e.reference_table = table;
        let (fields, r) = group(r, d)?;
        e.reference_fields = fields;
        rest = r;
        for _ in 0..2 {
            let Some(r) = take(rest, "ON") else { break };
            let (update, r) = if let Some(r) = take(r, "UPDATE") {
                (true, r)
            } else {
                (false, take(r, "DELETE")?)
            };
            let action = [
                "NO ACTION",
                "RESTRICT",
                "CASCADE",
                "SET NULL",
                "SET DEFAULT",
            ]
            .into_iter()
            .find(|a| take(r, a).is_some())?;
            rest = take(r, action)?;
            if update {
                e.on_update = action.into();
            } else {
                e.on_delete = action.into();
            }
        }
    }
    if let Some(r) = take(rest, "USING") {
        let (m, r) = parse::ident(r).ok()?;
        e.method = m;
        rest = r;
    }
    if let Some(r) = take(rest, "COMMENT") {
        let literal = sql_span::spans(r, d.lex())
            .into_iter()
            .next()
            .filter(|sp| sp.kind == SpanKind::String && sp.range.start == 0)?;
        let body = &r[literal.range.start + 1..literal.range.end - 1];
        e.comment = body.replace("''", "'").replace("\\'", "'");
        rest = r[literal.range.end..].trim();
    }
    if let Some(r) = take(rest, "WHERE") {
        e.expression = r.into();
        rest = "";
    }
    rest.trim().is_empty().then_some(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forms_round_trip_and_unknown_clauses_stay_advanced() {
        let scope = Scope::new("c", "main", None);
        for sql in [
            "CREATE UNIQUE INDEX \"idx\" ON \"main\".\"t\" (lower(name)) WHERE id > 0",
            "CONSTRAINT fk FOREIGN KEY (a, b) REFERENCES other (id, code) ON DELETE CASCADE ON UPDATE SET NULL",
            "CONSTRAINT ck CHECK (id > 0)",
            "UNIQUE (name)",
            "PRIMARY KEY (id)",
        ] {
            let e = parse_entry(sql, &navidog_sqlite::SqliteDialect).unwrap();
            let result = e
                .sql(
                    &navidog_sqlite::SqliteDialect,
                    &scope,
                    "t",
                    sql.starts_with("CREATE"),
                )
                .unwrap();
            assert!(
                parse_entry(&result, &navidog_sqlite::SqliteDialect).is_some(),
                "{result}"
            );
        }
        let fk = parse_entry(
            "CONSTRAINT fk FOREIGN KEY (a) REFERENCES public.parent (id)",
            &navidog_sqlite::SqliteDialect,
        )
        .unwrap();
        assert_eq!(fk.reference_schema, "public");
        assert_eq!(fk.reference_table, "parent");
        let fk = parse_entry(
            "CONSTRAINT fk FOREIGN KEY (a) REFERENCES \"literal.dot\" (id)",
            &navidog_sqlite::SqliteDialect,
        )
        .unwrap();
        assert!(fk.reference_schema.is_empty());
        assert_eq!(fk.reference_table, "literal.dot");
        assert_eq!(
            split_field_list("lower(name), \"a,b\", id", &navidog_sqlite::SqliteDialect)
                .unwrap()
                .len(),
            3
        );
        assert!(
            parse_entry(
                "CONSTRAINT fk FOREIGN KEY (id) REFERENCES p(id) DEFERRABLE INITIALLY DEFERRED",
                &navidog_sqlite::SqliteDialect,
            )
            .is_none()
        );
        let mut exclusion = DesignEntry::new(EntryKind::Exclude);
        exclusion.fields = "id WITH =\n, name WITH =".into();
        exclusion.expression = "id > 0\nAND name <> ''".into();
        let sql = exclusion
            .sql(&navidog_sqlite::SqliteDialect, &scope, "t", false)
            .unwrap();
        assert!(!sql.contains('\n'), "{sql}");
        assert!(sql.contains("id WITH = , name WITH ="), "{sql}");
        assert!(sql.contains("WHERE (id > 0 AND name <> '')"), "{sql}");
    }
}
