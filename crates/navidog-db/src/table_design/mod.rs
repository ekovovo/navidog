//! Lossless table drafts and guarded, dialect-specific schema changes.
mod comments;
mod entries;
mod options;
mod parse;
mod properties;
pub use entries::{DesignEntry, EntryKind, parse_entry, split_field_list};
pub use options::{MYSQL_OPTIONS, POSTGRES_OPTIONS, edit_table_option, table_option};
pub use properties::{ColumnProperties, ColumnProperty, column_properties, edit_column};
mod plan;
#[cfg(test)]
mod tests;

use crate::{DbError, DbResult, Relational, dialect::Dialect};
use navidog_core::object::{ObjectKind, Scope};
pub use plan::{plan, plan_with};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DesignColumn {
    pub id: u64,
    pub original: Option<String>,
    pub name: String,
    pub definition: String,
    #[serde(default)]
    pub comment: String,
}
impl DesignColumn {
    pub fn new(id: u64, name: impl Into<String>, definition: impl Into<String>) -> Self {
        Self {
            id,
            original: None,
            name: name.into(),
            definition: definition.into(),
            comment: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TableDesign {
    pub name: String,
    pub columns: Vec<DesignColumn>,
    pub constraints: Vec<String>,
    pub indexes: Vec<String>,
    pub options: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub unlogged: bool,
}
impl TableDesign {
    pub fn new() -> Self {
        Self {
            columns: vec![DesignColumn::new(1, "id", "INTEGER NOT NULL")],
            constraints: vec!["PRIMARY KEY (id)".into()],
            ..Default::default()
        }
    }
    pub fn create_sql(&self, dialect: Dialect, scope: &Scope, name: &str) -> DbResult<String> {
        self.validate(dialect)?;
        let mut parts: Vec<_> = self
            .columns
            .iter()
            .map(|c| format!("  {} {}", dialect.quote_ident(&c.name), c.definition.trim()))
            .collect();
        parts.extend(self.constraints.iter().map(|c| format!("  {}", c.trim())));
        Ok(format!(
            "CREATE {}TABLE {} (\n{}\n){}",
            if dialect == Dialect::Postgres && self.unlogged {
                "UNLOGGED "
            } else {
                ""
            },
            dialect.qualified(scope, name),
            parts.join(",\n"),
            if self.options.trim().is_empty() {
                String::new()
            } else {
                format!(" {}", self.options.trim())
            }
        ))
    }
    fn validate(&self, dialect: Dialect) -> DbResult<()> {
        if self.name.trim().is_empty() || self.columns.is_empty() {
            return Err(DbError::msg(
                "A table needs a name and at least one column.",
            ));
        }
        let mut names = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        let mut originals = std::collections::HashSet::new();
        for c in &self.columns {
            if c.name.trim().is_empty()
                || c.name.contains('\0')
                || !names.insert(c.name.to_lowercase())
                || !ids.insert(c.id)
                || c.original
                    .as_ref()
                    .is_some_and(|name| !originals.insert(name))
            {
                return Err(DbError::msg(
                    "Column names and identities must be nonempty and unique.",
                ));
            }
            if c.definition.trim().is_empty() {
                return Err(DbError::msg("A column needs a type definition."));
            }
            parse::fragment(&c.definition)?;
        }
        if self.comment.contains('\0') || self.columns.iter().any(|c| c.comment.contains('\0')) {
            return Err(DbError::msg("Comments cannot contain NUL characters."));
        }
        if dialect != Dialect::Postgres
            && (!self.comment.is_empty() || self.columns.iter().any(|c| !c.comment.is_empty()))
        {
            return Err(DbError::msg(
                "Separate comment definitions are supported only on PostgreSQL.",
            ));
        }
        for c in &self.constraints {
            parse::fragment(c)?;
        }
        parse::fragment(&self.options)?;
        for index in &self.indexes {
            if crate::sql::split_script(index, dialect).len() != 1 {
                return Err(DbError::msg(
                    "Each supplemental definition must contain one SQL statement.",
                ));
            }
            if dialect == Dialect::Postgres
                && (index.starts_with("COMMENT ON TABLE ")
                    || index.starts_with("COMMENT ON COLUMN "))
            {
                continue;
            }
            if crate::sql::split_script(index, dialect).len() != 1
                || !index.trim_start().to_uppercase().starts_with("CREATE ")
            {
                return Err(DbError::msg(
                    "Each index must contain one CREATE INDEX statement.",
                ));
            }
            plan::index_name(index)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TableSnapshot {
    pub design: TableDesign,
    pub ddl: String,
    pub triggers: Vec<String>,
}

pub async fn load(r: &mut dyn Relational, scope: &Scope, name: &str) -> DbResult<TableSnapshot> {
    let dialect = r.dialect();
    let mut ddl = r.object_ddl(scope, ObjectKind::Table, name).await?;
    let mut design = parse::create(&ddl, name, dialect)?;
    if dialect == Dialect::Postgres {
        design.migrate_comments(scope)?;
        // The general DDL view omits identity/generated flags and nondefault collations.
        // Include them in both the editable draft and the concurrency fingerprint.
        let table = dialect.qualified(scope, name);
        let option_query = format!(
            "SELECT pg_get_userbyid(c.relowner), c.relpersistence, array_to_string(c.reloptions, ', '), t.spcname, (SELECT string_agg(quote_ident(n.nspname)||'.'||quote_ident(p.relname), ', ' ORDER BY i.inhseqno) FROM pg_inherits i JOIN pg_class p ON p.oid=i.inhparent JOIN pg_namespace n ON n.oid=p.relnamespace WHERE i.inhrelid=c.oid) FROM pg_class c LEFT JOIN pg_tablespace t ON t.oid=c.reltablespace WHERE c.oid={}::regclass",
            dialect.string_literal(&table)
        );
        let option_sets = r.execute(&option_query).await?;
        if let Some(row) = option_sets.iter().flat_map(|s| &s.rows).next() {
            let cell = |i: usize| row.get(i).and_then(|v| v.as_deref()).unwrap_or("");
            design.owner = cell(0).into();
            design.unlogged = cell(1) == "u";
            let mut options = Vec::new();
            if !cell(4).is_empty() {
                options.push(format!("INHERITS ({})", cell(4)));
            }
            if !cell(2).is_empty() {
                options.push(format!("WITH ({})", cell(2)));
            }
            if !cell(3).is_empty() {
                options.push(format!("TABLESPACE {}", dialect.quote_ident(cell(3))));
            }
            design.options = options.join(" ");
        }
        ddl.push_str(&format!(
            "\n-- Table options: {}",
            serde_json::to_string(&option_sets.iter().flat_map(|s| &s.rows).collect::<Vec<_>>())
                .unwrap_or_default()
        ));
        let query = format!(
            "SELECT a.attname, format_type(a.atttypid,a.atttypmod), a.attnotnull, pg_get_expr(ad.adbin,ad.adrelid), a.attidentity, a.attgenerated, CASE WHEN a.attcollation <> ty.typcollation THEN nc.nspname END, CASE WHEN a.attcollation <> ty.typcollation THEN co.collname END, col_description(a.attrelid,a.attnum) FROM pg_attribute a JOIN pg_type ty ON ty.oid=a.atttypid LEFT JOIN pg_attrdef ad ON ad.adrelid=a.attrelid AND ad.adnum=a.attnum LEFT JOIN pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_namespace nc ON nc.oid=co.collnamespace WHERE a.attrelid={}::regclass AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
            dialect.string_literal(&table)
        );
        let sets = r.execute(&query).await?;
        for row in sets.iter().flat_map(|s| &s.rows) {
            let cell = |i: usize| row.get(i).and_then(|v| v.as_deref()).unwrap_or("");
            let Some(column) = design.columns.iter_mut().find(|c| c.name == cell(0)) else {
                continue;
            };
            let mut definition = cell(1).to_string();
            if !cell(7).is_empty() {
                definition.push_str(&format!(
                    " COLLATE {}.{}",
                    dialect.quote_ident(cell(6)),
                    dialect.quote_ident(cell(7))
                ));
            }
            if !cell(5).is_empty() {
                definition.push_str(&format!(
                    " GENERATED ALWAYS AS ({}) {}",
                    cell(3),
                    if cell(5) == "s" { "STORED" } else { "VIRTUAL" }
                ));
            } else if !cell(4).is_empty() {
                definition.push_str(if cell(4) == "a" {
                    " GENERATED ALWAYS AS IDENTITY"
                } else {
                    " GENERATED BY DEFAULT AS IDENTITY"
                });
            }
            if cell(2) == "t" {
                definition.push_str(" NOT NULL");
            }
            if cell(4).is_empty() && cell(5).is_empty() && !cell(3).is_empty() {
                definition.push_str(&format!(" DEFAULT {}", cell(3)));
            }
            column.definition = definition;
            column.comment = cell(8).into();
        }
        ddl.push_str(&format!(
            "\n-- Column metadata: {}",
            serde_json::to_string(&sets.iter().flat_map(|s| s.rows.clone()).collect::<Vec<_>>())
                .unwrap_or_default()
        ));
    }
    let mut triggers = Vec::new();
    if dialect == Dialect::Sqlite {
        let sql = format!(
            "SELECT type, sql FROM {}.sqlite_schema WHERE tbl_name = {} AND type IN ('index', 'trigger') AND sql IS NOT NULL ORDER BY type, name",
            dialect.quote_ident(&scope.database),
            dialect.string_literal(name)
        );
        for set in r.execute(&sql).await? {
            for row in set.rows {
                let value = row.get(1).and_then(|v| v.clone()).unwrap_or_default();
                if row.first().and_then(|v| v.as_deref()) == Some("index") {
                    design.indexes.push(value);
                } else {
                    triggers.push(value);
                }
            }
        }
    }
    Ok(TableSnapshot {
        design,
        ddl,
        triggers,
    })
}

/// Table names in one SQLite database, used to pick a free `_table_old_YYYYMMDD` backup.
pub async fn sqlite_table_names(r: &mut dyn Relational, scope: &Scope) -> DbResult<Vec<String>> {
    let sql = format!(
        "SELECT name FROM {}.sqlite_schema WHERE type = 'table'",
        Dialect::Sqlite.quote_ident(&scope.database)
    );
    let mut names = Vec::new();
    for set in r.execute(&sql).await? {
        for row in set.rows {
            if let Some(name) = row.first().and_then(|value| value.clone()) {
                names.push(name);
            }
        }
    }
    Ok(names)
}

fn sqlite_rename(statements: &[String]) -> bool {
    statements.first().is_some_and(|sql| {
        let upper = sql.to_uppercase();
        upper.contains(" RENAME TO ") && !upper.contains("RENAME COLUMN")
    })
}

/// Applies precisely the previewed plan on a caller-owned session. Never retries DDL.
pub async fn apply(
    r: &mut dyn Relational,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
) -> DbResult<TableSnapshot> {
    let d = r.dialect();
    let mut legacy_before = before.cloned();
    if d == Dialect::Postgres
        && let Some(old) = &mut legacy_before
    {
        old.design.migrate_comments(scope)?;
    }
    let before = legacy_before.as_ref();
    if let Some(old) = before {
        let current = load(r, scope, &old.design.name).await?;
        if &current != old {
            return Err(DbError::msg(
                "The table changed outside this designer. Reload before saving.",
            ));
        }
    }
    match d {
        Dialect::MySql => {
            r.execute(&format!("USE {}", d.quote_ident(&scope.database)))
                .await?;
        }
        Dialect::Postgres => {
            r.execute(&format!(
                "SET search_path TO {}",
                d.quote_ident(scope.schema.as_deref().unwrap_or("public"))
            ))
            .await?;
        }
        Dialect::Sqlite if scope.database != "main" => {
            return Err(DbError::msg(
                "The table designer only supports the primary SQLite database.",
            ));
        }
        _ => {}
    }
    let occupied = if d == Dialect::Sqlite && before.is_some() {
        sqlite_table_names(r, scope).await?
    } else {
        Vec::new()
    };
    let statements = plan_with(d, scope, before, after, &occupied)?;
    if statements.is_empty() {
        return load(r, scope, &after.name).await;
    }
    let sqlite_rebuild = d == Dialect::Sqlite && before.is_some() && sqlite_rename(&statements);
    let foreign_keys = if sqlite_rebuild {
        r.execute("PRAGMA foreign_keys")
            .await?
            .first()
            .and_then(|s| s.rows.first())
            .and_then(|row| row.first())
            .and_then(|v| v.as_deref())
            .unwrap_or("0")
            == "1"
    } else {
        false
    };
    if sqlite_rebuild {
        r.execute("PRAGMA foreign_keys = OFF").await?;
    }
    let legacy = if sqlite_rebuild {
        let value = r
            .execute("PRAGMA legacy_alter_table")
            .await?
            .first()
            .and_then(|s| s.rows.first())
            .and_then(|r| r.first())
            .and_then(|v| v.as_deref())
            .unwrap_or("0")
            == "1";
        r.execute("PRAGMA legacy_alter_table = ON").await?;
        value
    } else {
        false
    };
    let transactional = d != Dialect::MySql;
    let result = async {
        if transactional {
            r.execute(if d == Dialect::Sqlite {
                "BEGIN IMMEDIATE"
            } else {
                "BEGIN"
            })
            .await?;
        }
        // Recheck inside the transaction/lock as well as before the preview is applied.
        if let Some(old) = before {
            if d == Dialect::Postgres {
                r.execute(&format!(
                    "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
                    d.qualified(scope, &old.design.name)
                ))
                .await?;
            }
            if load(r, scope, &old.design.name).await? != *old {
                return Err(DbError::msg(
                    "The table changed outside this designer. Reload before saving.",
                ));
            }
        }
        if sqlite_rebuild && let Some(old) = before {
            let removed: Vec<_> = old.design.columns.iter().filter(|c| !after.columns.iter().any(|n|n.original.as_ref()==Some(&c.name))).collect();
            if !removed.is_empty() {
                // SQLite validates view/trigger/index dependencies during native DROP COLUMN.
                // Roll this preflight back before copying data into the replacement table.
                r.execute("SAVEPOINT navidog_drop_preflight").await?;
                for column in removed {
                    if let Err(error) = r.execute(&format!("ALTER TABLE {} DROP COLUMN {}",d.qualified(scope,&old.design.name),d.quote_ident(&column.name))).await {
                        return Err(DbError::msg(format!("Cannot drop a field with dependent schema objects; remove those dependencies first: {error}")));
                    }
                }
                r.execute("ROLLBACK TO navidog_drop_preflight").await?;
                r.execute("RELEASE navidog_drop_preflight").await?;
            }
        }
        let sequence = if sqlite_rebuild
            && before.is_some_and(|old| old.ddl.to_uppercase().contains("AUTOINCREMENT"))
        {
            let query = format!(
                "SELECT seq FROM {}.sqlite_sequence WHERE name = {}",
                d.quote_ident(&scope.database),
                d.string_literal(&after.name)
            );
            r.execute(&query)
                .await?
                .first()
                .and_then(|s| s.rows.first())
                .and_then(|r| r.first())
                .and_then(|v| v.as_deref())
                .and_then(|v| v.parse::<u64>().ok())
        } else {
            None
        };
        for sql in &statements {
            r.execute(sql).await?;
        }
        if let Some(sequence) = sequence {
            let seq_table=format!("{}.sqlite_sequence",d.quote_ident(&scope.database));
            let name=d.string_literal(&after.name);
            r.execute(&format!("UPDATE {seq_table} SET seq = MAX(seq, {sequence}) WHERE name = {name}")).await?;
            r.execute(&format!("INSERT INTO {seq_table}(name,seq) SELECT {name},{sequence} WHERE NOT EXISTS(SELECT 1 FROM {seq_table} WHERE name={name})")).await?;
        }
        if sqlite_rebuild {
            let check = format!(
                "PRAGMA {}.foreign_key_check",
                d.quote_ident(&scope.database)
            );
            if r.execute(&check).await?.iter().any(|s| !s.rows.is_empty()) {
                return Err(DbError::msg(
                    "Foreign key check failed; the schema change was rolled back.",
                ));
            }
        }
        let saved = load(r, scope, &after.name).await?;
        if transactional {
            r.execute("COMMIT").await?;
        }
        Ok(saved)
    }
    .await;
    if result.is_err() && transactional {
        let _ = r.execute("ROLLBACK").await;
    }
    if sqlite_rebuild {
        let restore = r
            .execute(if foreign_keys {
                "PRAGMA foreign_keys = ON"
            } else {
                "PRAGMA foreign_keys = OFF"
            })
            .await;
        r.execute(if legacy {
            "PRAGMA legacy_alter_table = ON"
        } else {
            "PRAGMA legacy_alter_table = OFF"
        })
        .await?;
        if let Err(e) = restore {
            return Err(DbError::msg(format!(
                "Could not restore foreign key mode: {e}"
            )));
        }
    }
    result
}
