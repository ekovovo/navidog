//! SQL statement splitting: cuts at statement-ending semicolons, skipping strings, quoted
//! identifiers, comments, PostgreSQL `$tag$` strings, and `BEGIN … END` blocks inside
//! `CREATE TRIGGER / FUNCTION / PROCEDURE / EVENT`. MySQL scripts may switch the terminator with
//! a `DELIMITER` line, as the mysql client does.

use std::ops::Range;

use crate::dialect::Dialect;

/// One statement of a script and where it sits in the original text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    /// Trimmed, without its terminator; leading comments stay (result tabs read `-- NAME:`)
    pub text: String,
    /// Byte range of `text` in the script
    pub range: Range<usize>,
}

/// The statements' text only; see [`split_script`].
pub fn split_statements(sql: &str, dialect: Dialect) -> Vec<String> {
    split_script(sql, dialect)
        .into_iter()
        .map(|s| s.text)
        .collect()
}

/// Splits a script into statements. Empty and comment-only statements are dropped, and so are
/// MySQL `DELIMITER` lines: they configure the splitter and never go to the server.
pub fn split_script(sql: &str, dialect: Dialect) -> Vec<Statement> {
    let bytes = sql.as_bytes();
    let mysql = dialect == Dialect::MySql;
    let mut out = Vec::new();
    let mut delimiter = String::from(";");
    let mut start = 0;
    let mut i = 0;
    let mut depth = 0usize;
    let mut has_code = false;
    while i < bytes.len() {
        if mysql
            && !has_code
            && at_line_start(bytes, i)
            && let Some((end, next)) = delimiter_line(sql, i)
        {
            if let Some(next) = next {
                delimiter = next;
            }
            start = end;
            i = end;
            continue;
        }
        let c = bytes[i];
        if delimiter != ";" && sql[i..].starts_with(delimiter.as_str()) {
            push(&mut out, sql, start..i, has_code);
            i += delimiter.len();
            start = i;
            has_code = false;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                let escapes =
                    mysql || (dialect == Dialect::Postgres && c == b'\'' && e_prefix(bytes, i));
                i = skip_quoted(bytes, i, c, escapes);
                has_code = true;
                continue;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'#' if mysql => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
                continue;
            }
            b'$' if dialect == Dialect::Postgres => {
                if let Some(end) = skip_dollar(sql, i) {
                    i = end;
                    has_code = true;
                    continue;
                }
            }
            b';' if depth == 0 && delimiter == ";" => {
                push(&mut out, sql, start..i, has_code);
                start = i + 1;
                has_code = false;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let end = i + sql[i..]
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(sql.len() - i);
                let word = &sql[i..end];
                if delimiter != ";" {
                    // A custom delimiter ends statements whatever the block depth
                } else if word.eq_ignore_ascii_case("BEGIN") && is_routine(&sql[start..i]) {
                    depth += 1;
                } else if word.eq_ignore_ascii_case("END") && depth > 0 {
                    // `END IF`, `END LOOP` and the like do not close the block. Bytes, not
                    // `next[..k.len()]`, so a multi-byte character there cannot panic.
                    let next = sql[end..].trim_start().as_bytes();
                    let closes = !["IF", "LOOP", "WHILE", "REPEAT", "CASE"].iter().any(|k| {
                        next.get(..k.len())
                            .is_some_and(|b| b.eq_ignore_ascii_case(k.as_bytes()))
                    });
                    if closes {
                        depth -= 1;
                    }
                }
                has_code = true;
                i = end;
                continue;
            }
            c if !c.is_ascii_whitespace() => has_code = true,
            _ => {}
        }
        i += 1;
    }
    push(&mut out, sql, start..sql.len(), has_code);
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

fn at_line_start(bytes: &[u8], i: usize) -> bool {
    bytes[..i]
        .iter()
        .rev()
        .take_while(|b| **b != b'\n')
        .all(|b| b.is_ascii_whitespace())
}

/// A `DELIMITER <symbol>` line at `i`: where the line ends, and the new terminator (`None` when
/// the line names none).
fn delimiter_line(sql: &str, i: usize) -> Option<(usize, Option<String>)> {
    const KEYWORD: &str = "DELIMITER";
    let rest = &sql[i..];
    if !navidog_util::starts_with_ignore_ascii_case(rest, KEYWORD) {
        return None;
    }
    let after = &rest[KEYWORD.len()..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let end = rest.find('\n').map_or(sql.len(), |n| i + n);
    let next = sql[i + KEYWORD.len()..end]
        .split_whitespace()
        .next()
        .map(String::from);
    Some((end, next))
}

/// `E'…'`: a PostgreSQL escape string, where a backslash escapes the next character.
fn e_prefix(bytes: &[u8], quote: usize) -> bool {
    quote > 0
        && bytes[quote - 1].eq_ignore_ascii_case(&b'e')
        && (quote < 2 || !(bytes[quote - 2].is_ascii_alphanumeric() || bytes[quote - 2] == b'_'))
}

fn is_routine(prefix: &str) -> bool {
    let p = prefix.trim_start().to_ascii_uppercase();
    p.starts_with("CREATE")
        && ["TRIGGER", "FUNCTION", "PROCEDURE", "EVENT"]
            .iter()
            .any(|k| p.contains(k))
}

/// Skips a quoted span (a doubled quote escapes; with `escapes`, so does a backslash) and
/// returns the position after the closing quote.
fn skip_quoted(bytes: &[u8], start: usize, quote: u8, escapes: bool) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if escapes && quote != b'`' => i += 2,
            b if b == quote => {
                if bytes.get(i + 1) == Some(&quote) {
                    i += 2;
                } else {
                    return i + 1;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

/// PostgreSQL `$tag$ … $tag$`; `None` when this is not a valid opening tag.
fn skip_dollar(sql: &str, start: usize) -> Option<usize> {
    let rest = &sql[start + 1..];
    let tag_len = rest.find('$')?;
    let tag = &rest[..tag_len];
    if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || tag.starts_with(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    let delim = &sql[start..start + tag_len + 2];
    let body_start = start + tag_len + 2;
    Some(
        sql[body_start..]
            .find(delim)
            .map_or(sql.len(), |n| body_start + n + delim.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str, d: Dialect) -> Vec<String> {
        split_statements(sql, d)
    }

    #[test]
    fn splits_respecting_quotes_comments_and_blocks() {
        let sql = "SELECT 'a;b', \"c;d\", `e;f`; -- x;y\nSELECT 2 /* ; */;\n\
                   CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; SELECT 2; END;\n\
                   BEGIN; COMMIT;  ;  -- trailing";
        assert_eq!(
            texts(sql, Dialect::MySql),
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
                Dialect::Postgres
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
            texts(r"SELECT 'it\'s;', 'x''y;'; SELECT 1", Dialect::MySql),
            [r"SELECT 'it\'s;', 'x''y;'", "SELECT 1"]
        );
        // Standard PostgreSQL strings take a backslash literally; E-strings escape with it
        assert_eq!(
            texts(
                r"SELECT 'C:\'; SELECT E'a\';b'; SELECT 3",
                Dialect::Postgres
            ),
            [r"SELECT 'C:\'", r"SELECT E'a\';b'", "SELECT 3"]
        );
        let proc = "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END; SELECT 3";
        assert_eq!(
            texts(proc, Dialect::MySql),
            [
                "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END",
                "SELECT 3"
            ]
        );
        assert_eq!(
            texts(
                "DO $body$ BEGIN PERFORM 1; END $body$; SELECT $1",
                Dialect::Postgres
            ),
            ["DO $body$ BEGIN PERFORM 1; END $body$", "SELECT $1"]
        );
        assert!(texts("  -- only comment\n", Dialect::Sqlite).is_empty());
        // `#` is a comment in MySQL only; PostgreSQL uses it as an operator
        assert_eq!(texts("SELECT 1 # ;\n", Dialect::MySql), ["SELECT 1 # ;"]);
        assert_eq!(texts("SELECT 1 # 2; SELECT 3", Dialect::Postgres).len(), 2);
        // A multi-byte character right after END
        assert_eq!(
            texts(
                "CREATE PROCEDURE p() BEGIN SELECT 1; END 名; SELECT 2",
                Dialect::MySql
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
            texts(sql, Dialect::MySql),
            [
                "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND",
                "CALL p()"
            ]
        );
        // Not a DELIMITER line in the middle of a statement, nor in other dialects
        assert_eq!(texts("SELECT delimiter FROM t;", Dialect::MySql).len(), 1);
        assert_eq!(
            texts("DELIMITER //\nSELECT 1//", Dialect::Postgres).len(),
            1
        );
    }

    #[test]
    fn ranges_point_at_the_trimmed_statement() {
        let sql = "  SELECT 1;\n\n-- NAME:two\nSELECT 2 ;";
        let stmts = split_script(sql, Dialect::Sqlite);
        assert_eq!(stmts.len(), 2);
        for s in &stmts {
            assert_eq!(&sql[s.range.clone()], s.text);
        }
        assert_eq!(stmts[1].text, "-- NAME:two\nSELECT 2");
    }
}
