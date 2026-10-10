//! Format menu text transforms (30-query-editor.md §3): beautify, minify, comments, brackets.

use std::ops::Range;

/// One lexical token, enough to check that formatting kept the SQL the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Word(String),
    /// Strings, quoted identifiers and dollar-quoted bodies, verbatim
    Quoted(String),
    /// Comments, with their inner whitespace collapsed
    Comment(String),
    Punct(char),
}

/// Skips a quoted span; a doubled quote escapes, and with `backslash` so does a backslash.
fn quoted_end(b: &[u8], start: usize, q: u8, backslash: bool) -> usize {
    let mut i = start + 1;
    while i < b.len() {
        if backslash && b[i] == b'\\' && q != b'`' {
            i += 2;
            continue;
        }
        if b[i] == q {
            if b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `$tag$ … $tag$` starting at `start`.
fn dollar_end(s: &str, start: usize) -> Option<usize> {
    let rest = &s[start + 1..];
    let n = rest.find('$')?;
    let tag = &rest[..n];
    if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || tag.starts_with(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    let delim = &s[start..start + n + 2];
    let body = start + n + 2;
    Some(
        s[body..]
            .find(delim)
            .map_or(s.len(), |k| body + k + delim.len()),
    )
}

/// Spans of `sql` that are strings, quoted identifiers or comments: (range, is_comment).
fn spans(sql: &str, hash_comments: bool) -> Vec<(Range<usize>, bool)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i = quoted_end(b, i, q, hash_comments);
                out.push((start..i, false));
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(b.len(), |n| i + n);
                out.push((start..i, true));
            }
            b'#' if hash_comments => {
                i = sql[i..].find('\n').map_or(b.len(), |n| i + n);
                out.push((start..i, true));
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
                out.push((start..i, true));
            }
            b'$' => match dollar_end(sql, i) {
                Some(end) => {
                    i = end;
                    out.push((start..i, false));
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    out
}

/// Tokens of `sql` outside whitespace; word case is folded so keyword case does not count.
pub fn tokens(sql: &str, hash_comments: bool) -> Vec<Token> {
    let mut out = Vec::new();
    let mut last = 0;
    let push_plain = |text: &str, out: &mut Vec<Token>| {
        let mut word = String::new();
        for ch in text.chars() {
            if ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '@' || ch == '$' {
                word.push(ch.to_ascii_lowercase());
                continue;
            }
            if !word.is_empty() {
                out.push(Token::Word(std::mem::take(&mut word)));
            }
            if !ch.is_whitespace() {
                out.push(Token::Punct(ch));
            }
        }
        if !word.is_empty() {
            out.push(Token::Word(word));
        }
    };
    for (range, comment) in spans(sql, hash_comments) {
        push_plain(&sql[last..range.start], &mut out);
        let text = &sql[range.clone()];
        out.push(if comment {
            Token::Comment(text.split_whitespace().collect::<Vec<_>>().join(" "))
        } else {
            Token::Quoted(text.to_string())
        });
        last = range.end;
    }
    push_plain(&sql[last..], &mut out);
    // `a.b` and `a . b` are the same SQL
    let mut merged: Vec<Token> = Vec::new();
    for t in out {
        match (merged.last_mut(), &t) {
            (Some(Token::Word(w)), Token::Word(n)) if w.ends_with('.') || n.starts_with('.') => {
                w.push_str(n)
            }
            _ => merged.push(t),
        }
    }
    merged
}

#[derive(Clone, Copy, Debug)]
pub struct BeautifyOptions {
    /// Spaces per indent level; `None` indents with tabs
    pub indent_spaces: Option<u8>,
    pub uppercase: bool,
    /// Parentheses whose content fits this length stay on one line
    pub max_inline: usize,
    pub postgres: bool,
    /// MySQL: `#` starts a comment and backslashes escape in strings
    pub mysql: bool,
}

/// Beautify SQL with `sqlformat`. Strings, quoted identifiers and dollar bodies are swapped for
/// placeholders first so the formatter cannot touch them; comments stay for it to place.
pub fn beautify(sql: &str, o: &BeautifyOptions) -> String {
    let mut protected = String::with_capacity(sql.len());
    let mut kept = Vec::new();
    let mut last = 0;
    for (range, comment) in spans(sql, o.mysql) {
        if comment {
            continue;
        }
        protected.push_str(&sql[last..range.start]);
        protected.push_str(&format!("__navidog_q{}__", kept.len()));
        kept.push(&sql[range.clone()]);
        last = range.end;
    }
    protected.push_str(&sql[last..]);
    // sqlformat knows `--` and `/* */`; MySQL `#` comments become `--` and back
    let hashes: Vec<Range<usize>> = if o.mysql {
        spans(&protected, true)
            .into_iter()
            .filter(|(r, c)| *c && protected[r.clone()].starts_with('#'))
            .map(|(r, _)| r)
            .collect()
    } else {
        Vec::new()
    };
    let mut input = protected.clone();
    for r in hashes.iter().rev() {
        input.replace_range(r.start..r.start + 1, "--#");
    }
    let options = sqlformat::FormatOptions {
        indent: match o.indent_spaces {
            Some(n) => sqlformat::Indent::Spaces(n),
            None => sqlformat::Indent::Tabs,
        },
        uppercase: Some(o.uppercase),
        max_inline_block: o.max_inline,
        dialect: if o.postgres {
            sqlformat::Dialect::PostgreSql
        } else {
            sqlformat::Dialect::Generic
        },
        ..Default::default()
    };
    let mut out = sqlformat::format(&input, &sqlformat::QueryParams::None, &options);
    if o.mysql {
        out = out.replace("--#", "#");
    }
    for (i, original) in kept.iter().enumerate().rev() {
        out = out.replace(&format!("__navidog_q{i}__"), original);
        // Uppercasing may have touched the placeholder
        out = out.replace(&format!("__NAVIDOG_Q{i}__"), original);
    }
    out
}

/// Simplify SQL: whitespace outside strings collapses to one space, each statement takes one
/// line, and line comments become block comments so they cannot swallow the code after them.
pub fn minify(sql: &str, mysql: bool) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut last = 0;
    let mut pending_space = false;
    let flush_plain = |text: &str, out: &mut String, pending: &mut bool| {
        for ch in text.chars() {
            if ch.is_whitespace() {
                *pending = true;
                continue;
            }
            if *pending && !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            *pending = false;
            out.push(ch);
            if ch == ';' {
                out.push('\n');
            }
        }
    };
    for (range, comment) in spans(sql, mysql) {
        flush_plain(&sql[last..range.start], &mut out, &mut pending_space);
        let text = &sql[range.clone()];
        let piece = if comment && !text.starts_with("/*") {
            let body = text.trim_start_matches("--").trim_start_matches('#').trim();
            format!("/* {} */", body.replace("*/", "* /"))
        } else {
            text.to_string()
        };
        if pending_space && !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        pending_space = false;
        out.push_str(&piece);
        last = range.end;
    }
    flush_plain(&sql[last..], &mut out, &mut pending_space);
    out.trim().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentMode {
    Comment,
    Uncomment,
    /// Comment all lines unless every line already is
    Toggle,
}

/// Comments or uncomments whole lines with a leading `-- `. Blank lines are left alone.
pub fn comment_lines(text: &str, mode: CommentMode) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let commented = |l: &str| l.trim_start().starts_with("--");
    let comment = match mode {
        CommentMode::Comment => true,
        CommentMode::Uncomment => false,
        CommentMode::Toggle => lines.iter().any(|l| !l.trim().is_empty() && !commented(l)),
    };
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                return l.to_string();
            }
            let indent = &l[..l.len() - l.trim_start().len()];
            let body = l.trim_start();
            if comment {
                format!("{indent}-- {body}")
            } else if let Some(rest) = body.strip_prefix("--") {
                format!("{indent}{}", rest.strip_prefix(' ').unwrap_or(rest))
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `selection` widened to whole lines (the caret's line when nothing is selected). A selection
/// ending at the start of a line does not take that line.
pub fn line_range(text: &str, selection: Range<usize>) -> Range<usize> {
    let end_at = if selection.end > selection.start && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let start = text[..selection.start].rfind('\n').map_or(0, |n| n + 1);
    let end = text[end_at..].find('\n').map_or(text.len(), |n| end_at + n);
    start..end
}

/// Matching `(` `)` pairs outside strings and comments, as (open, close) byte offsets.
pub fn bracket_pairs(sql: &str, hash_comments: bool) -> Vec<(usize, usize)> {
    let skip = spans(sql, hash_comments);
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let mut spans = skip.iter().peekable();
    for (i, b) in sql.bytes().enumerate() {
        while spans.peek().is_some_and(|(r, _)| r.end <= i) {
            spans.next();
        }
        if spans.peek().is_some_and(|(r, _)| r.start <= i) {
            continue;
        }
        match b {
            b'(' => stack.push(i),
            b')' => {
                if let Some(open) = stack.pop() {
                    out.push((open, i));
                }
            }
            _ => {}
        }
    }
    out.sort_unstable();
    out
}

/// The bracket next to `caret` (after it, else before it) and its partner.
pub fn bracket_at(pairs: &[(usize, usize)], caret: usize) -> Option<(usize, usize)> {
    for pos in [Some(caret), caret.checked_sub(1)].into_iter().flatten() {
        if let Some(p) = pairs.iter().find(|(o, c)| *o == pos || *c == pos) {
            return Some(*p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(postgres: bool, mysql: bool) -> BeautifyOptions {
        BeautifyOptions {
            indent_spaces: Some(4),
            uppercase: true,
            max_inline: 50,
            postgres,
            mysql,
        }
    }

    /// Beautifying keeps every token, whatever the dialect's quoting and comments.
    #[test]
    fn beautify_keeps_the_token_sequence() {
        let cases = [
            (
                "select a.id, `weird name`, 'it''s -- not a comment' from t a -- trailing\nwhere a.x = \"q\" # hash\n and b in (1,2,3); select 2",
                false,
                true,
            ),
            (
                "select $$ keep   this ; $$, $tag$ also $$ inside $tag$, x::int from \"T\" where y = E'a\\'b' /* block */ and z = $1",
                true,
                false,
            ),
            (
                "select [id], name from t where note = 'x  y' -- c\norder by name desc limit 10",
                false,
                false,
            ),
        ];
        for (sql, pg, my) in cases {
            let out = beautify(sql, &opts(pg, my));
            assert_eq!(tokens(&out, my), tokens(sql, my), "{sql}\n=>\n{out}");
            assert!(out.contains("SELECT"), "{out}");
        }
        // Short parentheses stay inline
        let out = beautify("select f(a, b) from t", &opts(false, false));
        assert!(out.contains("f(a, b)"), "{out}");
    }

    #[test]
    fn minify_collapses_whitespace_and_line_comments() {
        let sql = "SELECT  a,\n   'x   y'  -- note\nFROM t;\n\nSELECT 2 # hash\n";
        assert_eq!(
            minify(sql, true),
            "SELECT a, 'x   y' /* note */ FROM t;\nSELECT 2 /* hash */"
        );
        assert_eq!(
            tokens(&minify(sql, true), true).len(),
            tokens(sql, true).len()
        );
    }

    #[test]
    fn comments_toggle_by_line() {
        assert_eq!(comment_lines("a\n  b", CommentMode::Toggle), "-- a\n  -- b");
        assert_eq!(comment_lines("-- a\n  -- b", CommentMode::Toggle), "a\n  b");
        // One uncommented line comments all
        assert_eq!(
            comment_lines("-- a\nb", CommentMode::Toggle),
            "-- -- a\n-- b"
        );
        assert_eq!(comment_lines("--a\n\nb", CommentMode::Uncomment), "a\n\nb");
    }

    #[test]
    fn line_ranges_and_brackets() {
        let text = "ab\ncd\nef";
        assert_eq!(line_range(text, 4..4), 3..5);
        assert_eq!(line_range(text, 1..4), 0..5);
        assert_eq!(line_range(text, 0..3), 0..2);
        let sql = "f(a, '(', g(b)) -- )\n";
        let pairs = bracket_pairs(sql, false);
        assert_eq!(pairs, [(1, 14), (11, 13)]);
        assert_eq!(bracket_at(&pairs, 1), Some((1, 14)));
        assert_eq!(bracket_at(&pairs, 15), Some((1, 14)));
        assert_eq!(bracket_at(&pairs, 5), None);
    }
}
