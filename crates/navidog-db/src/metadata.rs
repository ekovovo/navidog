//! Metadata queries (20-objects.md §3). Every dialect's SQL lives here and runs through
//! `Relational::execute`; drivers only declare their dialect.

use navidog_core::object::{ObjectKind, Scope};

use crate::dialect::Dialect;
use crate::driver::{
    ColumnInfo, DbError, DbResult, ObjectInfo, Relational, ResultSet, TableChildren,
};

async fn query<R: Relational + ?Sized>(r: &mut R, sql: &str) -> DbResult<ResultSet> {
    let mut sets = r.execute(sql).await?;
    sets.drain(..)
        .find(|s| !s.columns.is_empty())
        .ok_or_else(|| DbError::msg("查询没有返回结果集"))
}

fn cell(row: &[Option<String>], ix: usize) -> String {
    row.get(ix).cloned().flatten().unwrap_or_default()
}

pub async fn list_schemas<R: Relational + ?Sized>(r: &mut R) -> DbResult<Vec<String>> {
    if !r.dialect().has_schemas() {
        return Ok(Vec::new());
    }
    // System schemas (pg_catalog, information_schema) are listed; the navigator hides them
    // unless "Show Hidden Items" is on. Toast and temp schemas are internal and never shown.
    let sql = "SELECT nspname FROM pg_namespace \
               WHERE nspname NOT LIKE 'pg\\_toast%' AND nspname NOT LIKE 'pg\\_temp%' ORDER BY nspname";
    Ok(query(r, sql)
        .await?
        .rows
        .iter()
        .map(|row| cell(row, 0))
        .collect())
}

/// Object list. `properties` keys are stable English names; the UI translates them into
/// column headers.
pub async fn list_objects<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<Vec<ObjectInfo>> {
    let d = r.dialect();
    let (sql, props): (String, &[&str]) = match d {
        Dialect::MySql => mysql_objects(d, scope, kind)?,
        Dialect::Postgres => pg_objects(d, scope, kind)?,
        Dialect::Sqlite => sqlite_objects(d, scope, kind)?,
    };
    let set = query(r, &sql).await?;
    Ok(set
        .rows
        .iter()
        .map(|row| {
            let properties = props
                .iter()
                .enumerate()
                .map(|(i, k)| (k.to_string(), cell(row, i + 1)))
                .filter(|(_, v)| !v.is_empty())
                .collect::<Vec<_>>();
            let comment = properties
                .iter()
                .find(|(k, _)| k == "comment")
                .map(|(_, v)| v.clone());
            ObjectInfo {
                name: cell(row, 0),
                kind,
                comment,
                properties,
            }
        })
        .collect())
}

fn unsupported(kind: ObjectKind) -> DbError {
    DbError::msg(format!("不支持的对象分类: {}", kind.key()))
}

fn mysql_objects(
    d: Dialect,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<(String, &'static [&'static str])> {
    let db = d.string_literal(&scope.database);
    Ok(match kind {
        ObjectKind::Table => (
            format!(
                "SELECT TABLE_NAME, TABLE_ROWS, DATA_LENGTH, ENGINE, CREATE_TIME, UPDATE_TIME, TABLE_COLLATION, TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = {db} AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME"
            ),
            &[
                "rows",
                "data_length",
                "engine",
                "created",
                "modified",
                "collation",
                "comment",
            ],
        ),
        ObjectKind::View => (
            format!(
                "SELECT TABLE_NAME, DEFINER, SECURITY_TYPE, CHECK_OPTION, IS_UPDATABLE \
                 FROM information_schema.VIEWS WHERE TABLE_SCHEMA = {db} ORDER BY TABLE_NAME"
            ),
            &["definer", "security", "check_option", "updatable"],
        ),
        ObjectKind::Function => (
            format!(
                "SELECT ROUTINE_NAME, ROUTINE_TYPE, DTD_IDENTIFIER, CREATED, LAST_ALTERED, ROUTINE_COMMENT \
                 FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = {db} ORDER BY ROUTINE_NAME"
            ),
            &["type", "returns", "created", "modified", "comment"],
        ),
        ObjectKind::Event => (
            format!(
                "SELECT EVENT_NAME, STATUS, EVENT_TYPE, EXECUTE_AT, INTERVAL_VALUE, INTERVAL_FIELD, EVENT_COMMENT \
                 FROM information_schema.EVENTS WHERE EVENT_SCHEMA = {db} ORDER BY EVENT_NAME"
            ),
            &[
                "status",
                "type",
                "execute_at",
                "interval",
                "interval_field",
                "comment",
            ],
        ),
        other => return Err(unsupported(other)),
    })
}

fn pg_relkind(kind: ObjectKind) -> Option<&'static str> {
    Some(match kind {
        ObjectKind::Table => "'r', 'p'",
        ObjectKind::View => "'v'",
        ObjectKind::MaterializedView => "'m'",
        ObjectKind::Sequence => "'S'",
        _ => return None,
    })
}

fn pg_objects(
    d: Dialect,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<(String, &'static [&'static str])> {
    let schema = d.string_literal(scope.schema.as_deref().unwrap_or("public"));
    if kind == ObjectKind::Function {
        return Ok((
            format!(
                "SELECT p.proname, CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END, \
                 pg_get_function_identity_arguments(p.oid), pg_get_function_result(p.oid), \
                 pg_get_userbyid(p.proowner), obj_description(p.oid, 'pg_proc') \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = {schema} AND p.prokind IN ('f', 'p') ORDER BY p.proname"
            ),
            &["type", "arguments", "returns", "owner", "comment"],
        ));
    }
    let relkind = pg_relkind(kind).ok_or_else(|| unsupported(kind))?;
    Ok((
        format!(
            "SELECT c.relname, CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::bigint END, \
             pg_total_relation_size(c.oid), pg_get_userbyid(c.relowner), obj_description(c.oid, 'pg_class') \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {schema} AND c.relkind IN ({relkind}) ORDER BY c.relname"
        ),
        &["rows", "data_length", "owner", "comment"],
    ))
}

fn sqlite_objects(
    d: Dialect,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<(String, &'static [&'static str])> {
    let ty = match kind {
        ObjectKind::Table => "table",
        ObjectKind::View => "view",
        ObjectKind::Index => "index",
        ObjectKind::Trigger => "trigger",
        other => return Err(unsupported(other)),
    };
    Ok((
        format!(
            "SELECT name, tbl_name FROM {}.sqlite_schema \
             WHERE type = '{ty}' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND sql IS NOT NULL ORDER BY name",
            d.quote_ident(&scope.database)
        ),
        &["table"],
    ))
}

pub async fn object_ddl<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
    name: &str,
) -> DbResult<String> {
    let d = r.dialect();
    match d {
        Dialect::MySql => {
            let target = d.qualified(scope, name);
            let (sql, col) = match kind {
                ObjectKind::Table => (format!("SHOW CREATE TABLE {target}"), 1),
                ObjectKind::View => (format!("SHOW CREATE VIEW {target}"), 1),
                ObjectKind::Event => (format!("SHOW CREATE EVENT {target}"), 3),
                ObjectKind::Function => {
                    let ty = query(
                        r,
                        &format!(
                            "SELECT ROUTINE_TYPE FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = {} AND ROUTINE_NAME = {}",
                            d.string_literal(&scope.database),
                            d.string_literal(name)
                        ),
                    )
                    .await?;
                    let ty = ty
                        .rows
                        .first()
                        .map(|r| cell(r, 0))
                        .unwrap_or_else(|| "FUNCTION".into());
                    (format!("SHOW CREATE {ty} {target}"), 2)
                }
                other => return Err(unsupported(other)),
            };
            let set = query(r, &sql).await?;
            Ok(set
                .rows
                .first()
                .map(|row| cell(row, col))
                .unwrap_or_default()
                + ";")
        }
        Dialect::Postgres => pg_ddl(r, scope, kind, name).await,
        Dialect::Sqlite => {
            let set = query(
                r,
                &format!(
                    "SELECT sql FROM {}.sqlite_schema WHERE name = {}",
                    d.quote_ident(&scope.database),
                    d.string_literal(name)
                ),
            )
            .await?;
            Ok(set.rows.first().map(|row| cell(row, 0)).unwrap_or_default() + ";")
        }
    }
}

async fn pg_ddl<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
    name: &str,
) -> DbResult<String> {
    let d = Dialect::Postgres;
    let target = d.qualified(scope, name);
    let regclass = d.string_literal(&target);
    let first = |set: ResultSet| set.rows.first().map(|row| cell(row, 0)).unwrap_or_default();
    match kind {
        ObjectKind::View => {
            let def = first(
                query(
                    r,
                    &format!("SELECT pg_get_viewdef({regclass}::regclass, true)"),
                )
                .await?,
            );
            Ok(format!("CREATE VIEW {target} AS\n{def}"))
        }
        ObjectKind::MaterializedView => {
            let def = first(
                query(
                    r,
                    &format!("SELECT pg_get_viewdef({regclass}::regclass, true)"),
                )
                .await?,
            );
            Ok(format!("CREATE MATERIALIZED VIEW {target} AS\n{def}"))
        }
        ObjectKind::Function => {
            let schema = d.string_literal(scope.schema.as_deref().unwrap_or("public"));
            let sql = format!(
                "SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = {schema} AND p.proname = {} AND p.prokind IN ('f', 'p') LIMIT 1",
                d.string_literal(name)
            );
            Ok(first(query(r, &sql).await?))
        }
        ObjectKind::Sequence => {
            let set = query(
                r,
                &format!(
                    "SELECT seqstart, seqincrement, seqmin, seqmax, seqcache, seqcycle FROM pg_sequence WHERE seqrelid = {regclass}::regclass"
                ),
            )
            .await?;
            let row = set.rows.first().cloned().unwrap_or_default();
            Ok(format!(
                "CREATE SEQUENCE {target}\n  START WITH {}\n  INCREMENT BY {}\n  MINVALUE {}\n  MAXVALUE {}\n  CACHE {}{};",
                cell(&row, 0),
                cell(&row, 1),
                cell(&row, 2),
                cell(&row, 3),
                cell(&row, 4),
                if cell(&row, 5) == "t" {
                    "\n  CYCLE"
                } else {
                    ""
                }
            ))
        }
        ObjectKind::Table => {
            let cols = query(
                r,
                &format!(
                    "SELECT a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(ad.adbin, ad.adrelid) \
                     FROM pg_attribute a LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
                     WHERE a.attrelid = {regclass}::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
                ),
            )
            .await?;
            let cons = query(
                r,
                &format!(
                    "SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = {regclass}::regclass \
                     ORDER BY contype DESC, conname"
                ),
            )
            .await?;
            let idx = query(
                r,
                &format!(
                    "SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i \
                     WHERE i.indrelid = {regclass}::regclass AND NOT i.indisprimary \
                     AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = i.indexrelid)"
                ),
            )
            .await?;
            let comment = first(
                query(
                    r,
                    &format!("SELECT obj_description({regclass}::regclass, 'pg_class')"),
                )
                .await?,
            );
            let mut lines: Vec<String> = cols
                .rows
                .iter()
                .map(|row| {
                    let mut l = format!("  {} {}", d.quote_ident(&cell(row, 0)), cell(row, 1));
                    if cell(row, 2) == "t" {
                        l.push_str(" NOT NULL");
                    }
                    let default = cell(row, 3);
                    if !default.is_empty() {
                        l.push_str(&format!(" DEFAULT {default}"));
                    }
                    l
                })
                .collect();
            lines.extend(cons.rows.iter().map(|row| {
                format!(
                    "  CONSTRAINT {} {}",
                    d.quote_ident(&cell(row, 0)),
                    cell(row, 1)
                )
            }));
            let mut ddl = format!("CREATE TABLE {target} (\n{}\n);", lines.join(",\n"));
            for row in &idx.rows {
                ddl.push_str(&format!("\n{};", cell(row, 0)));
            }
            if !comment.is_empty() {
                ddl.push_str(&format!(
                    "\nCOMMENT ON TABLE {target} IS {};",
                    d.string_literal(&comment)
                ));
            }
            Ok(ddl)
        }
        other => Err(unsupported(other)),
    }
}

pub async fn table_columns<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<Vec<ColumnInfo>> {
    let d = r.dialect();
    match d {
        Dialect::MySql => {
            let sql = format!(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT \
                 FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY ORDINAL_POSITION",
                d.string_literal(&scope.database),
                d.string_literal(table)
            );
            Ok(query(r, &sql)
                .await?
                .rows
                .iter()
                .map(|row| ColumnInfo {
                    name: cell(row, 0),
                    type_name: cell(row, 1),
                    nullable: cell(row, 2) == "YES",
                    primary_key: cell(row, 3) == "PRI",
                    default: row.get(4).cloned().flatten(),
                    comment: Some(cell(row, 5)).filter(|c| !c.is_empty()),
                })
                .collect())
        }
        Dialect::Postgres => {
            let regclass = d.string_literal(&d.qualified(scope, table));
            let sql = format!(
                "SELECT a.attname, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
                 COALESCE((SELECT true FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)), false), \
                 pg_get_expr(ad.adbin, ad.adrelid), col_description(a.attrelid, a.attnum) \
                 FROM pg_attribute a LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
                 WHERE a.attrelid = {regclass}::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
            );
            Ok(query(r, &sql)
                .await?
                .rows
                .iter()
                .map(|row| ColumnInfo {
                    name: cell(row, 0),
                    type_name: cell(row, 1),
                    nullable: cell(row, 2) == "t",
                    primary_key: cell(row, 3) == "t",
                    default: row.get(4).cloned().flatten(),
                    comment: row.get(5).cloned().flatten(),
                })
                .collect())
        }
        Dialect::Sqlite => {
            let sql = format!(
                "PRAGMA {}.table_xinfo({})",
                d.quote_ident(&scope.database),
                d.quote_ident(table)
            );
            Ok(query(r, &sql)
                .await?
                .rows
                .iter()
                .map(|row| ColumnInfo {
                    name: cell(row, 1),
                    type_name: cell(row, 2),
                    nullable: cell(row, 3) == "0",
                    primary_key: cell(row, 5) != "0",
                    default: row.get(4).cloned().flatten(),
                    comment: None,
                })
                .collect())
        }
    }
}

/// Indexes, foreign keys and triggers of a table, shown under it in the navigator (F-NAV-002).
pub async fn table_children<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<TableChildren> {
    let d = r.dialect();
    let t = d.string_literal(table);
    let [indexes, foreign_keys, triggers] = match d {
        Dialect::MySql => {
            let db = d.string_literal(&scope.database);
            [
                format!(
                    "SELECT DISTINCT INDEX_NAME FROM information_schema.STATISTICS \
                     WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {t} ORDER BY INDEX_NAME"
                ),
                format!(
                    "SELECT CONSTRAINT_NAME FROM information_schema.TABLE_CONSTRAINTS \
                     WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {t} AND CONSTRAINT_TYPE = 'FOREIGN KEY' \
                     ORDER BY CONSTRAINT_NAME"
                ),
                format!(
                    "SELECT TRIGGER_NAME FROM information_schema.TRIGGERS \
                     WHERE EVENT_OBJECT_SCHEMA = {db} AND EVENT_OBJECT_TABLE = {t} ORDER BY TRIGGER_NAME"
                ),
            ]
        }
        Dialect::Postgres => {
            let rel = format!("{}::regclass", d.string_literal(&d.qualified(scope, table)));
            [
                format!(
                    "SELECT c.relname FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid WHERE i.indrelid = {rel} ORDER BY 1"
                ),
                format!(
                    "SELECT conname FROM pg_constraint WHERE conrelid = {rel} AND contype = 'f' ORDER BY 1"
                ),
                format!(
                    "SELECT tgname FROM pg_trigger WHERE tgrelid = {rel} AND NOT tgisinternal ORDER BY 1"
                ),
            ]
        }
        Dialect::Sqlite => {
            let schema = format!("{}.sqlite_schema", d.quote_ident(&scope.database));
            [
                format!(
                    "SELECT name FROM {schema} WHERE type = 'index' AND tbl_name = {t} ORDER BY name"
                ),
                // SQLite foreign keys have no names; label them by the referenced table
                format!(
                    "SELECT DISTINCT '→ ' || \"table\" FROM pragma_foreign_key_list({t}, {}) ORDER BY 1",
                    d.string_literal(&scope.database)
                ),
                format!(
                    "SELECT name FROM {schema} WHERE type = 'trigger' AND tbl_name = {t} ORDER BY name"
                ),
            ]
        }
    };
    let mut names = async |sql: String| -> DbResult<Vec<String>> {
        Ok(query(r, &sql)
            .await?
            .rows
            .iter()
            .map(|row| cell(row, 0))
            .collect())
    };
    Ok(TableChildren {
        indexes: names(indexes).await?,
        foreign_keys: names(foreign_keys).await?,
        triggers: names(triggers).await?,
    })
}
