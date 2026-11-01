//! MySQL and MariaDB SQL dialect. Both drivers share this implementation.

use navidog_model::object::{ObjectKind, Scope};
use navidog_sql::{
    ColumnType, ConstraintKind, ContextSelection, Dialect, DialectFeatures, ExplainStyle, Lex,
    TableOptionStyle, TypeArgs,
};

#[derive(Debug)]
pub struct MysqlDialect;

impl Dialect for MysqlDialect {
    fn id(&self) -> &'static str {
        "mysql"
    }
    fn lex(&self) -> Lex {
        LEX
    }
    fn quote_ident(&self, name: &str) -> String {
        format!("`{}`", name.replace('`', "``"))
    }
    fn string_literal(&self, value: &str) -> String {
        let escaped = value.replace('\'', "''");
        format!("'{}'", escaped.replace('\\', "\\\\"))
    }
    fn qualify_prefix<'a>(&self, scope: &'a Scope) -> Option<&'a str> {
        Some(scope.database.as_str())
    }
    fn object_kinds(&self) -> &'static [ObjectKind] {
        &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::Function,
            ObjectKind::Event,
            ObjectKind::Tablespace,
        ]
    }
    fn lists_kind(&self, kind: ObjectKind, driver: &str, version: &str) -> bool {
        if kind == ObjectKind::Sequence {
            return navidog_sql::definition::mariadb_sequences(driver, version);
        }
        self.object_kinds().contains(&kind)
    }
    fn definition_family(&self) -> Option<navidog_sql::definition::DefinitionFamily> {
        Some(navidog_sql::definition::DefinitionFamily::Mysql)
    }
    fn trigger_definitions_sql(&self, scope: &Scope, table: &str) -> Option<String> {
        let database = self.string_literal(&scope.database);
        let table = self.string_literal(table);
        // Names are quoted in SQL so a backtick in the identifier cannot break the statement.
        Some(format!(
            "SELECT CONCAT( \
                'CREATE TRIGGER `', REPLACE(TRIGGER_NAME, '`', '``'), '` ', \
                ACTION_TIMING, ' ', EVENT_MANIPULATION, \
                ' ON `', REPLACE(EVENT_OBJECT_TABLE, '`', '``'), '` FOR EACH ', \
                ACTION_ORIENTATION, ' ', ACTION_STATEMENT) \
             FROM information_schema.TRIGGERS \
             WHERE EVENT_OBJECT_SCHEMA = {database} AND EVENT_OBJECT_TABLE = {table} \
             ORDER BY TRIGGER_NAME, EVENT_MANIPULATION"
        ))
    }
    fn definition_statements(
        &self,
        scope: &navidog_model::object::Scope,
        draft: &navidog_sql::definition::DefinitionDraft,
    ) -> Result<Vec<String>, String> {
        navidog_sql::definition::plan(
            self,
            navidog_sql::definition::DefinitionFamily::Mysql,
            scope,
            draft,
        )
    }
    fn begin_transaction(&self) -> &'static str {
        "START TRANSACTION"
    }
    fn is_system_database(&self, name: &str) -> bool {
        matches!(
            name,
            "information_schema" | "mysql" | "performance_schema" | "sys"
        )
    }
    fn is_system_schema(&self, _: &str) -> bool {
        false
    }
    fn extra_keywords(&self) -> &'static [&'static str] {
        crate::words::KEYWORDS
    }
    fn extra_functions(&self) -> &'static [&'static str] {
        crate::words::FUNCTIONS
    }
    fn ident_quote(&self) -> char {
        '`'
    }
    fn drop_constraint_clause(&self, kind: ConstraintKind, name: &str) -> Option<String> {
        Some(match kind {
            ConstraintKind::Primary => "DROP PRIMARY KEY".into(),
            ConstraintKind::ForeignKey => format!("DROP FOREIGN KEY {}", self.quote_ident(name)),
            ConstraintKind::Index => format!("DROP INDEX {}", self.quote_ident(name)),
            // MySQL drops UNIQUE and CHECK constraints through DROP INDEX / DROP CHECK, which
            // this designer does not model; an EXCLUDE constraint does not exist there.
            ConstraintKind::Unique | ConstraintKind::Check | ConstraintKind::Exclude => {
                return None;
            }
        })
    }
    fn context_selection(&self) -> ContextSelection {
        ContextSelection::Database
    }
    fn features(&self) -> DialectFeatures {
        DialectFeatures {
            limit_unkeyed_writes: true,
            supports_column_position: true,
            standalone_indexes: false,
            supports_zerofill: true,
            supports_on_update: true,
            transactional_ddl: false,
            drop_index_alters_table: true,
            rewrites_columns: true,
            inline_table_options: true,
            supports_profile: true,
            counts_changed_rows: true,
            switches_database: true,
            column_comment_inline: true,
            supports_index_comment: true,
            supports_column_charset: true,
            supports_unsigned: true,
            supports_index_expression: false,
            supports_partitions: true,
            ..DialectFeatures::default()
        }
    }
    fn table_option_style(&self) -> TableOptionStyle {
        TableOptionStyle::Clauses
    }
    fn table_option_keys(&self) -> &'static [&'static str] {
        crate::words::TABLE_OPTIONS
    }
    fn explain_style(&self) -> ExplainStyle {
        ExplainStyle::TableAndJson
    }
    fn column_types(&self) -> &'static [ColumnType] {
        COLUMN_TYPES
    }
    fn default_column_type(&self) -> &'static str {
        "int"
    }
    fn added_column_type(&self) -> &'static str {
        "text"
    }
    fn index_methods(&self) -> &'static [&'static str] {
        &["BTREE", "HASH"]
    }
    fn session_context_sql(&self) -> Option<&'static str> {
        Some("SELECT DATABASE()")
    }
    fn prepare_database_sql(&self, database: &str) -> Option<String> {
        (!database.is_empty()).then(|| format!("USE {}", self.quote_ident(database)))
    }
    fn console_prompt(&self, _: &str, _: bool, continuation: bool) -> String {
        if continuation {
            "    ->".into()
        } else {
            "mysql>".into()
        }
    }
    fn rename_object(
        &self,
        scope: &Scope,
        kind: ObjectKind,
        old: &str,
        new: &str,
    ) -> Option<String> {
        match kind {
            ObjectKind::Table | ObjectKind::View => Some(format!(
                "RENAME TABLE {} TO {}",
                self.qualified(scope, old),
                self.qualified(scope, new)
            )),
            _ => None,
        }
    }
    fn duplicate_table(
        &self,
        scope: &Scope,
        old: &str,
        new: &str,
        with_data: bool,
        _: Option<&str>,
    ) -> Option<Vec<String>> {
        let (from, to) = (self.qualified(scope, old), self.qualified(scope, new));
        let mut out = vec![format!("CREATE TABLE {to} LIKE {from}")];
        if with_data {
            out.push(format!("INSERT INTO {to} SELECT * FROM {from}"));
        }
        Some(out)
    }
}

const LEX: Lex = Lex::MYSQL;

// Names are what MySQL 8.4 prints in SHOW CREATE TABLE. Synonyms collapse onto those names.
const COLUMN_TYPES: &[ColumnType] = &[
    ColumnType {
        name: "int",
        synonyms: &["INTEGER", "INT"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "bigint",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "smallint",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "tinyint",
        synonyms: &["BOOLEAN", "BOOL"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "varchar",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "char",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "text",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "decimal",
        synonyms: &["NUMERIC"],
        args: TypeArgs::LengthAndScale,
    },
    ColumnType {
        name: "double",
        synonyms: &["DOUBLE PRECISION", "REAL"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "date",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "time",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "timestamp",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "blob",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "json",
        synonyms: &[],
        args: TypeArgs::None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_has_no_master_table() {
        assert!(MysqlDialect.master_table_sql("testdb").is_none());
    }
}
