use super::parse::{ident, positions};
use super::*;
use navidog_sql::dialect::ConstraintKind;

pub fn plan(
    d: &dyn Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
) -> DbResult<Vec<String>> {
    plan_on(d, scope, before, after, &[], &today())
}

/// Like [`plan`], but SQLite rebuilds avoid backup names already in `occupied`.
pub fn plan_with(
    d: &dyn Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
    occupied: &[String],
) -> DbResult<Vec<String>> {
    plan_on(d, scope, before, after, occupied, &today())
}

pub(super) fn plan_on(
    d: &dyn Dialect,
    scope: &Scope,
    before: Option<&TableSnapshot>,
    after: &TableDesign,
    occupied: &[String],
    day: &str,
) -> DbResult<Vec<String>> {
    let mut after = after.clone();
    let mut before = before.cloned();
    if d.features().separate_comments {
        after.migrate_comments(scope, d)?;
        if let Some(old) = &mut before {
            old.design.migrate_comments(scope, d)?;
        }
    }
    let after = &after;
    let before = before.as_ref();
    after.validate(d)?;
    let Some(before) = before else {
        let mut out = vec![after.create_sql(d, scope, &after.name)?];
        out.extend(after.indexes.clone());
        if d.features().separate_comments {
            out.extend(comments::statements(scope, None, after, d));
        }
        if let Some(sql) = d.table_owner_sql(&d.qualified(scope, &after.name), &after.owner)
            && !after.owner.is_empty()
        {
            out.push(sql);
        }
        out.extend(after.partition_children.clone());
        out.extend(trigger_creates(after));
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
    if d.features().rebuilds_tables {
        if old.columns == after.columns
            && old.constraints == after.constraints
            && old.options == after.options
            && old.partition == after.partition
            && old.partition_children == after.partition_children
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
            statements.extend(trigger_diff(
                d,
                scope,
                &old.name,
                &resolved_triggers(old, Some(before)),
                &resolved_triggers(after, Some(before)),
            )?);
            return Ok(statements);
        }
        return sqlite(d, scope, before, after, occupied, day);
    }
    if !d.features().supports_column_position && columns_reordered(old, after) {
        return postgres_rebuild(d, scope, before, after);
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
        let drop = d.drop_constraint_clause(kind, &name).ok_or_else(|| {
            DbError::msg(
                "Removing this constraint requires dialect-specific SQL; it has been preserved.",
            )
        })?;
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
        let position = if d.features().supports_column_position {
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
                    && (!moved || !d.features().supports_column_position)
                {
                    continue;
                }
                if d.features().rewrites_columns {
                    out.push(format!(
                        "ALTER TABLE {table} CHANGE COLUMN {} {} {}{position}",
                        d.quote_ident(&previous.name),
                        d.quote_ident(&c.name),
                        c.definition
                    ));
                } else {
                    if c.name != previous.name {
                        out.push(format!(
                            "ALTER TABLE {table} RENAME COLUMN {} TO {}",
                            d.quote_ident(&previous.name),
                            d.quote_ident(&c.name)
                        ));
                    }
                    if c.definition != previous.definition {
                        if let Some(statements) =
                            alter_identity(d, &table, &c.name, &previous.definition, &c.definition)?
                        {
                            out.extend(statements);
                            continue;
                        }
                        let (ty, null, default) = pg_column(&c.definition, d)?;
                        let (old_ty, old_null, old_default) = pg_column(&previous.definition, d)?;
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
    if d.features().supports_unlogged && old.unlogged != after.unlogged {
        out.push(format!(
            "ALTER TABLE {table} SET {}",
            if after.unlogged { "UNLOGGED" } else { "LOGGED" }
        ));
    }
    for s in old.indexes.iter().filter(|s| !after.indexes.contains(s)) {
        let name = index_name(s)?;
        if d.features().drop_index_alters_table {
            out.push(format!(
                "ALTER TABLE {table} DROP INDEX {}",
                d.quote_ident(&name)
            ));
            continue;
        }
        out.push(format!(
            "DROP INDEX {}{}",
            if d.features().has_schemas {
                format!(
                    "{}.",
                    d.quote_ident(
                        scope
                            .schema
                            .as_deref()
                            .or(d.default_schema())
                            .unwrap_or_default()
                    )
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
    if d.features().separate_comments {
        out.extend(comments::statements(scope, Some(old), after, d));
    }
    if old.owner != after.owner
        && let Some(sql) = d.table_owner_sql(&table, &after.owner)
    {
        if after.owner.is_empty() {
            return Err(DbError::msg("An existing PostgreSQL table needs an owner."));
        }
        out.push(sql);
    }
    out.extend(partition_alters(d, scope, old, after)?);
    out.extend(trigger_diff(
        d,
        scope,
        &old.name,
        &resolved_triggers(old, Some(before)),
        &resolved_triggers(after, Some(before)),
    )?);
    Ok(out)
}

/// The constraint's kind and name, from its `CREATE TABLE` clause text.
fn constraint_name(sql: &str) -> DbResult<(ConstraintKind, String)> {
    let (first, rest) = ident(sql)?;
    let first = first.to_uppercase();
    if first == "PRIMARY" {
        return Ok((ConstraintKind::Primary, "PRIMARY".into()));
    }
    if first == "CONSTRAINT" {
        let (name, rest) = ident(rest)?;
        let (kind, _) = ident(rest)?;
        let kind = match kind.to_uppercase().as_str() {
            "PRIMARY" => ConstraintKind::Primary,
            "FOREIGN" => ConstraintKind::ForeignKey,
            "UNIQUE" => ConstraintKind::Unique,
            "CHECK" => ConstraintKind::Check,
            "EXCLUDE" => ConstraintKind::Exclude,
            _ => return Err(DbError::msg("Cannot identify this constraint safely.")),
        };
        return Ok((kind, name));
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
    Ok((ConstraintKind::Index, ident(rest)?.0))
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
fn resolved_triggers(design: &TableDesign, snapshot: Option<&TableSnapshot>) -> Vec<String> {
    if let Some(triggers) = &design.triggers {
        return triggers.clone();
    }
    snapshot
        .map(|snapshot| snapshot.triggers.clone())
        .unwrap_or_default()
}

fn trigger_creates(after: &TableDesign) -> Vec<String> {
    after.triggers.clone().unwrap_or_default()
}

fn trigger_diff(
    d: &dyn Dialect,
    scope: &Scope,
    table: &str,
    old: &[String],
    new: &[String],
) -> DbResult<Vec<String>> {
    let mut out = Vec::new();
    let qualified = d.qualified(scope, table);
    for sql in old {
        if new.iter().any(|kept| kept == sql) {
            continue;
        }
        let name = trigger_name(sql)?;
        out.push(if d.features().rebuilds_tables {
            format!(
                "DROP TRIGGER {}.{}",
                d.quote_ident(&scope.database),
                d.quote_ident(&name)
            )
        } else if d.features().enriches_from_catalog {
            format!("DROP TRIGGER {} ON {qualified}", d.quote_ident(&name))
        } else {
            format!("DROP TRIGGER {}", d.quote_ident(&name))
        });
    }
    for sql in new {
        if !old.iter().any(|kept| kept == sql) {
            out.push(sql.clone());
        }
    }
    Ok(out)
}

fn partition_alters(
    d: &dyn Dialect,
    scope: &Scope,
    old: &TableDesign,
    after: &TableDesign,
) -> DbResult<Vec<String>> {
    let mut out = Vec::new();
    if old.partition != after.partition {
        if !after.partition.trim().is_empty() && !d.features().supports_partitions {
            return Err(DbError::msg("This database cannot partition tables."));
        }
        if d.features().inline_table_options {
            let table = d.qualified(scope, &after.name);
            if after.partition.trim().is_empty() {
                out.push(format!("ALTER TABLE {table} REMOVE PARTITIONING"));
            } else {
                out.push(format!("ALTER TABLE {table} {}", after.partition.trim()));
            }
        } else if !old.partition.is_empty() || !after.partition.is_empty() {
            return Err(DbError::msg(
                "Changing the partition method of an existing table is not supported.",
            ));
        }
    }
    if d.features().supports_partitions && !d.features().inline_table_options {
        for child in &old.partition_children {
            if after.partition_children.contains(child) {
                continue;
            }
            let name = child_table_name(child)?;
            out.push(format!("DROP TABLE {}", d.qualified(scope, &name)));
        }
        for child in &after.partition_children {
            if !old.partition_children.contains(child) {
                out.push(child.clone());
            }
        }
    }
    Ok(out)
}

fn child_table_name(sql: &str) -> DbResult<String> {
    let mut rest = sql.trim();
    for word in ["CREATE", "TABLE"] {
        if rest.len() < word.len() || !rest[..word.len()].eq_ignore_ascii_case(word) {
            return Err(DbError::msg("Expected a CREATE TABLE statement."));
        }
        rest = rest[word.len()..].trim_start();
    }
    let (name, rest) = ident(rest)?;
    if let Some(rest) = rest.strip_prefix('.') {
        let (name, _) = ident(rest)?;
        return Ok(name);
    }
    Ok(name)
}

fn columns_reordered(old: &TableDesign, after: &TableDesign) -> bool {
    let current: Vec<&str> = after
        .columns
        .iter()
        .filter_map(|c| c.original.as_deref())
        .collect();
    let previous: Vec<&str> = old
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .filter(|name| current.contains(name))
        .collect();
    current != previous
}

fn rebuild_note(sql: String, drop_rowid: bool) -> String {
    let extra = if drop_rowid {
        " Changing WITHOUT ROWID drops the implicit rowid."
    } else {
        ""
    };
    format!("-- Rebuilding the whole table.{extra}\n{sql}")
}

fn strip_identity(definition: &str, identity: Option<&(usize, usize, String, String)>) -> String {
    match identity {
        Some((start, end, _, _)) => format!("{}{}", &definition[..*start], &definition[*end..]),
        None => definition.to_string(),
    }
}

/// Identity add, drop, and mode switch. Sequence options must stay as loaded.
/// `None` means the column has no identity on either side.
fn alter_identity(
    d: &dyn Dialect,
    table: &str,
    column: &str,
    old_def: &str,
    new_def: &str,
) -> DbResult<Option<Vec<String>>> {
    let new_id = properties::identity(new_def, d);
    let old_id = properties::identity(old_def, d);
    if new_id.is_none() && old_id.is_none() {
        return Ok(None);
    }
    if let (Some(new_id), Some(old_id)) = (&new_id, &old_id) {
        if new_id.3 != old_id.3 {
            return Err(DbError::msg(
                "Changing identity sequence options is not supported.",
            ));
        }
    } else if new_id.as_ref().is_some_and(|id| !id.3.is_empty()) {
        return Err(DbError::msg(
            "Changing identity sequence options is not supported.",
        ));
    }
    let new_rest = strip_identity(new_def, new_id.as_ref());
    let old_rest = strip_identity(old_def, old_id.as_ref());
    let mut out = Vec::new();
    if new_rest.trim() != old_rest.trim() {
        out.extend(pg_column_alters(
            d,
            table,
            column,
            old_rest.trim(),
            new_rest.trim(),
        )?);
    }
    let target = format!("ALTER TABLE {table} ALTER COLUMN {}", d.quote_ident(column));
    match (&old_id, &new_id) {
        (None, Some(id)) => out.push(format!("{target} ADD GENERATED {} AS IDENTITY", id.2)),
        (Some(_), None) => out.push(format!("{target} DROP IDENTITY")),
        (Some(old_id), Some(new_id)) if old_id.2 != new_id.2 => {
            out.push(format!("{target} SET GENERATED {}", new_id.2));
        }
        _ => {}
    }
    Ok(Some(out))
}

fn pg_column_alters(
    d: &dyn Dialect,
    table: &str,
    column: &str,
    old_def: &str,
    new_def: &str,
) -> DbResult<Vec<String>> {
    let (ty, null, default) = pg_column(new_def, d)?;
    let (old_ty, old_null, old_default) = pg_column(old_def, d)?;
    let target = format!("ALTER TABLE {table} ALTER COLUMN {}", d.quote_ident(column));
    let mut out = Vec::new();
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
    Ok(out)
}

/// PRIMARY KEY, UNIQUE and EXCLUDE names are schema-wide indexes. The old table
/// still owns them while the copy is created, so the copy uses a temporary name.
fn stage_index_constraints(design: &mut TableDesign, d: &dyn Dialect) -> Vec<(String, String)> {
    let mut renames = Vec::new();
    for constraint in &mut design.constraints {
        let Ok((head, rest)) = ident(constraint) else {
            continue;
        };
        if !head.eq_ignore_ascii_case("CONSTRAINT") {
            continue;
        }
        let Ok((name, after)) = ident(rest) else {
            continue;
        };
        let kind = after.split_whitespace().next().unwrap_or("");
        if !matches!(
            kind.to_ascii_uppercase().as_str(),
            "PRIMARY" | "UNIQUE" | "EXCLUDE"
        ) {
            continue;
        }
        let temp = temp_constraint_name(&name);
        *constraint = format!("CONSTRAINT {} {after}", d.quote_ident(&temp));
        renames.push((temp, name));
    }
    renames
}

fn temp_constraint_name(name: &str) -> String {
    const SUFFIX: &str = "__navidog";
    let mut base: String = name.chars().collect();
    while base.len() + SUFFIX.len() > 63 {
        base.pop();
    }
    format!("{base}{SUFFIX}")
}

/// `(column, quoted sequence, nextval expression)`.
fn nextval_bindings(design: &TableDesign, d: &dyn Dialect) -> Vec<(String, String, String)> {
    design
        .columns
        .iter()
        .filter_map(|column| {
            // A new column has no `original`. Taking `OWNED BY` on its sequence can
            // steal one that belongs to another table. An existing column may still
            // point at a shared sequence: the planner has no ownership catalog.
            column.original.as_ref()?;
            let (raw, expr) = nextval_call(&column.definition)?;
            Some((column.name.clone(), quote_sequence(d, &raw), expr))
        })
        .collect()
}

fn nextval_call(definition: &str) -> Option<(String, String)> {
    let lower = definition.to_ascii_lowercase();
    let start = lower.find("nextval(")?;
    let mut rest = definition[start + "nextval(".len()..].trim_start();
    if !rest.starts_with('\'') {
        return None;
    }
    rest = &rest[1..];
    let mut raw = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '\'' if chars.as_str().starts_with('\'') => {
                chars.next();
                raw.push('\'');
            }
            '\'' => break,
            c => raw.push(c),
        }
    }
    let after = chars.as_str();
    let close = after.find(')')?;
    let expr_end = definition.len() - after.len() + close + 1;
    Some((raw, definition[start..expr_end].to_string()))
}

fn quote_sequence(d: &dyn Dialect, raw: &str) -> String {
    match raw.split_once('.') {
        Some((schema, name)) => format!(
            "{}.{}",
            d.quote_ident(&unquote_ident(schema)),
            d.quote_ident(&unquote_ident(name))
        ),
        None => d.quote_ident(&unquote_ident(raw)),
    }
}

fn unquote_ident(raw: &str) -> String {
    let raw = raw.trim();
    let quote = raw.chars().next();
    if matches!(quote, Some('"' | '`')) && raw.ends_with(quote.unwrap()) && raw.len() >= 2 {
        let inner = &raw[1..raw.len() - 1];
        inner.replace("\"\"", "\"").replace("``", "`")
    } else {
        raw.to_string()
    }
}

fn postgres_rebuild(
    d: &dyn Dialect,
    scope: &Scope,
    before: &TableSnapshot,
    after: &TableDesign,
) -> DbResult<Vec<String>> {
    let old = &before.design;
    let mut staged = after.clone();
    let renames = stage_index_constraints(&mut staged, d);
    let sequences = nextval_bindings(after, d);
    let temp = format!("{}__navidog_new", old.name);
    let mut out = vec![rebuild_note(staged.create_sql(d, scope, &temp)?, false)];
    let mut destination = Vec::new();
    let mut source = Vec::new();
    let mut identity = false;
    for column in &after.columns {
        let Some(original) = &column.original else {
            continue;
        };
        if properties::is_generated(&column.definition, d) {
            continue;
        }
        if properties::identity(&column.definition, d).is_some() {
            identity = true;
        }
        destination.push(d.quote_ident(&column.name));
        source.push(d.quote_ident(original));
    }
    if !destination.is_empty() {
        let overriding = if identity {
            " OVERRIDING SYSTEM VALUE"
        } else {
            ""
        };
        out.push(format!(
            "INSERT INTO {} ({}){overriding} SELECT {} FROM {}",
            d.qualified(scope, &temp),
            destination.join(", "),
            source.join(", "),
            d.qualified(scope, &old.name)
        ));
    }
    let temp_table = d.qualified(scope, &temp);
    for (column, sequence, _) in &sequences {
        // The copy's DEFAULT points at the old table's sequence. Drop it, and
        // clear OWNED BY, before DROP TABLE or the sequence is dropped with the old table.
        out.push(format!(
            "ALTER TABLE {temp_table} ALTER COLUMN {} DROP DEFAULT",
            d.quote_ident(column)
        ));
        out.push(format!("ALTER SEQUENCE {sequence} OWNED BY NONE"));
    }
    out.push(format!("DROP TABLE {}", d.qualified(scope, &old.name)));
    out.push(format!(
        "ALTER TABLE {temp_table} RENAME TO {}",
        d.quote_ident(&old.name)
    ));
    let renamed = d.qualified(scope, &old.name);
    for (column, sequence, expr) in &sequences {
        out.push(format!(
            "ALTER TABLE {renamed} ALTER COLUMN {} SET DEFAULT {expr}",
            d.quote_ident(column)
        ));
        out.push(format!(
            "ALTER SEQUENCE {sequence} OWNED BY {renamed}.{}",
            d.quote_ident(column)
        ));
    }
    for (temp_name, original) in &renames {
        out.push(format!(
            "ALTER TABLE {renamed} RENAME CONSTRAINT {} TO {}",
            d.quote_ident(temp_name),
            d.quote_ident(original)
        ));
    }
    out.extend(after.indexes.clone());
    out.extend(resolved_triggers(after, Some(before)));
    for column in &after.columns {
        if column.original.is_none() || properties::identity(&column.definition, d).is_none() {
            continue;
        }
        let table = d.qualified(scope, &old.name);
        let quoted = d.quote_ident(&column.name);
        out.push(format!(
            "SELECT setval(pg_get_serial_sequence({}, {}), COALESCE((SELECT MAX({quoted}) FROM {table}), 1), (SELECT MAX({quoted}) FROM {table}) IS NOT NULL)",
            d.string_literal(&d.qualified(scope, &old.name)),
            d.string_literal(&column.name),
        ));
    }
    if d.features().separate_comments {
        out.extend(comments::statements(scope, None, after, d));
    }
    if !after.owner.is_empty()
        && let Some(sql) = d.table_owner_sql(&d.qualified(scope, &old.name), &after.owner)
    {
        out.push(sql);
    }
    Ok(out)
}

fn pg_column(definition: &str, d: &dyn Dialect) -> DbResult<(String, bool, Option<String>)> {
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
    let spaces = positions(definition, b' ', d)?;
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
    d: &dyn Dialect,
    scope: &Scope,
    before: &TableSnapshot,
    after: &TableDesign,
    occupied: &[String],
    day: &str,
) -> DbResult<Vec<String>> {
    let old = &before.design;
    let old_without = table_option(&old.options, "without_rowid", d)? == "true";
    let new_without = table_option(&after.options, "without_rowid", d)? == "true";
    // Renaming fields also requires rewriting dependent constraint/index/trigger SQL.
    // A rename by itself uses SQLite's own rewrite. Replacing changed triggers first,
    // while the old names still exist, lets that same rewrite update the new bodies.
    // Rebuilding instead would keep a backup whose foreign keys still bind live
    // parents, and would recreate untouched triggers with the old column names.
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
        if rename_only {
            let mut out = trigger_diff(
                d,
                scope,
                &old.name,
                &resolved_triggers(old, Some(before)),
                &resolved_triggers(after, Some(before)),
            )?;
            out.extend(after.columns.iter().filter_map(|c| {
                let previous = c.original.as_ref()?;
                (previous != &c.name).then(|| {
                    format!(
                        "ALTER TABLE {} RENAME COLUMN {} TO {}",
                        d.qualified(scope, &old.name),
                        d.quote_ident(previous),
                        d.quote_ident(&c.name)
                    )
                })
            }));
            return Ok(out);
        }
    }
    let backup = backup_name(&after.name, day, occupied)?;
    let table = d.qualified(scope, &old.name);
    let mut out = vec![rebuild_note(
        format!("ALTER TABLE {table} RENAME TO {}", d.quote_ident(&backup)),
        !old_without && new_without,
    )];
    out.push(after.create_sql(d, scope, &after.name)?);
    let mut destination = Vec::new();
    let mut source = Vec::new();
    for c in &after.columns {
        if let Some(old_name) = &c.original {
            if properties::is_generated(&c.definition, d) {
                continue;
            }
            destination.push(d.quote_ident(&c.name));
            source.push(d.quote_ident(old_name));
        }
    }
    let without = new_without || old_without;
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
                    if !positions(body, b',', d).is_ok_and(|commas| commas.is_empty()) {
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
    out.extend(resolved_triggers(after, Some(before)));
    Ok(out)
}
