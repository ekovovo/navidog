//! MySQL and MariaDB catalog SQL.

use navidog_db::driver::{ColumnInfo, DbResult, ObjectInfo, Relational};
use navidog_db::metadata::{self, cell};
use navidog_db::outline::{self, Outline, OutlineTable};
use navidog_model::object::{ObjectKind, Scope};

pub async fn list_databases<R: Relational + ?Sized>(r: &mut R) -> DbResult<Vec<String>> {
    metadata::first_column(r, "SHOW DATABASES").await
}

pub async fn list_schemas<R: Relational + ?Sized>(_r: &mut R) -> DbResult<Vec<String>> {
    Ok(Vec::new())
}

pub async fn list_objects<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<Vec<ObjectInfo>> {
    let (sql, props) = object_sql(r.dialect().as_ref(), scope, kind)?;
    let set = metadata::query(r, &sql).await?;
    Ok(metadata::objects(&set.rows, kind, props))
}

fn object_sql(
    d: &dyn navidog_sql::Dialect,
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
        ObjectKind::Tablespace => (
            "SELECT TABLESPACE_NAME FROM information_schema.FILES \
             WHERE TABLESPACE_NAME IS NOT NULL AND TABLESPACE_NAME <> '' \
             GROUP BY TABLESPACE_NAME ORDER BY TABLESPACE_NAME"
                .into(),
            &[],
        ),
        ObjectKind::Sequence => (
            format!(
                "SELECT SEQUENCE_NAME FROM information_schema.SEQUENCES \
                 WHERE SEQUENCE_SCHEMA = {db} ORDER BY SEQUENCE_NAME"
            ),
            &[],
        ),
        other => return Err(metadata::unsupported(other)),
    })
}

pub async fn object_ddl<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
    name: &str,
) -> DbResult<String> {
    let d = r.dialect();
    let target = d.qualified(scope, name);
    let (sql, col) = match kind {
        ObjectKind::Table => (format!("SHOW CREATE TABLE {target}"), 1),
        ObjectKind::View => (format!("SHOW CREATE VIEW {target}"), 1),
        ObjectKind::Event => (format!("SHOW CREATE EVENT {target}"), 3),
        ObjectKind::Tablespace => (
            format!(
                "SELECT CONCAT('CREATE TABLESPACE ', TABLESPACE_NAME, ' ADD DATAFILE ', QUOTE(FILE_NAME)) \
                 FROM information_schema.FILES WHERE TABLESPACE_NAME = {} LIMIT 1",
                d.string_literal(name)
            ),
            0,
        ),
        // MariaDB returns `Table` and `Create Table`, same as SHOW CREATE TABLE.
        ObjectKind::Sequence => (format!("SHOW CREATE SEQUENCE {target}"), 1),
        ObjectKind::Function => {
            let ty = metadata::query(
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
                .map(|row| cell(row, 0))
                .unwrap_or_else(|| "FUNCTION".into());
            (format!("SHOW CREATE {ty} {target}"), 2)
        }
        other => return Err(metadata::unsupported(other)),
    };
    let set = metadata::query(r, &sql).await?;
    Ok(set
        .rows
        .first()
        .map(|row| cell(row, col))
        .unwrap_or_default()
        + ";")
}

pub async fn table_columns<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<Vec<ColumnInfo>> {
    let d = r.dialect();
    let sql = format!(
        "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT \
         FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY ORDINAL_POSITION",
        d.string_literal(&scope.database),
        d.string_literal(table)
    );
    Ok(metadata::query(r, &sql)
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

pub async fn table_children<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<navidog_db::TableChildren> {
    let d = r.dialect();
    let db = d.string_literal(&scope.database);
    let t = d.string_literal(table);
    metadata::children_from(
        r,
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
    )
    .await
}

pub fn foreign_key_sql(dialect: &dyn navidog_sql::Dialect, scope: &Scope, table: &str) -> String {
    let db = dialect.string_literal(&scope.database);
    let t = dialect.string_literal(table);
    format!(
        "SELECT COLUMN_NAME, REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
         FROM information_schema.KEY_COLUMN_USAGE \
         WHERE TABLE_SCHEMA = {db} AND TABLE_NAME = {t} AND REFERENCED_TABLE_NAME IS NOT NULL \
         ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION"
    )
}

pub async fn foreign_keys<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<Vec<navidog_db::ForeignKey>> {
    let sql = foreign_key_sql(r.dialect().as_ref(), scope, table);
    Ok(metadata::foreign_keys_from_rows(
        &metadata::query(r, &sql).await?.rows,
    ))
}

pub async fn outline<R: Relational + ?Sized>(
    rel: &mut R,
    scope: &Scope,
    system: bool,
) -> DbResult<Outline> {
    let d = rel.dialect();
    let db = d.string_literal(&scope.database);
    let mut tables: Vec<OutlineTable> = outline::rows(
        rel.execute_read(&format!(
            "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = {db} ORDER BY TABLE_NAME"
        ))
        .await?,
    )
    .into_iter()
    .map(|r| OutlineTable {
        name: r[0].clone(),
        view: r.get(1).is_some_and(|t| t.contains("VIEW")),
        columns: Vec::new(),
    })
    .collect();
    let columns = outline::rows(
        rel.execute_read(&format!(
            "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = {db} ORDER BY TABLE_NAME, ORDINAL_POSITION"
        ))
        .await?,
    );
    outline::group_columns(&mut tables, columns);
    let functions = outline::rows(
        rel.execute_read(&format!(
            "SELECT ROUTINE_NAME FROM information_schema.ROUTINES \
             WHERE ROUTINE_SCHEMA = {db} ORDER BY ROUTINE_NAME"
        ))
        .await?,
    )
    .into_iter()
    .map(|r| r[0].clone())
    .collect();
    let mut schemas = rel.list_databases().await?;
    if !system {
        schemas.retain(|s| !d.is_system_database(s));
    }
    Ok(Outline {
        tables,
        functions,
        schemas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_key_sql_reads_the_catalog() {
        let scope = Scope::new("c", "sakila", Some("public".into()));
        let sql = foreign_key_sql(&crate::MysqlDialect, &scope, "film_actor");
        assert!(sql.contains("information_schema.KEY_COLUMN_USAGE"));
        assert!(sql.contains("REFERENCED_COLUMN_NAME"));
        assert!(sql.contains("'film_actor'"));
        assert!(sql.contains("'sakila'"));
    }
}
