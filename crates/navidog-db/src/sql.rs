//! SQL statement splitting: cuts at statement-ending semicolons, skipping strings, quoted
//! identifiers, comments, PostgreSQL `$tag$` strings, and `BEGIN … END` blocks inside
//! `CREATE TRIGGER / FUNCTION / PROCEDURE / EVENT`.

/// Splits into statements (trailing semicolon and surrounding whitespace removed; empty and
/// comment-only statements dropped).
pub fn split_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut depth = 0usize;
    let mut has_code = false;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(bytes, i, c);
                has_code = true;
                continue;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'#' => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
                continue;
            }
            b'$' => {
                if let Some(end) = skip_dollar(sql, i) {
                    i = end;
                    has_code = true;
                    continue;
                }
            }
            b';' if depth == 0 => {
                push(&mut out, &sql[start..i], has_code);
                start = i + 1;
                has_code = false;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let end = i + sql[i..]
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(sql.len() - i);
                let word = &sql[i..end];
                if word.eq_ignore_ascii_case("BEGIN") && is_routine(&sql[start..i]) {
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
    push(&mut out, &sql[start..], has_code);
    out
}

fn push(out: &mut Vec<String>, stmt: &str, has_code: bool) {
    let s = stmt.trim();
    if has_code && !s.is_empty() {
        out.push(s.to_string());
    }
}

fn is_routine(prefix: &str) -> bool {
    let p = prefix.trim_start().to_ascii_uppercase();
    p.starts_with("CREATE")
        && ["TRIGGER", "FUNCTION", "PROCEDURE", "EVENT"]
            .iter()
            .any(|k| p.contains(k))
}

/// Skips a quoted span (a doubled quote escapes, a backslash escapes the next character) and
/// returns the position after the closing quote.
fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quote == b'\'' => i += 2,
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
    use super::split_statements;

    #[test]
    fn splits_respecting_quotes_comments_and_blocks() {
        let sql = "SELECT 'a;b', \"c;d\", `e;f`; -- x;y\nSELECT 2 /* ; */;\n\
                   CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; SELECT 2; END;\n\
                   CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n\
                   BEGIN; COMMIT;  ;  -- trailing";
        assert_eq!(
            split_statements(sql),
            [
                "SELECT 'a;b', \"c;d\", `e;f`",
                "-- x;y\nSELECT 2 /* ; */",
                "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; SELECT 2; END",
                "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql",
                "BEGIN",
                "COMMIT",
            ]
        );
    }

    #[test]
    fn handles_escapes_end_if_and_dollar_tags() {
        assert_eq!(
            split_statements(r"SELECT 'it\'s;', 'x''y;'; SELECT 1"),
            [r"SELECT 'it\'s;', 'x''y;'", "SELECT 1"]
        );
        let proc = "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END; SELECT 3";
        assert_eq!(
            split_statements(proc),
            [
                "CREATE PROCEDURE p() BEGIN IF 1 THEN SELECT 1; END IF; END",
                "SELECT 3"
            ]
        );
        assert_eq!(
            split_statements("DO $body$ BEGIN PERFORM 1; END $body$; SELECT $1"),
            ["DO $body$ BEGIN PERFORM 1; END $body$", "SELECT $1"]
        );
        assert!(split_statements("  -- only comment\n").is_empty());
        // A multi-byte character right after END
        assert_eq!(
            split_statements("CREATE PROCEDURE p() BEGIN SELECT 1; END 名; SELECT 2"),
            ["CREATE PROCEDURE p() BEGIN SELECT 1; END 名", "SELECT 2"]
        );
    }
}
