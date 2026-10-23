use super::parse::{ident, positions};
use super::*;

pub fn plan(
    d: Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
) -> DbResult<Vec<String>> {
    plan_on(d, scope, before, after, &[], &today())
}

/// Like [`plan`], but SQLite rebuilds avoid backup names already in `occupied`.
pub fn plan_with(
    d: Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
    occupied: &[String],
) -> DbResult<Vec<String>> {
    plan_on(d, scope, before, after, occupied, &today())
}

pub(super) fn plan_on(
    d: Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
    occupied: &[String],
    day: &str,
) -> DbResult<Vec<String>> {
    let mut after = after.clone();
    let mut before = before.cloned();
    if d == Dialect::Postgres {
        after.migrate_comments(scope)?;
        if let Some(old) = &mut before {
            old.design.migrate_comments(scope)?;
        }
    }
    let after = &after;
    let before = before.as_ref();
    after.validate(d)?;
    let Some(before) = before else {
        let mut out = vec![after.create_sql(d, scope, &after.name)?];
        out.extend(after.indexes.clone());
        if d == Dialect::Postgres {
            out.extend(comments::statements(scope, None, after));
        }
        if d == Dialect::Postgres && !after.owner.is_empty() {
            out.push(format!(
                "ALTER TABLE {} OWNER TO {}",
                d.qualified(scope, &after.name),
                d.quote_ident(&after.owner)
            ));
        }
        return Ok(out);
    };
    let old = &before.design;
    if old == after {
        return Ok(Vec::new());
    }
    if old.name != after.name {
        return Err(DbError::msg(
            "Rename the table from the object navigator before editing its structure.",
        ));
    }
    if after
        .columns
        .iter()
        .filter_map(|c| c.original.as_ref())
        .any(|name| !old.columns.iter().any(|c| &c.name == name))
    {
        return Err(DbError::msg("Unknown original column identity."));
    }
    if d == Dialect::Sqlite {
        if old.columns == after.columns
            && old.constraints == after.constraints
            && old.options == after.options
        {
            let mut statements = Vec::new();
            for sql in old
                .indexes
                .iter()
                .filter(|sql| !after.indexes.contains(sql))
            {
                statements.push(format!(
                    "DROP INDEX {}.{}",
                    d.quote_ident(&scope.database),
                    d.quote_ident(&index_name(sql)?)
                ));
            }
            statements.extend(
                after
                    .indexes
                    .iter()
                    .filter(|sql| !old.indexes.contains(sql))
                    .cloned(),
            );
            return Ok(statements);
        }
        return sqlite(scope, before, after, occupied, day);
    }
    let table = d.qualified(scope, &old.name);
    let mut out = Vec::new();
    // Drop changed constraints before columns; add replacements after columns exist.
    for constraint in old
        .constraints
        .iter()
        .filter(|c| !after.constraints.contains(c))
    {
        let (kind, name) = constraint_name(constraint)?;
        let drop = match (d, kind.as_str()) {
            (Dialect::MySql, "PRIMARY") => "DROP PRIMARY KEY".into(),
            (Dialect::MySql, "FOREIGN") => format!("DROP FOREIGN KEY {}", d.quote_ident(&name)),
            (Dialect::MySql, "INDEX") => format!("DROP INDEX {}", d.quote_ident(&name)),
            (Dialect::MySql, _) => {
                return Err(DbError::msg(
                    "Removing this constraint requires dialect-specific SQL; it has been preserved.",
                ));
            }
            _ => format!("DROP CONSTRAINT {}", d.quote_ident(&name)),
        };
        out.push(format!("ALTER TABLE {table} {drop}"));
    }
    for c in old.columns.iter().filter(|c| {
        !after
            .columns
            .iter()
            .any(|n| n.original.as_ref() == Some(&c.name))
    }) {
        out.push(format!(
            "ALTER TABLE {table} DROP COLUMN {}",
            d.quote_ident(&c.name)
        ));
    }
    for (ix, c) in after.columns.iter().enumerate() {
        let original = c
            .original
            .as_ref()
            .and_then(|name| old.columns.iter().find(|c| &c.name == name));
        let position = if d == Dialect::MySql {
            if ix == 0 {
                " FIRST".into()
            } else {
                format!(" AFTER {}", d.quote_ident(&after.columns[ix - 1].name))
            }
        } else {
            String::new()
        };
        match original {
            None => out.push(format!(
                "ALTER TABLE {table} ADD COLUMN {} {}{position}",
                d.quote_ident(&c.name),
                c.definition
            )),
            Some(previous) => {
                let moved = old.columns.iter().position(|v| v.name == previous.name) != Some(ix);
                if c.name == previous.name
                    && c.definition == previous.definition
                    && (!moved || d != Dialect::MySql)
                {
                    continue;
                }
                if d == Dialect::MySql {
                    out.push(format!(
                        "ALTER TABLE {table} CHANGE COLUMN {} {} {}{position}",
                        d.quote_ident(&previous.name),
                        d.quote_ident(&c.name),
                        c.definition
                    ));
                } else {
                    if moved {
                        return Err(DbError::msg(
                            "PostgreSQL does not support reordering existing columns.",
                        ));
                    }
                    if c.name != previous.name {
                        out.push(format!(
                            "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                            d.quote_ident(&previous.name),
                            d.quote_ident(&c.name)
                        ));
                    }
                    if c.definition != previous.definition {
                        if let (Some((a, b, mode, params)), Some((oa, ob, old_mode, old_params))) = (
                            properties::identity(&c.definition),
                            properties::identity(&previous.definition),
                        ) {
                            let stripped = format!("{}{}", &c.definition[..a], &c.definition[b..]);
                            let old_stripped = format!(
                                "{}{}",
                                &previous.definition[..oa],
                                &previous.definition[ob..]
                            );
                            if stripped == old_stripped && params == old_params && mode != old_mode
                            {
                                out.push(format!(
                                    "ALTER TABLE {table} ALTER COLUMN {} SET GENERATED {mode}",
                                    d.quote_ident(&c.name)
                                ));
                                continue;
                            }
                        }
                        let (ty, null, default) = pg_column(&c.definition)?;
                        let (old_ty, old_null, old_default) = pg_column(&previous.definition)?;
                        let target = format!(
                            "ALTER TABLE {table} ALTER COLUMN {}",
                            d.quote_ident(&c.name)
                        );
                        // Drop a default before changing its type; the server may not cast it.
                        if default != old_default || ty != old_ty {
                            out.push(format!("{target} DROP DEFAULT"));
                        }
                        if ty != old_ty {
                            out.push(format!("{target} TYPE {ty}"));
                        }
                        if null != old_null {
                            out.push(format!(
                                "{target} {} NOT NULL",
                                if null { "SET" } else { "DROP" }
                            ));
                        }
                        if (default != old_default || ty != old_ty)
                            && let Some(value) = default
                        {
                            out.push(format!("{target} SET DEFAULT {value}"));
                        }
                    }
                }
            }
        }
    }
    for c in after
        .constraints
        .iter()
        .filter(|c| !old.constraints.contains(c))
    {
        out.push(format!("ALTER TABLE {table} ADD {c}"));
    }
    out.extend(options::option_alters(
        d,
        &table,
        &old.options,
        &after.options,
    )?);
    if d == Dialect::Postgres && old.unlogged != after.unlogged {
        out.push(format!(
            "ALTER TABLE {table} SET {}",
            if after.unlogged { "UNLOGGED" } else { "LOGGED" }
        ));
    }
    for s in old.indexes.iter().filter(|s| !after.indexes.contains(s)) {
        let name = index_name(s)?;
        if d == Dialect::MySql {
            out.push(format!(
                "ALTER TABLE {table} DROP INDEX {}",
                d.quote_ident(&name)
            ));
            continue;
        }
        out.push(format!(
            "DROP INDEX {}{}",
            if d == Dialect::Postgres {
                format!(
                    "{}.",
                    d.quote_ident(scope.schema.as_deref().unwrap_or("public"))
                )
            } else {
                String::new()
            },
            d.quote_ident(&name)
        ));
    }
    out.extend(
        after
            .indexes
            .iter()
            .filter(|s| !old.indexes.contains(s))
            .cloned(),
    );
    if d == Dialect::Postgres {
        out.extend(comments::statements(scope, Some(old), after));
    }
    if d == Dialect::Postgres && old.owner != after.owner {
        if after.owner.is_empty() {
            return Err(DbError::msg("An existing PostgreSQL table needs an owner."));
        }
        out.push(format!(
            "ALTER TABLE {table} OWNER TO {}",
            d.quote_ident(&after.owner)
        ));
    }
    Ok(out)
}

fn constraint_name(sql: &str) -> DbResult<(String, String)> {
    let (first, rest) = ident(sql)?;
    let first = first.to_uppercase();
    if first == "PRIMARY" {
        return Ok(("PRIMARY".into(), "PRIMARY".into()));
    }
    if first == "CONSTRAINT" {
        let (name, rest) = ident(rest)?;
        let (kind, _) = ident(rest)?;
        return Ok((kind.to_uppercase(), name));
    }
    let rest = if matches!(first.as_str(), "UNIQUE" | "FULLTEXT" | "SPATIAL") {
        let (key, rest) = ident(rest)?;
        if !matches!(key.to_uppercase().as_str(), "KEY" | "INDEX") {
            return Err(DbError::msg("Cannot identify this constraint safely."));
        }
        rest
    } else if matches!(first.as_str(), "KEY" | "INDEX") {
        rest
    } else {
        return Err(DbError::msg("Cannot identify this constraint safely."));
    };
    Ok(("INDEX".into(), ident(rest)?.0))
}
pub(super) fn index_name(sql: &str) -> DbResult<String> {
    let (_, rest) = ident(sql)?;
    let (mut word, mut rest) = ident(rest)?;
    if word.eq_ignore_ascii_case("UNIQUE") {
        (word, rest) = ident(rest)?;
    }
    if !word.eq_ignore_ascii_case("INDEX") {
        return Err(DbError::msg(
            "Only CREATE INDEX statements are supported on the index page.",
        ));
    }
    let (name, rest) = ident(rest)?;
    if !rest.to_uppercase().starts_with("ON ") {
        return Err(DbError::msg("Expected CREATE INDEX name ON table."));
    }
    Ok(name)
}

fn trigger_name(sql: &str) -> DbResult<String> {
    let (_, rest) = ident(sql)?;
    let (mut word, mut rest) = ident(rest)?;
    if word.eq_ignore_ascii_case("TEMP") || word.eq_ignore_ascii_case("TEMPORARY") {
        (word, rest) = ident(rest)?;
    }
    if !word.eq_ignore_ascii_case("TRIGGER") {
        return Err(DbError::msg("Expected a CREATE TRIGGER statement."));
    }
    Ok(ident(rest)?.0)
}

fn today() -> String {
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// `_table_old_YYYYMMDD`, then `_1`, `_2`, when that name is already a table.
pub(super) fn backup_name(table: &str, day: &str, taken: &[String]) -> DbResult<String> {
    let base = format!("_{table}_old_{day}");
    if !taken.iter().any(|name| name.eq_ignore_ascii_case(&base)) {
        return Ok(base);
    }
    // ponytail: a few same-day leftovers are enough; past this the schema is already a mess.
    for n in 1..1000 {
        let name = format!("{base}_{n}");
        if !taken.iter().any(|taken| taken.eq_ignore_ascii_case(&name)) {
            return Ok(name);
        }
    }
    Err(DbError::msg(
        "Too many leftover SQLite backup tables for this date.",
    ))
}
fn pg_column(definition: &str) -> DbResult<(String, bool, Option<String>)> {
    let upper = definition.to_uppercase();
    for unsupported in [
        " GENERATED ",
        " COLLATE ",
        " REFERENCES ",
        " CHECK ",
        " UNIQUE",
        " PRIMARY KEY",
        " CONSTRAINT ",
    ] {
        if upper.contains(unsupported) {
            return Err(DbError::msg(
                "This PostgreSQL field has attributes that cannot be changed safely in the designer.",
            ));
        }
    }
    // Split only at top-level whitespace so types and default strings may contain keywords.
    let spaces = positions(definition, b' ')?;
    let marker = spaces.into_iter().find(|i| {
        let tail = definition[*i..].trim_start().to_uppercase();
        tail.starts_with("NOT NULL") || tail.starts_with("DEFAULT ") || tail == "NULL"
    });
    let Some(ix) = marker else {
        return Ok((definition.trim().into(), false, None));
    };
    let ty = definition[..ix].trim().to_string();
    let mut rest = definition[ix..].trim();
    let mut not_null = false;
    if rest.to_uppercase().starts_with("NOT NULL") {
        not_null = true;
        rest = rest[8..].trim();
    } else if rest.to_uppercase().starts_with("NULL") {
        rest = rest[4..].trim();
    }
    let default = if rest.is_empty() {
        None
    } else if rest.to_uppercase().starts_with("DEFAULT ") {
        Some(rest[8..].trim().into())
    } else {
        return Err(DbError::msg("Unsupported PostgreSQL field definition."));
    };
    Ok((ty, not_null, default))
}
fn sqlite(
    scope: &Scope,
    before: &TableSnapshot,
    after: &TableDesign,
    occupied: &[String],
    day: &str,
) -> DbResult<Vec<String>> {
    let d = Dialect::Sqlite;
    let old = &before.design;
    if table_option(&old.options, "without_rowid", d)?
        != table_option(&after.options, "without_rowid", d)?
    {
        return Err(DbError::msg(
            "Changing WITHOUT ROWID on an existing table can lose row identity; create a new table instead.",
        ));
    }
    if old.columns.iter().chain(&after.columns).any(|c| {
        c.definition.to_uppercase().contains("GENERATED")
            || c.definition.to_uppercase().contains(" AS ")
    }) {
        return Err(DbError::msg(
            "Rebuilding tables with generated columns is not supported; no changes were made.",
        ));
    }
    // Renaming fields also requires rewriting dependent constraint/index/trigger SQL. Refuse
    // rather than guessing at identifier occurrences inside expressions and string literals.
    let renamed = after
        .columns
        .iter()
        .any(|c| c.original.as_ref().is_some_and(|old| old != &c.name));
    if renamed {
        let rename_only = old.columns.len() == after.columns.len()
            && old.constraints == after.constraints
            && old.indexes == after.indexes
            && old.options == after.options
            && old.columns.iter().zip(&after.columns).all(|(old, new)| {
                new.original.as_ref() == Some(&old.name) && old.definition == new.definition
            });
        if !rename_only {
            return Err(DbError::msg(
                "Save SQLite field renames separately before making other schema changes.",
            ));
        }
        return Ok(after
            .columns
            .iter()
            .filter(|c| c.original.as_ref().is_some_and(|old| old != &c.name))
            .map(|c| {
                format!(
                    "ALTER TABLE {} RENAME COLUMN {} TO {}",
                    d.qualified(scope, &old.name),
                    d.quote_ident(c.original.as_ref().unwrap()),
                    d.quote_ident(&c.name)
                )
            })
            .collect());
    }
    let backup = backup_name(&after.name, day, occupied)?;
    let table = d.qualified(scope, &old.name);
    let mut out = vec![format!(
        "ALTER TABLE {table} RENAME TO {}",
        d.quote_ident(&backup)
    )];
    out.push(after.create_sql(d, scope, &after.name)?);
    let mut destination = Vec::new();
    let mut source = Vec::new();
    for c in &after.columns {
        if let Some(old) = &c.original {
            destination.push(d.quote_ident(&c.name));
            source.push(d.quote_ident(old));
        }
    }
    let without = old.options.to_uppercase().contains("WITHOUT ROWID")
        || after.options.to_uppercase().contains("WITHOUT ROWID");
    if !without {
        let alias = ["rowid", "_rowid_", "oid"]
            .into_iter()
            .find(|n| {
                !old.columns
                    .iter()
                    .chain(&after.columns)
                    .any(|c| c.name.eq_ignore_ascii_case(n))
            })
            .ok_or_else(|| {
                DbError::msg(
                    "All SQLite rowid aliases are shadowed; rebuilding would lose row identity.",
                )
            })?;
        // Do not also insert rowid when a copied INTEGER primary key already supplies it.
        let copies_alias = after.columns.iter().any(|c| {
            if c.original.is_none()
                || c.definition
                    .split_whitespace()
                    .next()
                    .is_none_or(|ty| !ty.eq_ignore_ascii_case("INTEGER"))
            {
                return false;
            }
            let normalized = c
                .definition
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase();
            let inline =
                normalized.contains("PRIMARY KEY") && !normalized.contains("PRIMARY KEY DESC");
            inline
                || after.constraints.iter().any(|key| {
                    let Ok((head, mut rest)) = ident(key) else {
                        return false;
                    };
                    let head = if head.eq_ignore_ascii_case("CONSTRAINT") {
                        let Ok((_, next)) = ident(rest) else {
                            return false;
                        };
                        let Ok((head, next)) = ident(next) else {
                            return false;
                        };
                        rest = next;
                        head
                    } else {
                        head
                    };
                    if !head.eq_ignore_ascii_case("PRIMARY") {
                        return false;
                    }
                    let Some((_, body)) = rest.split_once('(') else {
                        return false;
                    };
                    let Some((body, _)) = body.rsplit_once(')') else {
                        return false;
                    };
                    if !positions(body, b',').is_ok_and(|commas| commas.is_empty()) {
                        return false;
                    }
                    ident(body).is_ok_and(|(name, _)| name == c.name)
                })
        });
        if !copies_alias {
            destination.push(d.quote_ident(alias));
            source.push(d.quote_ident(alias));
        }
    }
    if destination.is_empty() {
        return Err(DbError::msg(
            "Keep at least one original column when rebuilding an existing table.",
        ));
    }
    out.push(format!(
        "INSERT INTO {table} ({}) SELECT {} FROM {}",
        destination.join(", "),
        source.join(", "),
        d.qualified(scope, &backup)
    ));
    // Index and trigger names are schema-wide. They still belong to the renamed
    // table, so drop those objects and create them on the new table. The backup
    // table itself stays, with its rows.
    for sql in &old.indexes {
        out.push(format!(
            "DROP INDEX {}.{}",
            d.quote_ident(&scope.database),
            d.quote_ident(&index_name(sql)?)
        ));
    }
    for sql in &before.triggers {
        out.push(format!(
            "DROP TRIGGER {}.{}",
            d.quote_ident(&scope.database),
            d.quote_ident(&trigger_name(sql)?)
        ));
    }
    out.extend(after.indexes.clone());
    out.extend(before.triggers.clone());
    Ok(out)
}
