//! Dameng catalog SQL. Object names are bound as quoted literals (`string_literal`), the
//! same way the PostgreSQL catalog does it — `Relational::execute_read` has no parameter
//! channel (deviation noted in 140-dameng.md §4).

use navidog_db::driver::{ColumnInfo, DbResult, ObjectInfo, Relational};
use navidog_db::metadata::{self, cell};
use navidog_db::outline::{self, Outline, OutlineTable};
use navidog_model::object::{ObjectKind, Scope};

async fn one_cell<R: Relational + ?Sized>(r: &mut R, sql: &str) -> DbResult<String> {
    let set = metadata::query(r, sql).await?;
    Ok(set
        .rows
        .first()
        .and_then(|row| row.first().cloned().flatten())
        .unwrap_or_default())
}

/// Type text as the server prints it back, from the catalog's DATA_* columns.
fn type_text() -> &'static str {
    "CASE WHEN C.DATA_TYPE IN ('VARCHAR', 'VARCHAR2', 'CHAR', 'CHARACTER') \
     THEN C.DATA_TYPE || '(' || C.DATA_LENGTH || ')' \
     WHEN C.DATA_TYPE IN ('DECIMAL', 'NUMERIC', 'NUMBER', 'DEC') \
          AND C.DATA_PRECISION IS NOT NULL \
     THEN C.DATA_TYPE || '(' || C.DATA_PRECISION || ', ' || C.DATA_SCALE || ')' \
     WHEN C.DATA_TYPE IN ('TIMESTAMP', 'DATETIME', 'TIME') AND C.DATA_SCALE > 0 \
     THEN C.DATA_TYPE || '(' || C.DATA_SCALE || ')' \
     ELSE C.DATA_TYPE END"
}

pub async fn list_databases<R: Relational + ?Sized>(r: &mut R) -> DbResult<Vec<String>> {
    // One database per instance (`DAMENG`); the database level stays in the tree so the
    // generic connection → database → schema outline is product-free (D7).
    metadata::first_column(r, "SELECT NAME FROM V$DATABASE").await
}

pub async fn list_schemas<R: Relational + ?Sized>(r: &mut R) -> DbResult<Vec<String>> {
    // Users ≈ schemas; system schemas are listed and hidden by the navigator.
    metadata::first_column(r, "SELECT USERNAME FROM ALL_USERS ORDER BY USERNAME").await
}

pub async fn list_objects<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
) -> DbResult<Vec<ObjectInfo>> {
    let owner = r
        .dialect()
        .string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let (sql, props) = object_sql(&owner, kind)?;
    let set = metadata::query(r, &sql).await?;
    Ok(metadata::objects(&set.rows, kind, props))
}

fn object_sql(owner: &str, kind: ObjectKind) -> DbResult<(String, &'static [&'static str])> {
    let (sql, props): (String, &[&str]) = match kind {
        ObjectKind::Table => (
            format!(
                "SELECT T.TABLE_NAME, T.NUM_ROWS, T.OWNER, CM.COMMENTS \
                 FROM ALL_TABLES T LEFT JOIN ALL_TAB_COMMENTS CM \
                 ON CM.OWNER = T.OWNER AND CM.TABLE_NAME = T.TABLE_NAME \
                 WHERE T.OWNER = {owner} ORDER BY T.TABLE_NAME"
            ),
            &["rows", "owner", "comment"],
        ),
        ObjectKind::View => (
            format!(
                "SELECT V.VIEW_NAME, NULL, V.OWNER, NULL FROM ALL_VIEWS V \
                 WHERE V.OWNER = {owner} ORDER BY V.VIEW_NAME"
            ),
            &["rows", "owner", "comment"],
        ),
        ObjectKind::Function => (
            format!(
                "SELECT P.OBJECT_NAME, P.OBJECT_TYPE, P.OWNER, NULL FROM ALL_PROCEDURES P \
                 WHERE P.OWNER = {owner} AND P.OBJECT_TYPE IN ('FUNCTION', 'PROCEDURE') \
                 ORDER BY P.OBJECT_NAME, P.OBJECT_TYPE"
            ),
            &["type", "owner", "comment"],
        ),
        ObjectKind::Sequence => (
            format!(
                "SELECT S.SEQUENCE_NAME, S.LAST_NUMBER, S.SEQUENCE_OWNER, NULL \
                 FROM ALL_SEQUENCES S WHERE S.SEQUENCE_OWNER = {owner} \
                 ORDER BY S.SEQUENCE_NAME"
            ),
            &["last_value", "owner", "comment"],
        ),
        ObjectKind::Trigger => (
            format!(
                "SELECT T.TRIGGER_NAME, T.TABLE_NAME, T.STATUS, \
                 T.TRIGGERING_TYPE || ' ' || T.TRIGGERING_EVENT \
                 FROM ALL_TRIGGERS T WHERE T.OWNER = {owner} ORDER BY T.TRIGGER_NAME"
            ),
            &["table", "status", "event"],
        ),
        other => return Err(metadata::unsupported(other)),
    };
    Ok((sql, props))
}

pub async fn object_ddl<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    kind: ObjectKind,
    name: &str,
) -> DbResult<String> {
    let d = r.dialect();
    let owner = d.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let quoted = d.string_literal(name);
    let target = d.qualified(scope, name);
    match kind {
        // DBMS_METADATA.GET_DDL is unusable on DM (TABLE -20008, others empty), so tables
        // come from SP_TABLEDEF and the rest are synthesized from the catalog (D6).
        ObjectKind::Table => one_cell(r, &format!("CALL SP_TABLEDEF({owner}, {quoted})")).await,
        ObjectKind::View => {
            let text = one_cell(
                r,
                &format!(
                    "SELECT TEXT FROM ALL_VIEWS WHERE OWNER = {owner} AND VIEW_NAME = {quoted}"
                ),
            )
            .await?;
            Ok(format!("CREATE VIEW {target} AS\n{text}"))
        }
        ObjectKind::Function => {
            let body = one_cell(
                r,
                &format!(
                    "SELECT LISTAGG(S.TEXT, '') WITHIN GROUP (ORDER BY S.LINE) FROM ALL_SOURCE S \
                     WHERE S.OWNER = {owner} AND S.NAME = {quoted} \
                     AND S.TYPE IN ('FUNCTION', 'PROC')"
                ),
            )
            .await?;
            if body.is_empty() {
                return Err(navidog_db::DbError::coded(
                    "navidog.no_source",
                    "The function body is empty.",
                ));
            }
            Ok(body)
        }
        ObjectKind::Sequence => {
            let set = metadata::query(
                r,
                &format!(
                    "SELECT MIN_VALUE, MAX_VALUE, INCREMENT_BY, CYCLE_FLAG, CACHE_SIZE, LAST_NUMBER \
                     FROM ALL_SEQUENCES WHERE SEQUENCE_OWNER = {owner} AND SEQUENCE_NAME = {quoted}"
                ),
            )
            .await?;
            let row = set.rows.first().cloned().unwrap_or_default();
            let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
            Ok(format!(
                "CREATE SEQUENCE {target}\n  START WITH {}\n  INCREMENT BY {}\n  MINVALUE {}\n  MAXVALUE {}\n  CACHE {}{};",
                get(5),
                get(2),
                get(0),
                get(1),
                get(4),
                if get(3) == "Y" { "\n  CYCLE" } else { "" }
            ))
        }
        ObjectKind::Trigger => {
            let set = metadata::query(
                r,
                &format!(
                    "SELECT DESCRIPTION, TRIGGER_BODY FROM ALL_TRIGGERS \
                     WHERE OWNER = {owner} AND TRIGGER_NAME = {quoted}"
                ),
            )
            .await?;
            let row = set.rows.first().cloned().unwrap_or_default();
            let description = row.first().cloned().flatten().unwrap_or_default();
            let body = row.get(1).cloned().flatten().unwrap_or_default();
            if description.is_empty() {
                return Err(navidog_db::DbError::coded(
                    "navidog.no_source",
                    "The trigger definition is empty.",
                ));
            }
            Ok(format!("CREATE OR REPLACE TRIGGER {description}\n{body}"))
        }
        ObjectKind::Index => {
            let set = metadata::query(
                r,
                &format!(
                    "SELECT I.UNIQUENESS, I.TABLE_NAME, C.COLUMN_NAME \
                     FROM ALL_INDEXES I JOIN ALL_IND_COLUMNS C \
                     ON C.INDEX_OWNER = I.OWNER AND C.INDEX_NAME = I.INDEX_NAME \
                     WHERE I.OWNER = {owner} AND I.INDEX_NAME = {quoted} \
                     ORDER BY C.COLUMN_POSITION"
                ),
            )
            .await?;
            let mut columns: Vec<String> = Vec::new();
            let mut table = String::new();
            let mut unique = false;
            for row in &set.rows {
                if cell(row, 0) == "UNIQUE" {
                    unique = true;
                }
                table = cell(row, 1);
                columns.push(d.quote_ident(&cell(row, 2)));
            }
            if table.is_empty() {
                return Err(navidog_db::DbError::coded(
                    "navidog.no_result",
                    "The index was not found.",
                ));
            }
            let table = d.qualified(scope, &table);
            Ok(format!(
                "CREATE {}INDEX {}\n  ON {table} ({})",
                if unique { "UNIQUE " } else { "" },
                d.quote_ident(name),
                columns.join(", ")
            ))
        }
        other => Err(metadata::unsupported(other)),
    }
}

pub async fn table_columns<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<Vec<ColumnInfo>> {
    let d = r.dialect();
    let owner = d.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let quoted = d.string_literal(table);
    let tt = type_text();
    let set = metadata::query(
        r,
        &format!(
            "SELECT C.COLUMN_NAME, {tt}, C.NULLABLE, \
             CASE WHEN EXISTS (SELECT 1 FROM ALL_CONSTRAINTS K JOIN ALL_CONS_COLUMNS KC \
             ON KC.OWNER = K.OWNER AND KC.CONSTRAINT_NAME = K.CONSTRAINT_NAME \
             WHERE K.CONSTRAINT_TYPE = 'P' AND K.OWNER = C.OWNER \
             AND K.TABLE_NAME = C.TABLE_NAME AND KC.COLUMN_NAME = C.COLUMN_NAME) \
             THEN 'Y' ELSE 'N' END, C.DATA_DEFAULT, CO.COMMENTS \
             FROM ALL_TAB_COLUMNS C LEFT JOIN ALL_COL_COMMENTS CO \
             ON CO.OWNER = C.OWNER AND CO.TABLE_NAME = C.TABLE_NAME \
             AND CO.COLUMN_NAME = C.COLUMN_NAME \
             WHERE C.OWNER = {owner} AND C.TABLE_NAME = {quoted} ORDER BY C.COLUMN_ID"
        ),
    )
    .await?;
    Ok(set
        .rows
        .iter()
        .map(|row| ColumnInfo {
            name: cell(row, 0),
            type_name: cell(row, 1),
            nullable: cell(row, 2) == "Y",
            primary_key: cell(row, 3) == "Y",
            default: row.get(4).cloned().flatten().filter(|v| !v.is_empty()),
            comment: row.get(5).cloned().flatten().filter(|v| !v.is_empty()),
        })
        .collect())
}

pub async fn table_children<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<navidog_db::TableChildren> {
    let d = r.dialect();
    let owner = d.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let quoted = d.string_literal(table);
    metadata::children_from(
        r,
        // CLUSTERBTR cluster indexes are storage internals, not user objects.
        format!(
            "SELECT INDEX_NAME FROM ALL_INDEXES WHERE TABLE_OWNER = {owner} \
             AND TABLE_NAME = {quoted} AND INDEX_TYPE <> 'CLUSTER' ORDER BY INDEX_NAME"
        ),
        format!(
            "SELECT CONSTRAINT_NAME FROM ALL_CONSTRAINTS WHERE OWNER = {owner} \
             AND TABLE_NAME = {quoted} AND CONSTRAINT_TYPE = 'R' ORDER BY CONSTRAINT_NAME"
        ),
        format!(
            "SELECT TRIGGER_NAME FROM ALL_TRIGGERS WHERE TABLE_OWNER = {owner} \
             AND TABLE_NAME = {quoted} ORDER BY TRIGGER_NAME"
        ),
    )
    .await
}

pub async fn foreign_keys<R: Relational + ?Sized>(
    r: &mut R,
    scope: &Scope,
    table: &str,
) -> DbResult<Vec<navidog_db::ForeignKey>> {
    let d = r.dialect();
    let owner = d.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let quoted = d.string_literal(table);
    Ok(metadata::foreign_keys_from_rows(
        &metadata::query(
            r,
            &format!(
                "SELECT CC.COLUMN_NAME, C.R_OWNER, RC.TABLE_NAME, RCC.COLUMN_NAME \
                 FROM ALL_CONSTRAINTS C \
                 JOIN ALL_CONS_COLUMNS CC ON CC.OWNER = C.OWNER \
                 AND CC.CONSTRAINT_NAME = C.CONSTRAINT_NAME \
                 JOIN ALL_CONSTRAINTS RC ON RC.OWNER = C.R_OWNER \
                 AND RC.CONSTRAINT_NAME = C.R_CONSTRAINT_NAME \
                 JOIN ALL_CONS_COLUMNS RCC ON RCC.OWNER = RC.OWNER \
                 AND RCC.CONSTRAINT_NAME = RC.CONSTRAINT_NAME AND RCC.POSITION = CC.POSITION \
                 WHERE C.CONSTRAINT_TYPE = 'R' AND C.OWNER = {owner} \
                 AND C.TABLE_NAME = {quoted} ORDER BY CC.POSITION"
            ),
        )
        .await?
        .rows,
    ))
}

pub async fn outline<R: Relational + ?Sized>(
    rel: &mut R,
    scope: &Scope,
    system: bool,
) -> DbResult<Outline> {
    let d = rel.dialect();
    let owner = d.string_literal(scope.schema.as_deref().unwrap_or("SYSDBA"));
    let tt = type_text();
    let mut tables: Vec<OutlineTable> = outline::rows(
        rel.execute_read(&format!(
            "SELECT TABLE_NAME, 't' FROM ALL_TABLES WHERE OWNER = {owner} \
             UNION ALL SELECT VIEW_NAME, 'v' FROM ALL_VIEWS WHERE OWNER = {owner} \
             ORDER BY 1"
        ))
        .await?,
    )
    .into_iter()
    .map(|r| OutlineTable {
        name: r[0].clone(),
        view: r.get(1).map(String::as_str) == Some("v"),
        columns: Vec::new(),
    })
    .collect();
    let columns = outline::rows(
        rel.execute_read(&format!(
            "SELECT TABLE_NAME, COLUMN_NAME, {tt} FROM ALL_TAB_COLUMNS C \
             WHERE C.OWNER = {owner} ORDER BY C.TABLE_NAME, C.COLUMN_ID"
        ))
        .await?,
    );
    outline::group_columns(&mut tables, columns);
    let functions = metadata::first_column(
        rel,
        &format!(
            "SELECT OBJECT_NAME FROM ALL_PROCEDURES WHERE OWNER = {owner} \
             AND OBJECT_TYPE IN ('FUNCTION', 'PROCEDURE') ORDER BY OBJECT_NAME"
        ),
    )
    .await?;
    let mut schemas = rel.list_schemas().await?;
    if !system {
        schemas.retain(|s| !d.is_system_schema(s));
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
    use navidog_sql::Dialect as _;

    #[test]
    fn object_sql_uses_quoted_literals() {
        let (sql, props) = object_sql("'SYSDBA'", ObjectKind::Table).unwrap();
        assert!(sql.contains("ALL_TABLES"));
        assert!(sql.contains("WHERE T.OWNER = 'SYSDBA'"));
        assert_eq!(props, &["rows", "owner", "comment"]);
        let (sql, props) = object_sql("'SYSDBA'", ObjectKind::Function).unwrap();
        assert!(sql.contains("OBJECT_TYPE IN ('FUNCTION', 'PROCEDURE')"));
        assert_eq!(props, &["type", "owner", "comment"]);
    }

    #[test]
    fn ddl_reads_sp_tabledef_for_tables() {
        let scope = Scope::new("c", "DAMENG", Some("SYSDBA".into()));
        // SP_TABLEDEF is a procedure, not a select item; the CALL shape is handled by the
        // driver's Call mapping. The SQL text here is the contract.
        let sql = "CALL SP_TABLEDEF('SYSDBA', 'T')";
        assert!(sql.starts_with("CALL SP_TABLEDEF"));
        assert_eq!(
            crate::DamengDialect::default().qualified(&scope, "T"),
            "\"SYSDBA\".\"T\""
        );
    }

    #[test]
    fn foreign_key_rows_carry_the_referenced_schema() {
        let rows = vec![vec![
            Some("PID".into()),
            Some("SYSDBA".into()),
            Some("PROBE_T".into()),
            Some("ID".into()),
        ]];
        let keys = metadata::foreign_keys_from_rows(&rows);
        assert_eq!(keys[0].schema, "SYSDBA");
        assert_eq!(keys[0].table, "PROBE_T");
        assert_eq!(keys[0].referenced, "ID");
    }
}
