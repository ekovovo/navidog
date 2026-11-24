//! Dameng DM8 SQL dialect. Behavior that varies with the server is carried by the probed
//! `case_sensitive` / `compatible_mode` values; feature code branches on
//! [`DialectFeatures`] and capability methods, never on [`Dialect::id`].

use navidog_model::object::{ObjectKind, Scope};
use navidog_sql::{
    ColumnType, ContextSelection, Dialect, DialectFeatures, ExplainStyle, Lex, TableOptionStyle,
    TypeArgs,
};

/// Probe results from a live server (140-dameng.md §3.2). `Default` is `CASE_SENSITIVE() = 1`
/// and `COMPATIBLE_MODE = 0`, used before a connection exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamengDialect {
    /// `CASE_SENSITIVE()`: unquoted identifiers fold to upper case.
    pub case_sensitive: bool,
    /// `COMPATIBLE_MODE` from `V$DM_INI`: 0 (DM), 2 (Oracle), 4 (MySQL).
    pub compatible_mode: u8,
}

impl Default for DamengDialect {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            compatible_mode: 0,
        }
    }
}

impl DamengDialect {
    fn is_identity_definition(definition: &str) -> bool {
        // Match the IDENTITY(seed, increment) clause the server prints, but not a column
        // whose name or default merely contains the word.
        definition.to_ascii_uppercase().contains("IDENTITY(")
    }
}

impl Dialect for DamengDialect {
    fn id(&self) -> &'static str {
        "dameng"
    }
    fn lex(&self) -> Lex {
        Lex {
            // Backticks are only accepted in MySQL compatibility mode (-2007 otherwise).
            backtick_idents: self.compatible_mode == 4,
            hash_comments: false,
            dollar_quotes: false,
            bracket_idents: false,
            backslash_strings: false,
            escape_string_prefix: false,
            double_quote_strings: false,
            nested_block_comments: false,
            delimiter_command: false,
        }
    }
    fn quote_ident(&self, name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
    fn string_literal(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    fn qualify_prefix<'a>(&self, scope: &'a Scope) -> Option<&'a str> {
        scope.schema.as_deref()
    }
    fn object_kinds(&self) -> &'static [ObjectKind] {
        &[
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::Function,
            ObjectKind::Sequence,
            ObjectKind::Trigger,
        ]
    }
    fn definition_family(&self) -> Option<navidog_sql::definition::DefinitionFamily> {
        Some(navidog_sql::definition::DefinitionFamily::Dameng)
    }
    fn definition_statements(
        &self,
        scope: &Scope,
        draft: &navidog_sql::definition::DefinitionDraft,
    ) -> Result<Vec<String>, String> {
        navidog_sql::definition::plan(
            self,
            navidog_sql::definition::DefinitionFamily::Dameng,
            scope,
            draft,
        )
    }
    /// `BEGIN TRANSACTION` is rejected (-2004); the protocol-level begin is driver-internal.
    fn begin_transaction(&self) -> &'static str {
        "START TRANSACTION"
    }
    fn is_system_database(&self, _: &str) -> bool {
        false
    }
    fn is_system_schema(&self, name: &str) -> bool {
        matches!(
            name,
            "SYS" | "SYSDBA" | "SYSAUDITOR" | "SYSSSO" | "CTISYS" | "INFORMATION_SCHEMA"
        )
    }
    fn extra_keywords(&self) -> &'static [&'static str] {
        crate::words::KEYWORDS
    }
    fn extra_functions(&self) -> &'static [&'static str] {
        crate::words::FUNCTIONS
    }
    fn rename_object(
        &self,
        scope: &Scope,
        kind: ObjectKind,
        old: &str,
        new: &str,
    ) -> Option<String> {
        match kind {
            ObjectKind::Table => Some(format!(
                "ALTER TABLE {} RENAME TO {}",
                self.qualified(scope, old),
                self.quote_ident(new)
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
        let filter = if with_data { "" } else { " WHERE 1 = 0" };
        Some(vec![format!(
            "CREATE TABLE {to} AS SELECT * FROM {from}{filter}"
        )])
    }
    fn console_prompt(&self, _: &str, _: bool, continuation: bool) -> String {
        if continuation {
            "  ...> ".into()
        } else {
            "SQL> ".into()
        }
    }
    fn context_selection(&self) -> ContextSelection {
        ContextSelection::Schema
    }
    fn features(&self) -> DialectFeatures {
        DialectFeatures {
            has_schemas: true,
            folds_unquoted_upper: self.case_sensitive,
            empty_string_is_null: self.compatible_mode == 2,
            transactional_ddl: false,
            drop_index_by_name: true,
            drop_trigger_by_name: true,
            separate_comments: true,
            supports_identity: true,
            identity_editable: false,
            enriches_from_catalog: true,
            console_boxed_tables: true,
            console_errors_have_code: true,
            fk_reference_schema: true,
            supports_virtual_generated: false,
            supports_index_method: false,
            supports_index_expression: false,
            exposes_rowid: false,
            supports_partitions: false,
            ..DialectFeatures::default()
        }
    }
    fn table_option_style(&self) -> TableOptionStyle {
        // STORAGE(ON …, CLUSTERBTR) and similar private clauses stay in the advanced
        // SQL frame; the option form has no mapping for them (140-dameng.md §6.3).
        TableOptionStyle::StorageParameters
    }
    fn explain_style(&self) -> ExplainStyle {
        ExplainStyle::TextLines
    }
    fn column_types(&self) -> &'static [ColumnType] {
        COLUMN_TYPES
    }
    fn default_column_type(&self) -> &'static str {
        "INT"
    }
    fn added_column_type(&self) -> &'static str {
        "VARCHAR(200)"
    }
    fn default_schema(&self) -> Option<&'static str> {
        None
    }
    fn session_context_sql(&self) -> Option<&'static str> {
        // Columns follow the shared contract read by the designer's schema switch: database,
        // schema, and a "search path" that is the schema again for `restore_search_path_sql`.
        Some(
            "SELECT (SELECT NAME FROM V$DATABASE), SYS_CONTEXT('USERENV','CURRENT_SCHEMA'), \
             SYS_CONTEXT('USERENV','CURRENT_SCHEMA')",
        )
    }
    fn set_search_path_sql(&self, schema: &str) -> Option<String> {
        Some(format!("SET SCHEMA {}", self.quote_ident(schema)))
    }
    fn restore_search_path_sql(&self, path: &str) -> Option<String> {
        Some(format!("SET SCHEMA {}", self.quote_ident(path)))
    }
    /// Table options of an existing table. DM has no unlogged or inheritance, and no
    /// storage-parameter form; persistence is always `p` (140-dameng.md §5.4.1).
    fn table_options_sql(&self, scope: &Scope, name: &str) -> Option<String> {
        let owner = self.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
        let table = self.string_literal(name);
        Some(format!(
            "SELECT OWNER, 'p', '', TABLESPACE_NAME, '' FROM ALL_TABLES \
             WHERE OWNER = {owner} AND TABLE_NAME = {table}"
        ))
    }
    /// Column metadata of an existing table, nine columns per the shared enrich contract.
    /// Identity text needs `IDENT_SEED` / `IDENT_INCR`, which raise an error on tables
    /// without an identity column, so the identity branch is a `UNION ALL` arm filtered on
    /// `SYS.SYSCOLUMNS.INFO2`, where those tables produce no rows (140-dameng.md §5.4.1).
    fn column_metadata_sql(&self, scope: &Scope, name: &str) -> Option<String> {
        let owner = self.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
        let table = self.string_literal(name);
        let qualified = "C.OWNER || '.' || C.TABLE_NAME";
        let objid = "(SELECT O.ID FROM SYS.SYSOBJECTS O WHERE O.NAME = C.TABLE_NAME \
                     AND O.SCHID = (SELECT S2.ID FROM SYS.SYSOBJECTS S2 \
                     WHERE S2.NAME = C.OWNER AND S2.TYPE$ = 'SCH'))";
        Some(format!(
            "SELECT C.COLUMN_NAME, \
             CASE WHEN C.DATA_TYPE IN ('VARCHAR', 'VARCHAR2', 'CHAR', 'CHARACTER') \
             THEN C.DATA_TYPE || '(' || C.DATA_LENGTH || ')' \
             WHEN C.DATA_TYPE IN ('DECIMAL', 'NUMERIC', 'NUMBER', 'DEC') \
                  AND C.DATA_PRECISION IS NOT NULL \
             THEN C.DATA_TYPE || '(' || C.DATA_PRECISION || ', ' || C.DATA_SCALE || ')' \
             WHEN C.DATA_TYPE IN ('TIMESTAMP', 'DATETIME', 'TIME') \
                  AND C.DATA_SCALE > 0 \
             THEN C.DATA_TYPE || '(' || C.DATA_SCALE || ')' \
             ELSE C.DATA_TYPE END AS TYPE_TEXT, \
             CASE WHEN C.NULLABLE = 'N' THEN 't' ELSE 'f' END, C.DATA_DEFAULT, \
             NULL AS IDENTITY_TEXT, NULL AS GENERATED, NULL AS COLL_SCHEMA, \
             NULL AS COLL_NAME, CO.COMMENTS \
             FROM ALL_TAB_COLUMNS C LEFT JOIN ALL_COL_COMMENTS CO \
             ON CO.OWNER = C.OWNER AND CO.TABLE_NAME = C.TABLE_NAME \
             AND CO.COLUMN_NAME = C.COLUMN_NAME \
             WHERE C.OWNER = {owner} AND C.TABLE_NAME = {table} \
             UNION ALL \
             SELECT C.COLUMN_NAME, 'IDENTITY(' || IDENT_SEED({qualified}) || ', ' || \
             IDENT_INCR({qualified}) || ')' AS IDENTITY_TEXT, \
             CASE WHEN C.NULLABLE = 'N' THEN 't' ELSE 'f' END, C.DATA_DEFAULT, \
             'IDENTITY(' || IDENT_SEED({qualified}) || ', ' || IDENT_INCR({qualified}) || ')', \
             NULL, NULL, NULL, CO.COMMENTS \
             FROM ALL_TAB_COLUMNS C JOIN SYS.SYSCOLUMNS S \
             ON S.NAME = C.COLUMN_NAME AND S.ID = {objid} \
             LEFT JOIN ALL_COL_COMMENTS CO ON CO.OWNER = C.OWNER \
             AND CO.TABLE_NAME = C.TABLE_NAME AND CO.COLUMN_NAME = C.COLUMN_NAME \
             WHERE C.OWNER = {owner} AND C.TABLE_NAME = {table} AND S.INFO2 = 1 \
             ORDER BY 1"
        ))
    }
    /// The identity catalog value is already the `IDENTITY(seed, increment)` clause.
    fn identity_definition(&self, catalog: &str) -> String {
        if catalog.is_empty() {
            String::new()
        } else {
            format!(" {catalog}")
        }
    }
    /// DM uses the Oracle-style parenthesized form; column positions are not supported.
    fn add_column(&self, table: &str, column: &str, definition: &str, _position: &str) -> String {
        format!("ALTER TABLE {table} ADD ({column} {definition})")
    }
    /// Takes over the whole statement sequence for changing an existing column. Structural
    /// changes to an identity column are refused: DM cannot alter IDENTITY parameters, so
    /// the column would have to be dropped and re-added with data loss (140-dameng.md §6.3).
    fn modify_column(
        &self,
        table: &str,
        old_name: &str,
        new_name: &str,
        old_definition: &str,
        new_definition: &str,
    ) -> Option<Result<Vec<String>, String>> {
        if Self::is_identity_definition(old_definition)
            || Self::is_identity_definition(new_definition)
        {
            return Some(Err(
                "This column is an identity column. Dameng cannot alter IDENTITY parameters; \
                 drop the column and add it back (the data in it will be lost)."
                    .into(),
            ));
        }
        let mut out = Vec::new();
        if old_name != new_name {
            out.push(format!(
                "ALTER TABLE {table} RENAME COLUMN {old_name} TO {new_name}"
            ));
        }
        let (ty, null, default) = parse_definition(new_definition);
        let (old_ty, old_null, old_default) = parse_definition(old_definition);
        if ty != old_ty || null != old_null {
            // `MODIFY` sets type and nullability together; an omitted NOT NULL clears it.
            out.push(format!(
                "ALTER TABLE {table} MODIFY ({new_name} {ty}{null})"
            ));
        }
        if default != old_default {
            match default {
                Some(value) => out.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {new_name} SET DEFAULT {value}"
                )),
                None => out.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {new_name} DROP DEFAULT"
                )),
            }
        }
        Some(Ok(out))
    }
}

/// Splits a column definition into type text, a ` NOT NULL` tail, and a `DEFAULT` value.
/// Definitions come from the catalog enrich path and carry at most those three parts.
fn parse_definition(definition: &str) -> (String, String, Option<String>) {
    let definition = definition.trim();
    // Type text: a possibly multi-word name with one parenthesized argument list. The
    // argument list may contain spaces (`DECIMAL(10, 2)`), so cut at the closing paren
    // when there is one.
    let (ty, rest) = match definition.find('(').and_then(|open| {
        definition[open..]
            .find(')')
            .map(|close| (open, open + close + 1))
    }) {
        Some((_, end)) => (definition[..end].trim(), definition[end..].trim()),
        None => definition.split_once(' ').unwrap_or((definition, "")),
    };
    let upper = rest.to_ascii_uppercase();
    let null = if upper.contains("NOT NULL") {
        " NOT NULL"
    } else {
        ""
    };
    let default = upper
        .find("DEFAULT")
        .map(|at| rest[at + "DEFAULT".len()..].trim().to_string());
    (ty.to_string(), null.into(), default)
}

const COLUMN_TYPES: &[ColumnType] = &[
    ColumnType {
        name: "INT",
        synonyms: &["INTEGER", "PLS_INTEGER", "BINARY_INTEGER"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "BIGINT",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "TINYINT",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "SMALLINT",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "BIT",
        synonyms: &["BOOL"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "DECIMAL",
        synonyms: &["NUMERIC", "NUMBER", "DEC"],
        args: TypeArgs::LengthAndScale,
    },
    ColumnType {
        name: "FLOAT",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "REAL",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "DOUBLE PRECISION",
        synonyms: &["DOUBLE"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "BINARY",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "VARBINARY",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "CHAR",
        synonyms: &["CHARACTER"],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "VARCHAR",
        synonyms: &["VARCHAR2"],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "TEXT",
        synonyms: &["LONGVARCHAR"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "CLOB",
        synonyms: &["NCLOB", "NTEXT"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "BLOB",
        synonyms: &["IMAGE"],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "DATE",
        synonyms: &[],
        args: TypeArgs::None,
    },
    ColumnType {
        name: "TIME",
        synonyms: &[],
        args: TypeArgs::Length,
    },
    ColumnType {
        name: "TIMESTAMP",
        synonyms: &["DATETIME"],
        args: TypeArgs::Length,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_sql::dialect::Written;

    fn dialect() -> DamengDialect {
        DamengDialect::default()
    }

    fn scope() -> Scope {
        Scope::new("c", "DAMENG", Some("SYSDBA".into()))
    }

    #[test]
    fn quotes_identifiers_and_strings() {
        let d = dialect();
        assert_eq!(d.quote_ident("table"), "\"table\"");
        assert_eq!(d.quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(d.string_literal("it's"), "'it''s'");
        assert_eq!(d.qualified(&scope(), "T"), "\"SYSDBA\".\"T\"");
    }

    #[test]
    fn lex_follows_compatible_mode() {
        assert!(!dialect().lex().backtick_idents);
        let mysql_mode = DamengDialect {
            case_sensitive: true,
            compatible_mode: 4,
        };
        assert!(mysql_mode.lex().backtick_idents);
        assert!(!mysql_mode.lex().backslash_strings);
    }

    #[test]
    fn features_follow_the_probe() {
        let cs = dialect().features();
        assert!(cs.folds_unquoted_upper);
        assert!(!cs.folds_unquoted);
        assert!(!cs.empty_string_is_null);
        assert!(!cs.transactional_ddl);
        assert!(!cs.identity_editable);
        assert!(cs.drop_index_by_name && cs.drop_trigger_by_name);
        assert!(cs.separate_comments && cs.enriches_from_catalog);
        assert!(!cs.supports_virtual_generated);
        assert!(!cs.supports_index_method);
        assert_eq!(cs, dialect().features(), "features are Copy");

        let cs0 = DamengDialect {
            case_sensitive: false,
            compatible_mode: 0,
        }
        .features();
        assert!(!cs0.folds_unquoted_upper);

        let oracle_mode = DamengDialect {
            case_sensitive: false,
            compatible_mode: 2,
        }
        .features();
        assert!(oracle_mode.empty_string_is_null);
    }

    #[test]
    fn typed_name_quotes_lowercase_when_case_sensitive() {
        let d = dialect();
        // Unquoted names fold to upper case, so lowercase names must be quoted.
        assert_eq!(d.typed_name("actor"), "\"actor\"");
        assert_eq!(d.typed_name("ACTOR"), "ACTOR");
        // Keywords are always quoted.
        assert_eq!(d.typed_name("ORDER"), "\"ORDER\"");
        // With CASE_SENSITIVE=0 names are stored as written.
        let cs0 = DamengDialect {
            case_sensitive: false,
            compatible_mode: 0,
        };
        assert_eq!(cs0.typed_name("actor"), "actor");
    }

    #[test]
    fn transaction_and_context_sql() {
        let d = dialect();
        assert_eq!(d.begin_transaction(), "START TRANSACTION");
        assert_eq!(
            d.set_search_path_sql("OTHER").as_deref(),
            Some("SET SCHEMA \"OTHER\"")
        );
        assert_eq!(
            d.restore_search_path_sql("OTHER").as_deref(),
            Some("SET SCHEMA \"OTHER\"")
        );
    }

    #[test]
    fn explain_is_a_text_line_plan() {
        assert_eq!(dialect().explain_style(), ExplainStyle::TextLines);
    }

    #[test]
    fn add_column_uses_the_oracle_parenthesized_form() {
        assert_eq!(
            dialect().add_column("\"SYSDBA\".\"T\"", "\"C\"", "VARCHAR(10) NOT NULL", ""),
            "ALTER TABLE \"SYSDBA\".\"T\" ADD (\"C\" VARCHAR(10) NOT NULL)"
        );
    }

    #[test]
    fn modify_column_writes_rename_modify_and_default() {
        let d = dialect();
        let out = d
            .modify_column(
                "\"SYSDBA\".\"T\"",
                "\"A\"",
                "\"B\"",
                "VARCHAR(10) NOT NULL DEFAULT 'x'",
                "VARCHAR(20) DEFAULT 'y'",
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            vec![
                "ALTER TABLE \"SYSDBA\".\"T\" RENAME COLUMN \"A\" TO \"B\"",
                "ALTER TABLE \"SYSDBA\".\"T\" MODIFY (\"B\" VARCHAR(20))",
                "ALTER TABLE \"SYSDBA\".\"T\" ALTER COLUMN \"B\" SET DEFAULT 'y'",
            ]
        );
    }

    #[test]
    fn modify_column_drops_a_default_explicitly() {
        let out = dialect()
            .modify_column("\"T\"", "\"A\"", "\"A\"", "INT DEFAULT 1", "INT")
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            vec!["ALTER TABLE \"T\" ALTER COLUMN \"A\" DROP DEFAULT"]
        );
    }

    #[test]
    fn modify_column_refuses_identity_changes() {
        let d = dialect();
        for (old, new) in [
            ("INT IDENTITY(1, 1) NOT NULL", "INT"),
            ("INT", "INT IDENTITY(2, 1) NOT NULL"),
        ] {
            let err = d
                .modify_column("\"T\"", "\"ID\"", "\"ID\"", old, new)
                .unwrap()
                .unwrap_err();
            assert!(err.contains("identity"), "{err}");
        }
        // A name that merely contains the word is not an identity clause.
        assert!(
            d.modify_column("\"T\"", "\"MY_ID\"", "\"MY_ID\"", "INT", "INT")
                .unwrap()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn metadata_queries_bind_owner_and_table() {
        let d = dialect();
        let options = d.table_options_sql(&scope(), "T").unwrap();
        assert!(options.contains("ALL_TABLES"));
        assert!(options.contains("'SYSDBA'") && options.contains("'T'"));
        let columns = d.column_metadata_sql(&scope(), "T").unwrap();
        assert_eq!(
            columns.matches("ALL_TAB_COLUMNS").count(),
            2,
            "UNION ALL arms"
        );
        assert!(columns.contains("IDENT_SEED(C.OWNER || '.' || C.TABLE_NAME)"));
        assert!(columns.contains("S.INFO2 = 1"), "identity arm is filtered");
    }

    #[test]
    fn identity_definition_passes_the_clause_through() {
        let d = dialect();
        assert_eq!(d.identity_definition("IDENTITY(1, 1)"), " IDENTITY(1, 1)");
        assert_eq!(d.identity_definition(""), "");
    }

    #[test]
    fn parse_definition_splits_type_null_and_default() {
        assert_eq!(
            parse_definition("VARCHAR(10) NOT NULL DEFAULT 'x'"),
            ("VARCHAR(10)".into(), " NOT NULL".into(), Some("'x'".into()))
        );
        assert_eq!(
            parse_definition("DECIMAL(10, 2)"),
            ("DECIMAL(10, 2)".into(), String::new(), None)
        );
        assert_eq!(
            parse_definition("TIMESTAMP(6)"),
            ("TIMESTAMP(6)".into(), String::new(), None)
        );
    }

    #[test]
    fn no_master_table_or_exclusive_lock() {
        let d = dialect();
        assert!(d.master_table_sql("DAMENG").is_none());
        assert!(d.exclusive_lock_sql("\"T\"").is_none());
        // Identities carry no owner-side sequences and DM renames tables its own way.
        assert!(d.owned_sequences_sql("\"T\"").is_none());
        assert!(
            d.rename_object(&scope(), ObjectKind::Sequence, "S", "S2")
                .is_none()
        );
        assert!(
            d.rename_object(&scope(), ObjectKind::Table, "T", "T2")
                .is_some()
        );
    }

    #[test]
    fn an_emptied_cell_writes_null_and_empty_comparisons_check_null_in_mode_2() {
        // COMPATIBLE_MODE=2 stores '' as NULL: the write path emits NULL for an emptied
        // cell and the shared filter generator rewrites the empty comparisons.
        let mode2 = DamengDialect {
            case_sensitive: false,
            compatible_mode: 2,
        };
        let empty = Written::text(Some(String::new()));
        assert_eq!(mode2.written_sql(&empty), "NULL");
        let value = Written::text(Some("x".into()));
        assert_eq!(mode2.written_sql(&value), "'x'");
        let raw = Written::expr("SEQ.NEXTVAL");
        assert_eq!(mode2.written_sql(&raw), "SEQ.NEXTVAL");
    }

    #[test]
    fn duplicate_table_copies_with_or_without_data() {
        let d = dialect();
        assert_eq!(
            d.duplicate_table(&scope(), "T", "T2", true, None).unwrap(),
            vec!["CREATE TABLE \"SYSDBA\".\"T2\" AS SELECT * FROM \"SYSDBA\".\"T\""]
        );
        assert_eq!(
            d.duplicate_table(&scope(), "T", "T2", false, None).unwrap(),
            vec!["CREATE TABLE \"SYSDBA\".\"T2\" AS SELECT * FROM \"SYSDBA\".\"T\" WHERE 1 = 0"]
        );
    }
}
