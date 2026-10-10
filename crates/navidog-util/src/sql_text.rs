//! SQL text helpers that need no parser: leading comments, statement names, the query editor's
//! `[$name]` parameters and snippet placeholders.

use std::ops::Range;

/// Skips whitespace and `--`, `#` and `/* */` comments at the start of `sql`.
pub fn skip_leading_comments(sql: &str) -> &str {
    let mut rest = sql;
    loop {
        rest = rest.trim_start();
        if rest.starts_with("--") || rest.starts_with('#') {
            rest = rest.find('\n').map_or("", |n| &rest[n + 1..]);
        } else if let Some(body) = rest.strip_prefix("/*") {
            rest = body.find("*/").map_or("", |n| &body[n + 2..]);
        } else {
            return rest;
        }
    }
}

/// The first word of a statement after its leading comments (`SELECT`, `WITH`, …).
pub fn first_keyword(sql: &str) -> &str {
    let rest = skip_leading_comments(sql);
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// A result tab name from a leading `-- NAME:name` or `/*NAME:name*/` (31-query-results.md §4.1).
pub fn name_hint(sql: &str) -> Option<String> {
    let rest = sql.trim_start();
    let body = if let Some(line) = rest.strip_prefix("--") {
        line.lines().next().unwrap_or("")
    } else if let Some(block) = rest.strip_prefix("/*") {
        &block[..block.find("*/")?]
    } else {
        return None;
    };
    let name = body.trim().strip_prefix("NAME:")?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// One `[$name]` parameter occurrence: byte range of the whole marker, and the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub range: std::ops::Range<usize>,
    pub name: String,
}

/// `[$name]` markers outside strings, quoted identifiers and comments.
pub fn find_params(sql: &str) -> Vec<Param> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && q == b'\'' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        if bytes.get(i + 1) == Some(&q) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
            }
            b'[' if bytes.get(i + 1) == Some(&b'$') => match sql[i + 2..].find(']') {
                Some(n) if n > 0 && !sql[i + 2..i + 2 + n].contains(['\n', '[']) => {
                    out.push(Param {
                        range: i..i + 2 + n + 1,
                        name: sql[i + 2..i + 2 + n].to_string(),
                    });
                    i += 2 + n + 1;
                }
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    out
}

/// Distinct parameter names in order of first appearance.
pub fn param_names(sql: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for p in find_params(sql) {
        if !names.contains(&p.name) {
            names.push(p.name);
        }
    }
    names
}

/// Replaces every `[$name]` with `value(name)`.
pub fn substitute_params(sql: &str, mut value: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut last = 0;
    for p in find_params(sql) {
        out.push_str(&sql[last..p.range.start]);
        out.push_str(&value(&p.name));
        last = p.range.end;
    }
    out.push_str(&sql[last..]);
    out
}

/// The snippet text with its `${…}` markers removed, and the ranges of the placeholder texts.
/// PostgreSQL's `$1` and `$$` have no braces and stay as they are.
pub fn expand_placeholders(code: &str) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(code.len());
    let mut ranges = Vec::new();
    let mut rest = code;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start + 2..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = &rest[start + 2..start + 2 + len];
        let at = out.len();
        out.push_str(name);
        ranges.push(at..at + name.len());
        rest = &rest[start + 2 + len + 1..];
    }
    out.push_str(rest);
    (out, ranges)
}

/// Wraps the selected text of `code` as a placeholder ("Add Placeholder" in the snippet editor).
pub fn add_placeholder(code: &str, selection: Range<usize>) -> String {
    if selection.is_empty() || selection.end > code.len() {
        return code.to_string();
    }
    format!(
        "{}${{{}}}{}",
        &code[..selection.start],
        &code[selection.clone()],
        &code[selection.end..]
    )
}

/// Turns every `${text}` back into `text` ("Remove Placeholder").
pub fn remove_placeholders(code: &str) -> String {
    expand_placeholders(code).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_after_comments() {
        assert_eq!(first_keyword("-- x\n /* y */ select 1"), "select");
        assert_eq!(first_keyword("# z\nWITH a AS (SELECT 1)"), "WITH");
        assert_eq!(first_keyword("  "), "");
    }

    #[test]
    fn names_from_leading_comments() {
        assert_eq!(
            name_hint("-- NAME:actors\nSELECT 1").as_deref(),
            Some("actors")
        );
        assert_eq!(
            name_hint("/*NAME: films */ SELECT 1").as_deref(),
            Some("films")
        );
        assert_eq!(name_hint("-- note\nSELECT 1"), None);
        assert_eq!(name_hint("SELECT 1 -- NAME:x"), None);
    }

    #[test]
    fn params_skip_strings_and_comments() {
        let sql = "SELECT '[$no]' FROM t -- [$no]\nWHERE a = [$id] AND b = [$name] OR c = [$id] /* [$no] */";
        assert_eq!(param_names(sql), ["id", "name"]);
        let out = substitute_params(sql, |n| if n == "id" { "1".into() } else { "'x'".into() });
        assert!(out.contains("a = 1 AND b = 'x' OR c = 1"), "{out}");
        assert!(out.contains("'[$no]'"));
        assert!(find_params("[$] [$a\nb]").is_empty());
    }

    #[test]
    fn placeholders_expand_add_and_remove() {
        let (text, ranges) = expand_placeholders("SELECT ${cols} FROM ${table} WHERE id = $1");
        assert_eq!(text, "SELECT cols FROM table WHERE id = $1");
        assert_eq!(ranges, [7..11, 17..22]);
        assert_eq!(expand_placeholders("a ${unclosed").0, "a ${unclosed");
        let code = "SELECT * FROM t";
        let with = add_placeholder(code, 14..15);
        assert_eq!(with, "SELECT * FROM ${t}");
        assert_eq!(remove_placeholders(&with), code);
        assert_eq!(add_placeholder(code, 3..3), code);
    }
}
