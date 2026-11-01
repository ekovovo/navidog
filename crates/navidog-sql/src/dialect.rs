//! SQL dialects: identifier quoting, literals, paged queries and SQL generation for grid change
//! sets (21-table-viewer.md §2).

use navidog_model::object::{ObjectKind, Scope};

use crate::keywords::Words;
use crate::span::{Lex, Span, SpanKind};

/// How a session's current namespace is chosen. Feature code branches on this, not on a product name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextSelection {
    /// `USE` changes the current database.
    Database,
    /// The database is fixed for the session; `search_path` changes the schema.
    Schema,
    /// A file database has no server-side current catalog.
    File,
}

/// The kind of constraint an `ALTER TABLE … DROP` targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Primary,
    ForeignKey,
    Unique,
    Check,
    Index,
    /// PostgreSQL `EXCLUDE` constraints, dropped like any other named constraint.
    Exclude,
}

/// How a product writes the clauses that follow a table's column definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableOptionStyle {
    /// `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4` clauses (MySQL).
    Clauses,
    /// `WITH (fillfactor=70)` storage parameters, plus separate `TABLESPACE` / `INHERITS`
    /// (PostgreSQL).
    StorageParameters,
    /// Flags after the closing parenthesis, such as `STRICT` / `WITHOUT ROWID` (SQLite).
    TableFlags,
}

/// How many parenthesized arguments a designer type stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeArgs {
    /// The server drops a length, so the designer does not edit one.
    None,
    Length,
    LengthAndScale,
}

/// One type in the table designer menu, spelled as the server prints it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnType {
    pub name: &'static str,
    pub synonyms: &'static [&'static str],
    pub args: TypeArgs,
}

/// Which plan a product's `EXPLAIN` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplainStyle {
    /// `EXPLAIN` fills a table and `EXPLAIN FORMAT=JSON` returns a nested plan (MySQL, MariaDB).
    TableAndJson,
    /// `EXPLAIN (FORMAT JSON)` returns plan nodes with costs (PostgreSQL).
    JsonNodes,
    /// `EXPLAIN QUERY PLAN` returns `(id, parent, detail)` rows (SQLite).
    QueryPlanRows,
}

/// What a product can do, apart from the SQL it writes. Grouped rather than one trait method per
/// flag: a reader sees the whole list at once, and a new dialect fills in one value.
///
/// `Default` describes a plain relational product (transactional DDL, standalone `CREATE INDEX`,
/// column comments, `USING` methods, index expressions, schema-qualified foreign keys, virtual
/// generated columns); a product that differs overrides those fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialectFeatures {
    /// A schema level sits between the database and its objects.
    pub has_schemas: bool,
    /// Selecting a table exposes an implicit `rowid` column.
    pub exposes_rowid: bool,
    /// Writes that match no primary key are limited to one row.
    pub limit_unkeyed_writes: bool,
    /// Unquoted identifiers fold to lower case.
    pub folds_unquoted: bool,
    /// `DROP INDEX name` works without an `ALTER TABLE`.
    pub drop_index_by_name: bool,
    /// `DROP TRIGGER name` works without an `ALTER TABLE`.
    pub drop_trigger_by_name: bool,
    /// The console prints a boxed table and errors without a numeric code.
    pub console_boxed_tables: bool,
    /// An error line carries the server's numeric code.
    pub console_errors_have_code: bool,
    /// `LIKE` works on numbers and dates after a `::text` cast.
    pub like_text_cast: bool,
    /// `UNLOGGED` tables.
    pub supports_unlogged: bool,
    /// `FIRST` / `AFTER` in a column definition.
    pub supports_column_position: bool,
    /// The column comment is a separate `COMMENT ON COLUMN` statement.
    pub separate_comments: bool,
    /// Indexes are their own `CREATE INDEX` statements, not clauses of `ALTER TABLE`.
    pub standalone_indexes: bool,
    /// `ZEROFILL` on numeric columns.
    pub supports_zerofill: bool,
    /// `ON UPDATE` on a temporal column.
    pub supports_on_update: bool,
    /// DDL rolls back with the transaction.
    pub transactional_ddl: bool,
    /// Dropping an index goes through `ALTER TABLE`.
    pub drop_index_alters_table: bool,
    /// `BEGIN IMMEDIATE` rather than `BEGIN` when applying DDL.
    pub begin_immediate: bool,
    /// One `CHANGE COLUMN` rewrites the name, type and position together.
    pub rewrites_columns: bool,
    /// Table options are clauses of one `ALTER TABLE` (engine, charset).
    pub inline_table_options: bool,
    /// Changing an existing table means rebuilding it.
    pub rebuilds_tables: bool,
    /// After `SHOW`/`pg_get` DDL, read owner, persistence and column metadata from the catalog.
    pub enriches_from_catalog: bool,
    /// `SHOW PROFILE`: run again with profiling and show the stage table.
    pub supports_profile: bool,
    /// Affected rows of an `UPDATE` count only rows whose value changed, so a no-op write would
    /// look like a lost row.
    pub counts_changed_rows: bool,
    /// A session switches its current database with [`Dialect::prepare_database_sql`].
    pub switches_database: bool,
    /// Columns carry a comment at all.
    pub supports_column_comments: bool,
    /// The column comment is a clause of the column definition (MySQL `COMMENT '…'`).
    pub column_comment_inline: bool,
    /// `GENERATED … AS IDENTITY`, with `ALWAYS` / `BY DEFAULT`.
    pub supports_identity: bool,
    /// Text values reject `NUL`.
    pub text_rejects_nul: bool,
    /// `AUTOINCREMENT` only applies to `INTEGER PRIMARY KEY` columns.
    pub autoincrement_needs_integer_key: bool,
    /// `CREATE INDEX … COMMENT '…'`.
    pub supports_index_comment: bool,
    /// `ILIKE`, so the filter pane can offer case-insensitive contains.
    pub case_insensitive_like: bool,
    /// `GENERATED ALWAYS AS (…) VIRTUAL`; a product with only stored generated columns omits it.
    pub supports_virtual_generated: bool,
    /// Per-column `CHARACTER SET` / `COLLATE`.
    pub supports_column_charset: bool,
    /// `UNSIGNED` on numeric columns.
    pub supports_unsigned: bool,
    /// `CREATE INDEX … USING method`.
    pub supports_index_method: bool,
    /// Index entries may be expressions rather than column names.
    pub supports_index_expression: bool,
    /// A foreign key names a schema (or database) besides the table.
    pub fk_reference_schema: bool,
    /// PostgreSQL `EXCLUDE` constraints.
    pub supports_exclude: bool,
    /// `PARTITION BY` on a table (MySQL and PostgreSQL).
    pub supports_partitions: bool,
}

impl Default for DialectFeatures {
    fn default() -> Self {
        Self {
            has_schemas: false,
            exposes_rowid: false,
            limit_unkeyed_writes: false,
            folds_unquoted: false,
            drop_index_by_name: false,
            drop_trigger_by_name: false,
            console_boxed_tables: false,
            console_errors_have_code: true,
            like_text_cast: false,
            supports_unlogged: false,
            supports_column_position: false,
            separate_comments: false,
            standalone_indexes: true,
            supports_zerofill: false,
            supports_on_update: false,
            transactional_ddl: true,
            drop_index_alters_table: false,
            begin_immediate: false,
            rewrites_columns: false,
            inline_table_options: false,
            rebuilds_tables: false,
            enriches_from_catalog: false,
            supports_profile: false,
            counts_changed_rows: false,
            switches_database: false,
            supports_column_comments: true,
            column_comment_inline: false,
            supports_identity: false,
            text_rejects_nul: false,
            autoincrement_needs_integer_key: false,
            supports_index_comment: false,
            case_insensitive_like: false,
            supports_virtual_generated: true,
            supports_column_charset: false,
            supports_unsigned: false,
            supports_index_method: true,
            supports_index_expression: true,
            fk_reference_schema: true,
            supports_exclude: false,
            supports_partitions: false,
        }
    }
}

/// One database product's SQL rules. Implementations live in the per-database crates.
///
/// Feature code asks these methods; it must not compare [`Dialect::id`], which would lose the
/// exhaustiveness check a capability method has. `scripts/check-dialect-branches.sh` enforces it.
pub trait Dialect: std::fmt::Debug + Send + Sync {
    /// Registry-style identifier, for logs and diagnostics. Never branch on it.
    fn id(&self) -> &'static str;
    fn lex(&self) -> Lex;
    fn quote_ident(&self, name: &str) -> String;
    fn string_literal(&self, value: &str) -> String;
    /// Database or schema prefixed before an object name, when the dialect wants one.
    fn qualify_prefix<'a>(&self, scope: &'a Scope) -> Option<&'a str>;
    fn object_kinds(&self) -> &'static [ObjectKind];
    fn begin_transaction(&self) -> &'static str;
    fn is_system_database(&self, name: &str) -> bool;
    fn is_system_schema(&self, name: &str) -> bool;
    /// A catalog object the product ships with, such as SQLite's `sqlite_stat1`.
    fn is_system_object(&self, _name: &str) -> bool {
        false
    }
    fn extra_keywords(&self) -> &'static [&'static str];
    fn extra_functions(&self) -> &'static [&'static str];
    fn rename_object(
        &self,
        scope: &Scope,
        kind: ObjectKind,
        old: &str,
        new: &str,
    ) -> Option<String>;
    fn duplicate_table(
        &self,
        scope: &Scope,
        old: &str,
        new: &str,
        with_data: bool,
        ddl: Option<&str>,
    ) -> Option<Vec<String>>;
    fn console_prompt(&self, database: &str, superuser: bool, continuation: bool) -> String;
    fn context_selection(&self) -> ContextSelection;
    /// What this product can do. Prefer a field here over a new boolean method.
    fn features(&self) -> DialectFeatures;
    /// Query that reads the session's current database, and for [`ContextSelection::Schema`]
    /// also the schema and `search_path`. `None` when the product has no such state.
    fn session_context_sql(&self) -> Option<&'static str> {
        None
    }
    fn prepare_database_sql(&self, _database: &str) -> Option<String> {
        None
    }
    fn set_search_path_sql(&self, _schema: &str) -> Option<String> {
        None
    }
    /// Put back a recorded `search_path` setting, which may be a comma-separated list.
    /// [`set_search_path_sql`](Dialect::set_search_path_sql) quotes one schema name and would
    /// treat that list as a single identifier.
    fn restore_search_path_sql(&self, _path: &str) -> Option<String> {
        None
    }

    fn ident_quote(&self) -> char {
        '"'
    }
    /// MySQL drops constraints with `DROP PRIMARY KEY` / `DROP FOREIGN KEY` / `DROP INDEX`.
    /// `None` means this designer does not know a safe statement for that kind.
    fn drop_constraint_clause(&self, _kind: ConstraintKind, name: &str) -> Option<String> {
        Some(format!("DROP CONSTRAINT {}", self.quote_ident(name)))
    }
    /// Statement that locks the table for the duration of a structure change.
    fn exclusive_lock_sql(&self, _qualified: &str) -> Option<String> {
        None
    }
    /// `ALTER TABLE ... OWNER TO`, when the product has table owners.
    fn table_owner_sql(&self, _table: &str, _owner: &str) -> Option<String> {
        None
    }
    /// Default storage of a generated column when the user does not pick one.
    fn generated_default_storage(&self) -> &'static str {
        "VIRTUAL"
    }
    fn autoincrement_keyword(&self) -> &'static str {
        "AUTO_INCREMENT"
    }

    /// How the table designer's option form maps onto this product's `CREATE TABLE` tail.
    fn table_option_style(&self) -> TableOptionStyle;
    /// Keys of the table-option form, in display order.
    fn table_option_keys(&self) -> &'static [&'static str] {
        &[]
    }
    /// Keys the option form edits as checkboxes rather than text inputs.
    fn bool_table_options(&self) -> &'static [&'static str] {
        &[]
    }
    /// Which `EXPLAIN` form [`crate::Dialect`] consumers should use.
    fn explain_style(&self) -> ExplainStyle;
    /// Types offered in the table designer, in menu order. Each name is the spelling this
    /// product prints back after `CREATE TABLE`.
    fn column_types(&self) -> &'static [ColumnType] {
        &[]
    }
    /// The menu entry named by `name`, including a case-insensitive synonym.
    fn column_type(&self, name: &str) -> Option<&'static ColumnType> {
        let name = name.trim();
        self.column_types().iter().find(|ty| {
            ty.name.eq_ignore_ascii_case(name)
                || ty
                    .synonyms
                    .iter()
                    .any(|synonym| synonym.eq_ignore_ascii_case(name))
        })
    }
    /// Type of a new table's `id` column.
    fn default_column_type(&self) -> &'static str {
        "INTEGER"
    }
    /// Type of a field added to an existing design.
    fn added_column_type(&self) -> &'static str {
        "TEXT"
    }
    /// `USING` methods offered for an index.
    fn index_methods(&self) -> &'static [&'static str] {
        &["BTREE", "HASH"]
    }
    /// Schema assumed when a scope names none, when the product has one.
    fn default_schema(&self) -> Option<&'static str> {
        None
    }
    /// The only catalog whose tables the designer can rebuild by copying them; `None` when the
    /// dialect rebuilds without one.
    fn table_rebuild_catalog(&self) -> Option<&'static str> {
        None
    }

    fn literal(&self, value: Option<&str>) -> String {
        match value {
            None => "NULL".into(),
            Some(v) => self.string_literal(v),
        }
    }

    fn written_sql(&self, written: &Written) -> String {
        if written.raw
            && let Some(expr) = written
                .value
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        {
            expr.to_string()
        } else {
            self.literal(written.value.as_deref())
        }
    }

    fn qualified(&self, scope: &Scope, name: &str) -> String {
        match self.qualify_prefix(scope) {
            Some(p) => format!("{}.{}", self.quote_ident(p), self.quote_ident(name)),
            None => self.quote_ident(name),
        }
    }

    fn drop_keyword(&self, kind: ObjectKind) -> Option<&'static str> {
        Some(match kind {
            ObjectKind::Table => "TABLE",
            ObjectKind::View => "VIEW",
            ObjectKind::MaterializedView => "MATERIALIZED VIEW",
            ObjectKind::Event => "EVENT",
            ObjectKind::Trigger if self.features().drop_trigger_by_name => "TRIGGER",
            ObjectKind::Index if self.features().drop_index_by_name => "INDEX",
            ObjectKind::Sequence => "SEQUENCE",
            ObjectKind::Tablespace => "TABLESPACE",
            ObjectKind::Type => "TYPE",
            ObjectKind::Domain => "DOMAIN",
            ObjectKind::Extension => "EXTENSION",
            ObjectKind::ForeignServer => "SERVER",
            ObjectKind::Collation => "COLLATION",
            // Aggregate and operator drops need an argument signature this name cannot supply.
            ObjectKind::Aggregate
            | ObjectKind::Operator
            | ObjectKind::Trigger
            | ObjectKind::Index
            | ObjectKind::Cast
            | ObjectKind::Function
            | ObjectKind::Query => return None,
        })
    }

    /// Kinds this connection shows. MariaDB sequences depend on the driver id and server version.
    fn lists_kind(&self, kind: ObjectKind, _driver: &str, _version: &str) -> bool {
        self.object_kinds().contains(&kind)
    }

    /// Statement family for the shared object designer. `None` when the product has no designer.
    fn definition_family(&self) -> Option<crate::definition::DefinitionFamily> {
        None
    }

    /// `SELECT` of the catalog table that lists every object, such as SQLite's `sqlite_master`.
    /// Products without that table return `None`, so the menu item is absent.
    fn master_table_sql(&self, _database: &str) -> Option<String> {
        None
    }

    /// One column: `CREATE TRIGGER` text for this table. `None` when the snapshot loads triggers another way.
    fn trigger_definitions_sql(&self, _scope: &Scope, _table: &str) -> Option<String> {
        None
    }

    /// Preview SQL for a definition draft. Empty means nothing changed.
    fn definition_statements(
        &self,
        _scope: &Scope,
        _draft: &crate::definition::DefinitionDraft,
    ) -> Result<Vec<String>, String> {
        Err("This database cannot change that object.".into())
    }

    /// `REFRESH MATERIALIZED VIEW`, when the product has materialized views.
    fn refresh_materialized_view(
        &self,
        _scope: &Scope,
        _name: &str,
        _with_data: bool,
    ) -> Option<String> {
        None
    }

    /// Which account form to show. SQLite explains the missing extension and runs nothing.
    fn account_form(&self) -> crate::security::AccountForm {
        crate::security::account_form(self.definition_family())
    }

    fn account_list_sql(&self) -> Option<&'static str> {
        crate::security::account_list_sql(self.definition_family())
    }

    fn manages_privileges(&self) -> bool {
        crate::security::manages_privileges(self.definition_family())
    }

    fn monitors_server(&self) -> bool {
        crate::security::monitors_server(self.definition_family())
    }

    fn process_sql(&self) -> Option<&'static str> {
        crate::security::process_sql(self.definition_family())
    }

    fn kill_sql(&self, id: &str) -> Result<Option<String>, String> {
        crate::security::kill_sql(self.definition_family(), id)
    }

    fn variables_sql(&self) -> Option<&'static str> {
        crate::security::variables_sql(self.definition_family())
    }

    fn status_sql(&self) -> Option<&'static str> {
        crate::security::status_sql(self.definition_family())
    }

    /// `ALTER TABLE … RENAME COLUMN`.
    fn rename_column(&self, scope: &Scope, table: &str, old: &str, new: &str) -> String {
        format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {}",
            self.qualified(scope, table),
            self.quote_ident(old),
            self.quote_ident(new)
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn select_page(
        &self,
        scope: &Scope,
        table: &str,
        filter: &str,
        order: &str,
        limit: Option<u64>,
        offset: u64,
        rowid: bool,
    ) -> String {
        let list = if rowid && self.features().exposes_rowid {
            "*, rowid AS \"_rowid\""
        } else {
            "*"
        };
        let mut sql = format!("SELECT {list} FROM {}", self.qualified(scope, table));
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

    fn count(&self, scope: &Scope, table: &str, filter: &str) -> String {
        let mut sql = format!("SELECT COUNT(*) FROM {}", self.qualified(scope, table));
        if !filter.trim().is_empty() {
            sql.push_str(&format!(" WHERE {}", filter.trim()));
        }
        sql
    }

    fn render_changes(
        &self,
        scope: &Scope,
        table: &str,
        changes: &ChangeSet,
        has_primary_key: bool,
    ) -> Vec<String> {
        let target = self.qualified(scope, table);
        let limit = if self.features().limit_unkeyed_writes && !has_primary_key {
            " LIMIT 1"
        } else {
            ""
        };
        let mut out = Vec::new();
        for key in &changes.deletes {
            out.push(format!(
                "DELETE FROM {target} WHERE {}{limit}",
                assignments(self, &key.0, Assign::Key)
            ));
        }
        for u in &changes.updates {
            if u.values.is_empty() {
                continue;
            }
            out.push(format!(
                "UPDATE {target} SET {} WHERE {}{limit}",
                written_list(self, &u.values),
                assignments(self, &u.key.0, Assign::Key)
            ));
        }
        for row in &changes.inserts {
            let cols: Vec<_> = row.iter().map(|(c, _)| self.quote_ident(c)).collect();
            let vals: Vec<_> = row.iter().map(|(_, v)| self.written_sql(v)).collect();
            out.push(format!(
                "INSERT INTO {target} ({}) VALUES ({})",
                cols.join(", "),
                vals.join(", ")
            ));
        }
        out
    }

    fn insert_statements(
        &self,
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

    fn update_statements(
        &self,
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
                    assignments(self, &sets, Assign::Set),
                    assignments(self, &keys, Assign::Key)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn keywords(&self) -> Words {
        Words::new(crate::keywords::COMMON, self.extra_keywords())
    }

    fn functions(&self) -> Words {
        Words::new(crate::keywords::COMMON_FUNCTIONS, self.extra_functions())
    }

    /// Linear over both lists; no allocation, so completion can call it per candidate.
    fn is_keyword(&self, word: &str) -> bool {
        crate::keywords::COMMON
            .iter()
            .chain(self.extra_keywords())
            .any(|k| k.eq_ignore_ascii_case(word))
    }

    fn typed_name(&self, name: &str) -> String {
        let plain = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let folds = self.features().folds_unquoted && name.chars().any(|c| c.is_ascii_uppercase());
        if plain && !folds && !self.is_keyword(name) {
            name.to_string()
        } else {
            self.quote_ident(name)
        }
    }

    fn format_console_error(&self, code: Option<&str>, message: &str) -> String {
        if !self.features().console_errors_have_code {
            format!("ERROR:  {message}\n")
        } else if let Some(code) = code {
            format!("ERROR {code}: {message}\n")
        } else {
            format!("ERROR: {message}\n")
        }
    }
}

fn written_list<D: Dialect + ?Sized>(dialect: &D, values: &Writes) -> String {
    values
        .iter()
        .map(|(c, v)| format!("{} = {}", dialect.quote_ident(c), dialect.written_sql(v)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How a [`Values`] list is used: `WHERE` predicates match NULL with `IS NULL`, while assignments
/// in a `SET` list always compare with `=`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Assign {
    Set,
    Key,
}

fn assignments<D: Dialect + ?Sized>(dialect: &D, values: &Values, assign: Assign) -> String {
    values
        .iter()
        .map(|(c, v)| match (assign, v) {
            (Assign::Key, None) => format!("{} IS NULL", dialect.quote_ident(c)),
            _ => format!(
                "{} = {}",
                dialect.quote_ident(c),
                dialect.literal(v.as_deref())
            ),
        })
        .collect::<Vec<_>>()
        .join(if assign == Assign::Key { " AND " } else { ", " })
}

/// Values of a row keyed by column name; `None` is NULL.
pub type Values = Vec<(String, Option<String>)>;

/// A value written by an edit or an insert. When `raw` is set and the text is non-empty, it is SQL
/// (raw data mode) and is not quoted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub value: Option<String>,
    pub raw: bool,
}

impl Written {
    pub fn text(value: Option<String>) -> Self {
        Self { value, raw: false }
    }

    pub fn expr(sql: impl Into<String>) -> Self {
        Self {
            value: Some(sql.into()),
            raw: true,
        }
    }
}

pub type Writes = Vec<(String, Written)>;

/// Locates a row: only the primary key columns when there is a primary key, otherwise the whole
/// row's old values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowKey(pub Values);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowUpdate {
    pub key: RowKey,
    pub values: Writes,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub updates: Vec<RowUpdate>,
    pub inserts: Vec<Writes>,
    pub deletes: Vec<RowKey>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.inserts.is_empty() && self.deletes.is_empty()
    }
}

/// Replaces the table name in a `CREATE TABLE` statement. The name may be bare or quoted with
/// `"`, `` ` `` or `[]`, follow `IF NOT EXISTS`, and carry a database prefix. Comments before the
/// statement and a longer name that starts with `old` (`actor` vs `actor_id`) are handled by
/// reading the shared span list instead of scanning the text.
pub fn rename_create_table(ddl: &str, old: &str, new_qualified: &str, lex: Lex) -> Option<String> {
    let spans = crate::span::spans(ddl, lex);
    let code: Vec<_> = spans
        .iter()
        .filter(|span| span.kind.is_code() && span.kind != SpanKind::StatementEnd)
        .collect();
    let word = |span: &&Span| span.kind == SpanKind::Word;
    let text = |span: &Span| span.text(ddl);
    let mut i = 0;
    // CREATE [TEMP|TEMPORARY] TABLE
    if !code
        .get(i)
        .is_some_and(|s| word(s) && text(s).eq_ignore_ascii_case("CREATE"))
    {
        return None;
    }
    i += 1;
    if code.get(i).is_some_and(|s| {
        word(s)
            && ["TEMP", "TEMPORARY"]
                .iter()
                .any(|k| text(s).eq_ignore_ascii_case(k))
    }) {
        i += 1;
    }
    if !code
        .get(i)
        .is_some_and(|s| word(s) && text(s).eq_ignore_ascii_case("TABLE"))
    {
        return None;
    }
    i += 1;
    // IF NOT EXISTS
    if code
        .get(i)
        .is_some_and(|s| word(s) && text(s).eq_ignore_ascii_case("IF"))
    {
        for expected in ["NOT", "EXISTS"] {
            i += 1;
            if !code
                .get(i)
                .is_some_and(|s| word(s) && text(s).eq_ignore_ascii_case(expected))
            {
                return None;
            }
        }
        i += 1;
    }
    // [schema.]name, possibly quoted
    let first = *code.get(i)?;
    let start = first.range.start;
    let mut name = first;
    let mut end = name.range.end;
    if code.get(i + 1).is_some_and(|s| text(s) == ".") && code.get(i + 2).is_some() {
        name = code[i + 2];
        end = name.range.end;
    }
    if !matches!(name.kind, SpanKind::Word | SpanKind::QuotedIdent) {
        return None;
    }
    if !unquote_ident(text(name)).eq_ignore_ascii_case(old) {
        return None;
    }
    Some(format!("{}{new_qualified}{}", &ddl[..start], &ddl[end..]))
}

/// Identifier text without its quote characters.
fn unquote_ident(text: &str) -> &str {
    for (open, close) in [('"', '"'), ('`', '`'), ('[', ']')] {
        if text.starts_with(open) && text.ends_with(close) && text.len() >= 2 {
            return &text[1..text.len() - 1];
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(ddl: &str, old: &str) -> Option<String> {
        rename_create_table(ddl, old, "\"app\".\"copy\"", Lex::SQLITE)
    }

    #[test]
    fn renames_a_quoted_or_prefixed_table_name() {
        assert_eq!(
            rename(
                "CREATE TABLE IF NOT EXISTS \"app\".\"actor\" (id INTEGER PRIMARY KEY)",
                "actor"
            )
            .unwrap(),
            "CREATE TABLE IF NOT EXISTS \"app\".\"copy\" (id INTEGER PRIMARY KEY)"
        );
        assert_eq!(
            rename("create table [actor](id)", "actor").unwrap(),
            "create table \"app\".\"copy\"(id)"
        );
        assert_eq!(
            rename("CREATE TEMP TABLE `actor` (id)", "actor").unwrap(),
            "CREATE TEMP TABLE \"app\".\"copy\" (id)"
        );
    }

    #[test]
    fn ignores_a_comment_before_the_statement() {
        assert_eq!(
            rename(
                "-- CREATE TABLE other\nCREATE TABLE \"actor\" (id)",
                "actor"
            )
            .unwrap(),
            "-- CREATE TABLE other\nCREATE TABLE \"app\".\"copy\" (id)"
        );
    }

    #[test]
    fn refuses_a_different_or_prefix_name() {
        assert!(rename("CREATE TABLE \"actor_id\" (id)", "actor").is_none());
        assert!(rename("CREATE TABLE \"film\" (id)", "actor").is_none());
        assert!(rename("CREATE VIEW \"actor\" AS SELECT 1", "actor").is_none());
        assert!(rename("CREATE TABLE (id)", "actor").is_none());
    }
}
