//! SQL 方言：标识符引用、字面量、分页查询与网格修改集合的 SQL 生成（21-table-viewer.md 第 2 节）。

use navidog_core::object::{ObjectKind, Scope};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    MySql,
    Postgres,
    Sqlite,
}

/// 一行中按列名给出的值，`None` 为 NULL。
pub type Values = Vec<(String, Option<String>)>;

/// 定位一行：有主键时只含主键列，没有主键时是整行旧值。
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

    /// 字符串字面量；MySQL 默认 sql_mode 下反斜杠是转义符，需要一并转义。
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

    /// 表的限定名。PostgreSQL 会话已连接到具体数据库，只需加模式。
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

    /// 是否有模式层级（导航树在数据库与对象分类之间多一层）。
    pub fn has_schemas(self) -> bool {
        self == Self::Postgres
    }

    /// 表查看器的分页查询。`filter` 与 `order` 是用户输入的 WHERE / ORDER BY 子句（不含关键字）。
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

    /// 把网格的修改集合转成 SQL 语句（顺序：删除、更新、插入）。
    /// `has_primary_key` 为 false 时用整行旧值定位，MySQL 加 `LIMIT 1` 避免影响多行。
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

    /// “复制为 INSERT 语句”。
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

    /// “复制为 UPDATE 语句”：以 `key_columns` 定位，其余列作为赋值。
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

#[cfg(test)]
mod tests {
    use super::*;

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
