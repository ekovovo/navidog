//! SQL dialects: identifier quoting, literals, paged queries and SQL generation for grid change
//! sets (21-table-viewer.md §2).

use navidog_core::object::{ObjectKind, Scope};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    MySql,
    Postgres,
    Sqlite,
}

/// Values of a row keyed by column name; `None` is NULL.
pub type Values = Vec<(String, Option<String>)>;

/// Locates a row: only the primary key columns when there is a primary key, otherwise the whole
/// row's old values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowKey(pub Values);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowUpdate {
    pub key: RowKey,
    pub values: Values,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub updates: Vec<RowUpdate>,
    pub inserts: Vec<Values>,
    pub deletes: Vec<RowKey>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.inserts.is_empty() && self.deletes.is_empty()
    }
}

impl Dialect {
    pub fn quote_ident(self, name: &str) -> String {
        match self {
            Self::MySql => format!("`{}`", name.replace('`', "``")),
            Self::Postgres | Self::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    /// String literal; under MySQL's default sql_mode backslash is an escape character, so it is
    /// escaped too.
    pub fn string_literal(self, value: &str) -> String {
        let escaped = value.replace('\'', "''");
        match self {
            Self::MySql => format!("'{}'", escaped.replace('\\', "\\\\")),
            Self::Postgres | Self::Sqlite => format!("'{escaped}'"),
        }
    }

    pub fn literal(self, value: Option<&str>) -> String {
        match value {
            None => "NULL".into(),
            Some(v) => self.string_literal(v),
        }
    }

    /// Qualified table name. A PostgreSQL session is already connected to a specific database,
    /// so only the schema is prefixed.
    pub fn qualified(self, scope: &Scope, name: &str) -> String {
        let prefix = match self {
            Self::MySql | Self::Sqlite => Some(scope.database.as_str()),
            Self::Postgres => scope.schema.as_deref(),
        };
        match prefix {
            Some(p) => format!("{}.{}", self.quote_ident(p), self.quote_ident(name)),
            None => self.quote_ident(name),
        }
    }

    pub fn object_kinds(self) -> &'static [ObjectKind] {
        match self {
            Self::MySql => &[
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::Function,
                ObjectKind::Event,
            ],
            Self::Postgres => &[
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::MaterializedView,
                ObjectKind::Function,
                ObjectKind::Sequence,
            ],
            Self::Sqlite => &[
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::Index,
                ObjectKind::Trigger,
            ],
        }
    }

    pub fn begin_transaction(self) -> &'static str {
        match self {
            Self::MySql => "START TRANSACTION",
            Self::Postgres | Self::Sqlite => "BEGIN",
        }
    }

    /// The `DROP` keyword of a kind, or `None` when it cannot be dropped by name alone
    /// (functions need their signature).
    pub fn drop_keyword(self, kind: ObjectKind) -> Option<&'static str> {
        Some(match kind {
            ObjectKind::Table => "TABLE",
            ObjectKind::View => "VIEW",
            ObjectKind::MaterializedView => "MATERIALIZED VIEW",
            ObjectKind::Event => "EVENT",
            ObjectKind::Trigger if self == Self::Sqlite => "TRIGGER",
            ObjectKind::Index if self == Self::Sqlite => "INDEX",
            ObjectKind::Sequence => "SEQUENCE",
            _ => return None,
        })
    }

    /// `RENAME` statement for an object, or `None` when the dialect cannot rename that kind
    /// (PostgreSQL functions need their argument signature, SQLite views cannot be renamed).
    pub fn rename_object(
        self,
        scope: &Scope,
        kind: ObjectKind,
        old: &str,
        new: &str,
    ) -> Option<String> {
        let (from, to) = (self.qualified(scope, old), self.quote_ident(new));
        Some(match (self, kind) {
            (Self::MySql, ObjectKind::Table | ObjectKind::View) => {
                format!("RENAME TABLE {from} TO {}", self.qualified(scope, new))
            }
            (Self::Postgres, ObjectKind::Table) | (Self::Sqlite, ObjectKind::Table) => {
                format!("ALTER TABLE {from} RENAME TO {to}")
            }
            (Self::Postgres, ObjectKind::View) => format!("ALTER VIEW {from} RENAME TO {to}"),
            (Self::Postgres, ObjectKind::MaterializedView) => {
                format!("ALTER MATERIALIZED VIEW {from} RENAME TO {to}")
            }
            (Self::Postgres, ObjectKind::Sequence) => {
                format!("ALTER SEQUENCE {from} RENAME TO {to}")
            }
            _ => return None,
        })
    }

    /// Statements that copy a table's structure (and optionally its data) to `new`.
    /// SQLite has no `CREATE TABLE … LIKE`, so its original `CREATE TABLE` statement (`ddl`) is
    /// rewritten instead, which keeps keys and constraints.
    pub fn duplicate_table(
        self,
        scope: &Scope,
        old: &str,
        new: &str,
        with_data: bool,
        ddl: Option<&str>,
    ) -> Option<Vec<String>> {
        let (from, to) = (self.qualified(scope, old), self.qualified(scope, new));
        let create = match self {
            Self::MySql => format!("CREATE TABLE {to} LIKE {from}"),
            Self::Postgres => format!("CREATE TABLE {to} (LIKE {from} INCLUDING ALL)"),
            Self::Sqlite => rename_create_table(ddl?, old, &to)?,
        };
        let mut out = vec![create];
        if with_data {
            out.push(format!("INSERT INTO {to} SELECT * FROM {from}"));
        }
        Some(out)
    }

    /// System databases, hidden in the navigator unless "Show Hidden Items" is on.
    pub fn is_system_database(self, name: &str) -> bool {
        match self {
            Self::MySql => matches!(
                name,
                "information_schema" | "mysql" | "performance_schema" | "sys"
            ),
            Self::Postgres | Self::Sqlite => false,
        }
    }

    /// System schemas, hidden in the navigator unless "Show Hidden Items" is on.
    pub fn is_system_schema(self, name: &str) -> bool {
        self == Self::Postgres && matches!(name, "pg_catalog" | "information_schema")
    }

    /// Whether there is a schema level (an extra navigator tree level between database and
    /// object category).
    pub fn has_schemas(self) -> bool {
        self == Self::Postgres
    }

    /// Paged query for the table viewer. `filter` and `order` are the user-entered WHERE /
    /// ORDER BY clauses (without the keywords).
    pub fn select_page(
        self,
        scope: &Scope,
        table: &str,
        filter: &str,
        order: &str,
        limit: Option<u64>,
        offset: u64,
    ) -> String {
        let mut sql = format!("SELECT * FROM {}", self.qualified(scope, table));
        if !filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", filter.trim()));
        }
        if !order.trim().is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.trim()));
        }
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
        }
        sql
    }

    pub fn count(self, scope: &Scope, table: &str, filter: &str) -> String {
        let mut sql = format!("SELECT COUNT(*) FROM {}", self.qualified(scope, table));
        if !filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", filter.trim()));
        }
        sql
    }

    fn assignments(self, values: &Values, sep: &str) -> String {
        values
            .iter()
            .map(|(c, v)| match (sep, v) {
                (" AND ", None) => format!("{} IS NULL", self.quote_ident(c)),
                _ => format!("{} = {}", self.quote_ident(c), self.literal(v.as_deref())),
            })
            .collect::<Vec<_>>()
            .join(sep)
    }

    /// Turn the grid's change set into SQL statements (order: deletes, updates, inserts).
    /// When `has_primary_key` is false, rows are located by their whole old values, and MySQL
    /// adds `LIMIT 1` to avoid affecting multiple rows.
    pub fn render_changes(
        self,
        scope: &Scope,
        table: &str,
        changes: &ChangeSet,
        has_primary_key: bool,
    ) -> Vec<String> {
        let target = self.qualified(scope, table);
        let limit = if self == Self::MySql && !has_primary_key {
            " LIMIT 1"
        } else {
            ""
        };
        let mut out = Vec::new();
        for key in &changes.deletes {
            out.push(format!(
                "DELETE FROM {target} WHERE {}{limit}",
                self.assignments(&key.0, " AND ")
            ));
        }
        for u in &changes.updates {
            if u.values.is_empty() {
                continue;
            }
            out.push(format!(
                "UPDATE {target} SET {} WHERE {}{limit}",
                self.assignments(&u.values, ", "),
                self.assignments(&u.key.0, " AND ")
            ));
        }
        for row in &changes.inserts {
            let cols: Vec<_> = row.iter().map(|(c, _)| self.quote_ident(c)).collect();
            let vals: Vec<_> = row
                .iter()
                .map(|(_, v)| self.literal(v.as_deref()))
                .collect();
            out.push(format!(
                "INSERT INTO {target} ({}) VALUES ({})",
                cols.join(", "),
                vals.join(", ")
            ));
        }
        out
    }

    /// "Copy As > Insert Statement".
    pub fn insert_statements(
        self,
        scope: &Scope,
        table: &str,
        columns: &[String],
        rows: &[Vec<Option<String>>],
    ) -> String {
        let target = self.qualified(scope, table);
        let cols: Vec<_> = columns.iter().map(|c| self.quote_ident(c)).collect();
        rows.iter()
            .map(|r| {
                let vals: Vec<_> = r.iter().map(|v| self.literal(v.as_deref())).collect();
                format!(
                    "INSERT INTO {target} ({}) VALUES ({});",
                    cols.join(", "),
                    vals.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// "Copy As > Update Statement": rows are located by `key_columns`; the other columns become
    /// assignments.
    pub fn update_statements(
        self,
        scope: &Scope,
        table: &str,
        columns: &[String],
        key_columns: &[String],
        rows: &[Vec<Option<String>>],
    ) -> String {
        let target = self.qualified(scope, table);
        rows.iter()
            .map(|r| {
                let (keys, sets): (Values, Values) = columns
                    .iter()
                    .cloned()
                    .zip(r.iter().cloned())
                    .partition(|(c, _)| key_columns.contains(c));
                let keys = if keys.is_empty() { sets.clone() } else { keys };
                format!(
                    "UPDATE {target} SET {} WHERE {};",
                    self.assignments(&sets, ", "),
                    self.assignments(&keys, " AND ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Replaces the table name in a `CREATE TABLE` statement; the name may be bare or quoted with
/// `"`, `` ` `` or `[]`, and follow `IF NOT EXISTS`.
fn rename_create_table(ddl: &str, old: &str, new_qualified: &str) -> Option<String> {
    let head_len = {
        let upper = ddl.to_ascii_uppercase();
        let start = upper.find("CREATE TABLE")? + "CREATE TABLE".len();
        let rest = &upper[start..];
        let skipped = rest.len() - rest.trim_start().len();
        let after_if = rest
            .trim_start()
            .strip_prefix("IF NOT EXISTS")
            .map(|r| rest.len() - r.len());
        let pos = after_if.unwrap_or(skipped);
        let rest = &upper[start + pos..];
        start + pos + (rest.len() - rest.trim_start().len())
    };
    let tail = &ddl[head_len..];
    let forms = [
        format!("\"{old}\""),
        format!("`{old}`"),
        format!("[{old}]"),
        old.to_string(),
    ];
    let name = forms.iter().find(|f| tail.starts_with(f.as_str()))?;
    Some(format!(
        "{}{new_qualified}{}",
        &ddl[..head_len],
        &tail[name.len()..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_keywords_per_dialect() {
        assert_eq!(
            Dialect::MySql.drop_keyword(ObjectKind::Table),
            Some("TABLE")
        );
        assert_eq!(
            Dialect::Sqlite.drop_keyword(ObjectKind::Index),
            Some("INDEX")
        );
        assert_eq!(Dialect::MySql.drop_keyword(ObjectKind::Index), None);
        assert_eq!(Dialect::Postgres.drop_keyword(ObjectKind::Function), None);
    }

    #[test]
    fn rename_and_duplicate_statements() {
        let s = Scope::new("c", "app", None);
        let my = Dialect::MySql
            .rename_object(&s, ObjectKind::Table, "a", "b")
            .unwrap();
        assert_eq!(my, "RENAME TABLE `app`.`a` TO `app`.`b`");
        let pg = Scope::new("c", "db", Some("public".into()));
        let v = Dialect::Postgres
            .rename_object(&pg, ObjectKind::View, "v", "w")
            .unwrap();
        assert_eq!(v, "ALTER VIEW \"public\".\"v\" RENAME TO \"w\"");
        assert!(
            Dialect::Postgres
                .rename_object(&pg, ObjectKind::Function, "f", "g")
                .is_none()
        );
        assert!(
            Dialect::Sqlite
                .rename_object(&s, ObjectKind::View, "v", "w")
                .is_none()
        );

        let dup = Dialect::Postgres
            .duplicate_table(&pg, "t", "t_copy1", true, None)
            .unwrap();
        assert_eq!(
            dup,
            [
                "CREATE TABLE \"public\".\"t_copy1\" (LIKE \"public\".\"t\" INCLUDING ALL)",
                "INSERT INTO \"public\".\"t_copy1\" SELECT * FROM \"public\".\"t\"",
            ]
        );
        let ddl = "CREATE TABLE IF NOT EXISTS \"actor\" (id INTEGER PRIMARY KEY)";
        let lite = Dialect::Sqlite
            .duplicate_table(&s, "actor", "actor_copy1", false, Some(ddl))
            .unwrap();
        assert_eq!(
            lite,
            ["CREATE TABLE IF NOT EXISTS \"app\".\"actor_copy1\" (id INTEGER PRIMARY KEY)"]
        );
        let bare = rename_create_table("create table actor(id)", "actor", "\"x\"").unwrap();
        assert_eq!(bare, "create table \"x\"(id)");
    }

    fn scope(schema: Option<&str>) -> Scope {
        Scope::new("c", "app", schema.map(String::from))
    }

    #[test]
    fn quoting_and_literals() {
        assert_eq!(Dialect::MySql.quote_ident("a`b"), "`a``b`");
        assert_eq!(Dialect::Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(Dialect::MySql.literal(Some(r"it's \n")), r"'it''s \\n'");
        assert_eq!(Dialect::Postgres.literal(Some(r"it's \n")), r"'it''s \n'");
        assert_eq!(Dialect::Sqlite.literal(None), "NULL");
        assert_eq!(Dialect::MySql.qualified(&scope(None), "t"), "`app`.`t`");
        assert_eq!(
            Dialect::Postgres.qualified(&scope(Some("public")), "t"),
            "\"public\".\"t\""
        );
    }

    #[test]
    fn paging_and_count() {
        let s = scope(None);
        assert_eq!(
            Dialect::Sqlite.select_page(&s, "t", " id > 1 ", "name DESC", Some(1000), 2000),
            "SELECT * FROM \"app\".\"t\" WHERE id > 1 ORDER BY name DESC LIMIT 1000 OFFSET 2000"
        );
        assert_eq!(
            Dialect::MySql.select_page(&s, "t", "", "", None, 0),
            "SELECT * FROM `app`.`t`"
        );
        assert_eq!(
            Dialect::MySql.count(&s, "t", "a = 1"),
            "SELECT COUNT(*) FROM `app`.`t` WHERE a = 1"
        );
    }

    #[test]
    fn renders_changes_with_nulls_and_without_primary_key() {
        let s = scope(None);
        let changes = ChangeSet {
            deletes: vec![RowKey(vec![("id".into(), Some("3".into()))])],
            updates: vec![RowUpdate {
                key: RowKey(vec![("id".into(), Some("1".into())), ("note".into(), None)]),
                values: vec![
                    ("name".into(), Some("O'Brien".into())),
                    ("note".into(), None),
                ],
            }],
            inserts: vec![vec![("id".into(), Some("9".into())), ("name".into(), None)]],
        };
        let sql = Dialect::MySql.render_changes(&s, "t", &changes, false);
        assert_eq!(
            sql,
            [
                "DELETE FROM `app`.`t` WHERE `id` = '3' LIMIT 1",
                "UPDATE `app`.`t` SET `name` = 'O''Brien', `note` = NULL WHERE `id` = '1' AND `note` IS NULL LIMIT 1",
                "INSERT INTO `app`.`t` (`id`, `name`) VALUES ('9', NULL)",
            ]
        );
        let pg = Dialect::Postgres.render_changes(&scope(Some("public")), "t", &changes, true);
        assert!(pg[0].ends_with("WHERE \"id\" = '3'"), "{pg:?}");
    }

    #[test]
    fn copy_as_statements() {
        let s = scope(None);
        let cols = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec![Some("1".into()), Some("a".into())],
            vec![Some("2".into()), None],
        ];
        assert_eq!(
            Dialect::Sqlite.insert_statements(&s, "t", &cols, &rows),
            "INSERT INTO \"app\".\"t\" (\"id\", \"name\") VALUES ('1', 'a');\nINSERT INTO \"app\".\"t\" (\"id\", \"name\") VALUES ('2', NULL);"
        );
        assert_eq!(
            Dialect::Sqlite.update_statements(&s, "t", &cols, &["id".into()], &rows[..1]),
            "UPDATE \"app\".\"t\" SET \"name\" = 'a' WHERE \"id\" = '1';"
        );
    }
}
