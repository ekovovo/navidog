//! SQL statement splitting: cuts at statement-ending semicolons, skipping strings, quoted
//! identifiers, comments, PostgreSQL `$tag$` strings, and `BEGIN … END` blocks inside
//! `CREATE TRIGGER / FUNCTION / PROCEDURE / EVENT`. MySQL scripts may switch the terminator with
//! a `DELIMITER` line, as the mysql client does.

use std::ops::Range;

use navidog_sql::span::{self, Lex, SpanKind};

/// One statement of a script and where it sits in the original text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// Trimmed, without its terminator; leading comments stay (result tabs read `-- NAME:`)
    pub text: String,
    /// Byte range of `text` in the script
    pub range: Range<usize>,
}

/// `USE db` or `SET search_path`, the statements a reconnect has to replay so unqualified
/// names keep resolving where the user left them. A script of more than one statement is
/// not replayed: running it again would repeat the rest.
pub fn session_setup(sql: &str) -> Option<String> {
    let body = sql.trim().trim_end_matches(';').trim();
    if body.is_empty() || body.contains(';') {
        return None;
    }
    if use_statement(body) || search_path_statement(body) || set_schema_statement(body) {
        Some(body.to_string())
    } else {
        None
    }
}

fn keyword<'a>(sql: &'a str, kw: &str) -> Option<&'a str> {
    let (head, rest) = sql.split_at_checked(kw.len())?;
    if head.eq_ignore_ascii_case(kw) && rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn use_statement(sql: &str) -> bool {
    keyword(sql, "use").is_some_and(|rest| !rest.is_empty())
}

fn search_path_statement(sql: &str) -> bool {
    let Some(mut rest) = keyword(sql, "set") else {
        return false;
    };
    if let Some(after) = keyword(rest, "session") {
        rest = after;
    }
    let Some(head) = rest.get(.."search_path".len()) else {
        return false;
    };
    if !head.eq_ignore_ascii_case("search_path") {
        return false;
    }
    let after = &rest["search_path".len()..];
    after.is_empty() || after.starts_with(|c: char| c.is_whitespace()) || after.starts_with('=')
}

/// Dameng `SET SCHEMA "name"` switches the current schema and is replayed on reconnect.
fn set_schema_statement(sql: &str) -> bool {
    let Some(rest) = keyword(sql, "set") else {
        return false;
    };
    keyword(rest, "schema").is_some_and(|value| !value.is_empty())
}

/// The statements' text only; see [`split_script`].
pub fn split_statements(sql: &str, lex: Lex) -> Vec<String> {
    split_script(sql, lex).into_iter().map(|s| s.text).collect()
}

/// Splits a script into statements. Empty and comment-only statements are dropped, and so are
/// MySQL `DELIMITER` lines: they configure the scanner and never go to the server.
pub fn split_script(sql: &str, lex: Lex) -> Vec<Statement> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut code = false;
    for span in span::spans(sql, lex) {
        match span.kind {
            SpanKind::DelimiterLine => {
                if !code {
                    start = span.range.end;
                }
            }
            SpanKind::StatementEnd => {
                push(&mut out, sql, start..span.range.start, code);
                start = span.range.end;
                code = false;
            }
            kind if kind.is_comment() => {}
            _ => code = true,
        }
    }
    push(&mut out, sql, start..sql.len(), code);
    out
}

fn push(out: &mut Vec<Statement>, sql: &str, range: Range<usize>, has_code: bool) {
    let raw = &sql[range.clone()];
    let lead = raw.len() - raw.trim_start().len();
    let text = raw.trim();
    if has_code && !text.is_empty() {
        let start = range.start + lead;
        out.push(Statement {
            text: text.to_string(),
            range: start..start + text.len(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_sql::Dialect as _;

    fn texts(sql: &str, d: &dyn navidog_sql::Dialect) -> Vec<String> {
        split_statements(sql, d.lex())
    }

    #[test]
    fn splits_respecting_quotes_comments_and_blocks() {
        let sql = "SELECT 'a;b', \"c;d\", `e;f`; -- x;y\nSELECT 2 /* ; */;\n\
                   CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; SELECT 2; END;\n\
                   BEGIN; COMMIT;  ;  -- trailing";
        assert_eq!(
            texts(sql, &navidog_mysql::MysqlDialect),
            [
                "SELECT 'a;b', \"c;d\", `e;f`",
                "-- x;y\nSELECT 2 /* ; */",
                "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; SELECT 2; END",
                "BEGIN",
                "COMMIT",
            ]
        );
        assert_eq!(
            texts(
                "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql; SELECT 2",
                &navidog_postgres::PostgresDialect
            ),
            [
                "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql",
                "SELECT 2"
            ]
        );
    }

    #[test]
    fn handles_escapes_end_if_and_dollar_tags() {
        assert_eq!(
            texts(
                r"SELECT 'it\'s;', 'x''y;'; SELECT 1",
                &navidog_mysql::MysqlDialect
            ),
            [r"SELECT 'it\'s;', 'x''y;'", "SELECT 1"]
        );
        // Standard PostgreSQL strings take a backslash literally; E-strings escape with it
        assert_eq!(
            texts(
                r"SELECT 'C:\'; SELECT E'a\';b'; SELECT 3",
                &navidog_postgres::PostgresDialect
            ),
            [r"SELECT 'C:\'", r"SELECT E'a\';b'", "SELECT 3"]
        );
        let proc = "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END; SELECT 3";
        assert_eq!(
            texts(proc, &navidog_mysql::MysqlDialect),
            [
                "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END",
                "SELECT 3"
            ]
        );
        assert_eq!(
            texts(
                "DO $body$ BEGIN PERFORM 1; END $body$; SELECT $1",
                &navidog_postgres::PostgresDialect
            ),
            ["DO $body$ BEGIN PERFORM 1; END $body$", "SELECT $1"]
        );
        assert_eq!(
            texts(
                "SELECT foo$tag$ FROM t; DELETE FROM u",
                &navidog_postgres::PostgresDialect
            ),
            ["SELECT foo$tag$ FROM t", "DELETE FROM u"]
        );
        assert!(texts("  -- only comment\n", &navidog_sqlite::SqliteDialect).is_empty());
        // `#` is a comment in MySQL only; PostgreSQL uses it as an operator
        assert_eq!(
            texts("SELECT 1 # ;\n", &navidog_mysql::MysqlDialect),
            ["SELECT 1 # ;"]
        );
        assert_eq!(
            texts("SELECT 1 # 2; SELECT 3", &navidog_postgres::PostgresDialect).len(),
            2
        );
        // A multi-byte character right after END
        assert_eq!(
            texts(
                "CREATE PROCEDURE p() BEGIN SELECT 1; END 名; SELECT 2",
                &navidog_mysql::MysqlDialect
            ),
            ["CREATE PROCEDURE p() BEGIN SELECT 1; END 名", "SELECT 2"]
        );
    }

    #[test]
    fn delimiter_lines_switch_the_terminator_and_are_dropped() {
        let sql = "DELIMITER $$\n\
                   CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND$$\n\
                   DELIMITER ;\n\
                   CALL p();";
        assert_eq!(
            texts(sql, &navidog_mysql::MysqlDialect),
            [
                "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND",
                "CALL p()"
            ]
        );
        // Not a DELIMITER line in the middle of a statement, nor in other dialects
        assert_eq!(
            texts("SELECT delimiter FROM t;", &navidog_mysql::MysqlDialect).len(),
            1
        );
        assert_eq!(
            texts(
                "DELIMITER //\nSELECT 1//",
                &navidog_postgres::PostgresDialect
            )
            .len(),
            1
        );
        // Non-ASCII text under a custom delimiter must not panic while scanning
        assert_eq!(
            texts("DELIMITER //\nSELECT 你好//", &navidog_mysql::MysqlDialect),
            ["SELECT 你好"]
        );
    }

    #[test]
    fn session_setup_replays_use_search_path_and_set_schema() {
        assert_eq!(session_setup("USE `shop`"), Some("USE `shop`".to_string()));
        assert_eq!(
            session_setup("  set search_path TO \"app\", \"public\"; "),
            Some("set search_path TO \"app\", \"public\"".to_string())
        );
        assert_eq!(
            session_setup("SET SESSION search_path=app"),
            Some("SET SESSION search_path=app".to_string())
        );
        assert_eq!(session_setup("SET NAMES utf8"), None);
        assert_eq!(session_setup("SELECT 1"), None);
        assert_eq!(session_setup("USE shop; SELECT 1"), None);
        assert_eq!(session_setup("UPDATE t SET search_path = 1"), None);
        assert_eq!(
            session_setup("SET SCHEMA \"SYSDBA\""),
            Some("SET SCHEMA \"SYSDBA\"".to_string())
        );
        assert_eq!(session_setup("SET SCHEMA"), None);
    }

    #[test]
    fn ranges_point_at_the_trimmed_statement() {
        let sql = "  SELECT 1;\n\n-- NAME:two\nSELECT 2 ;";
        let stmts = split_script(sql, navidog_sqlite::SqliteDialect.lex());
        assert_eq!(stmts.len(), 2);
        for s in &stmts {
            assert_eq!(&sql[s.range.clone()], s.text);
        }
        assert_eq!(stmts[1].text, "-- NAME:two\nSELECT 2");
    }
}
