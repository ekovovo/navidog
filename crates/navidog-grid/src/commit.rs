//! Committing grid changes, shared by the table viewer and editable query results.

use navidog_db::dialect::Dialect;
use navidog_db::sessions::SharedSession;
use navidog_db::{DbResult, ResultSet};
use navidog_util::starts_with_ignore_ascii_case;
use rust_i18n::t;

/// Change statements and how to commit them.
#[derive(Clone)]
pub struct Commit {
    pub statements: Vec<String>,
    pub begin: &'static str,
    pub dialect: Dialect,
}

impl Commit {
    pub fn new(dialect: Dialect, statements: Vec<String>) -> Self {
        Self {
            statements,
            begin: dialect.begin_transaction(),
            dialect,
        }
    }
}

/// Applies the changes; either all take effect or none. Without an open transaction they run in
/// one that is rolled back on failure; inside the user's transaction a savepoint does that, so a
/// failed batch does not leave half its statements in the transaction.
/// When `end` is true, the already open transaction is ended (committed).
pub async fn apply(
    rel: &mut dyn navidog_db::Relational,
    c: Commit,
    in_tx: bool,
    end: bool,
) -> DbResult<()> {
    const SAVEPOINT: &str = "SAVEPOINT navidog_apply";
    let (start, undo) = if in_tx {
        (SAVEPOINT, "ROLLBACK TO SAVEPOINT navidog_apply")
    } else {
        (c.begin, "ROLLBACK")
    };
    rel.execute(start).await?;
    for s in &c.statements {
        let result = rel.execute(s).await;
        if let Err(e) = result.and_then(|sets| matched_a_row(s, &sets, c.dialect)) {
            let _ = rel.execute(undo).await;
            return Err(e);
        }
    }
    if in_tx {
        rel.execute("RELEASE SAVEPOINT navidog_apply").await?;
    }
    if !in_tx || end {
        rel.execute("COMMIT").await?;
    }
    Ok(())
}

/// An UPDATE or DELETE that matched no row means the grid's old values no longer find it (changed
/// elsewhere, or a value only the server knew). MySQL counts changed rather than matched rows for
/// UPDATE, so an UPDATE to the same value is 0 there too: only its DELETE is checked.
pub(crate) fn matched_a_row(sql: &str, sets: &[ResultSet], dialect: Dialect) -> DbResult<()> {
    let delete = starts_with_ignore_ascii_case(sql, "DELETE");
    let update = starts_with_ignore_ascii_case(sql, "UPDATE") && dialect != Dialect::MySql;
    if (delete || update) && sets.last().and_then(|s| s.affected_rows) == Some(0) {
        return Err(navidog_db::DbError::msg(
            t!("viewer.row_not_found", sql = sql).to_string(),
        ));
    }
    Ok(())
}

/// Formats driver details and localized recovery guidance at the grid UI boundary.
/// The database layer retains the original code and message for diagnostics.
pub fn error_message(error: &navidog_db::DbError) -> String {
    localized_error_message(error, &rust_i18n::locale())
}

fn localized_error_message(error: &navidog_db::DbError, locale: &str) -> String {
    navidog_ui::db_error_text_with(
        error.code.as_deref(),
        &error.message,
        error.is_session_lost(),
        error.is_outcome_unknown(),
        |detail| t!("viewer.session_lost", locale = locale, detail = detail).to_string(),
        |detail| t!("viewer.outcome_unknown", locale = locale, detail = detail).to_string(),
    )
}

/// Closes a session the caller held, in the background.
pub fn close_session(session: SharedSession) {
    navidog_db::runtime::spawn(async move {
        if let Some(s) = session.lock().await.take() {
            let _ = s.close().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_recovery_copy_keeps_the_driver_diagnostic_in_both_languages() {
        let mut error = navidog_db::DbError::coded("08006", "original grid diagnostic");
        let diagnostic = error.to_string();
        for locale in ["en", "zh-CN"] {
            assert_eq!(localized_error_message(&error, locale), diagnostic);
            error.disconnected = true;
            for unknown in [false, true] {
                error.outcome_unknown = unknown;
                let message = localized_error_message(&error, locale);
                assert!(message.starts_with(&diagnostic));
                assert!(!message.contains('\n'));
                if locale == "en" {
                    assert!(
                        message
                            .to_ascii_lowercase()
                            .contains("reopen the connection")
                    );
                } else {
                    assert!(message.contains("重新打开连接"));
                }
                assert_eq!(
                    message.contains(if locale == "en" {
                        "outcome may be unknown"
                    } else {
                        "执行结果可能未知"
                    }),
                    unknown,
                );
            }
            error.disconnected = false;
            error.outcome_unknown = false;
        }
        assert_eq!(error.to_string(), diagnostic);
    }

    #[test]
    fn a_write_that_matched_no_row_fails() {
        let affected = |n| {
            vec![ResultSet {
                affected_rows: Some(n),
                ..ResultSet::default()
            }]
        };
        let update = "UPDATE \"t\" SET \"name\" = 'b' WHERE \"id\" IS NULL";
        let delete = "DELETE FROM `t` WHERE `id` = 1 LIMIT 1";
        assert!(matched_a_row(update, &affected(0), Dialect::Sqlite).is_err());
        assert!(matched_a_row(update, &affected(1), Dialect::Postgres).is_ok());
        assert!(matched_a_row(delete, &affected(0), Dialect::MySql).is_err());
        // MySQL counts changed rows: an UPDATE to the same value is 0 and still fine
        assert!(matched_a_row(update, &affected(0), Dialect::MySql).is_ok());
        assert!(matched_a_row("INSERT INTO t VALUES (1)", &affected(0), Dialect::Sqlite).is_ok());
    }
}
