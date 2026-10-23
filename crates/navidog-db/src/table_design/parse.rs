use super::*;

/// Top-level separators, ignoring quoted text, SQL comments and nested expressions.
pub(super) fn positions(text: &str, delimiter: u8) -> DbResult<Vec<usize>> {
    scan(text, delimiter, true)
}
fn scan(text: &str, delimiter: u8, allow_comments: bool) -> DbResult<Vec<usize>> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut depth = 0i32;
    let mut out = Vec::new();
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' | b'[' => {
                let end = if b[i] == b'[' { b']' } else { b[i] };
                i += 1;
                loop {
                    if i >= b.len() {
                        return Err(DbError::msg("Unclosed SQL quote."));
                    }
                    if b[i] == b'\\' && end != b']' {
                        i += 2;
                        continue;
                    }
                    if b[i] == end {
                        i += 1;
                        if b.get(i) == Some(&end) {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'#' if !allow_comments => {
                return Err(DbError::msg(
                    "SQL comments are not supported in definitions.",
                ));
            }
            b'$' if !allow_comments && b.get(i + 1) == Some(&b'$') => {
                return Err(DbError::msg(
                    "Dollar quotes are not supported in definitions.",
                ));
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                if !allow_comments {
                    return Err(DbError::msg(
                        "SQL comments are not supported in definitions.",
                    ));
                }
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                if !allow_comments {
                    return Err(DbError::msg(
                        "SQL comments are not supported in definitions.",
                    ));
                }
                i += 2;
                while i + 1 < b.len() && &b[i..i + 2] != b"*/" {
                    i += 1;
                }
                if i + 1 >= b.len() {
                    return Err(DbError::msg("Unclosed SQL comment."));
                }
                i += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(DbError::msg("Unbalanced SQL parentheses."));
                }
            }
            c if c == delimiter && depth == 0 => out.push(i),
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return Err(DbError::msg("Unbalanced SQL parentheses."));
    }
    Ok(out)
}
pub(super) fn fragment(text: &str) -> DbResult<()> {
    if !scan(text, b';', false)?.is_empty() || text.contains('\0') {
        return Err(DbError::msg(
            "A SQL definition cannot contain extra statements.",
        ));
    }
    Ok(())
}
pub(super) fn ident(input: &str) -> DbResult<(String, &str)> {
    let input = input.trim_start();
    let Some(first) = input.chars().next() else {
        return Err(DbError::msg("Missing SQL identifier."));
    };
    if matches!(first, '`' | '"' | '[') {
        let end = if first == '[' { ']' } else { first };
        let mut name = String::new();
        let mut chars = input.char_indices().skip(1).peekable();
        while let Some((i, c)) = chars.next() {
            if c == end {
                if chars.peek().is_some_and(|(_, n)| *n == end) {
                    chars.next();
                    name.push(end);
                } else {
                    return Ok((name, input[i + c.len_utf8()..].trim_start()));
                }
            } else {
                name.push(c);
            }
        }
        return Err(DbError::msg("Unclosed identifier."));
    }
    let len = input
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(input.len());
    if len == 0 {
        return Err(DbError::msg("Missing SQL identifier."));
    }
    Ok((input[..len].into(), input[len..].trim_start()))
}
pub(super) fn create(ddl: &str, name: &str, d: Dialect) -> DbResult<TableDesign> {
    let statements = crate::sql::split_script(ddl, d);
    let sql = statements
        .first()
        .ok_or_else(|| DbError::msg("No CREATE TABLE definition returned."))?
        .text
        .trim();
    let upper = sql.to_uppercase();
    let unlogged = upper.starts_with("CREATE UNLOGGED TABLE ");
    if !upper.starts_with("CREATE TABLE ") && !unlogged {
        return Err(DbError::msg("Unsupported table definition."));
    }
    let rest = sql[if unlogged { 22 } else { 13 }..].trim_start();
    // Find the body while skipping quoted table/schema names.
    let (_, mut rest) = ident(rest)?;
    if rest.starts_with('.') {
        (_, rest) = ident(&rest[1..])?;
    }
    let start = rest
        .find('(')
        .ok_or_else(|| DbError::msg("Missing table body."))?;
    let body_and_options = &rest[start + 1..];
    // Try closing parentheses from the end; the body must be balanced.
    let end = body_and_options
        .char_indices()
        .filter(|(_, c)| *c == ')')
        .map(|(i, _)| i)
        .find(|i| positions(&body_and_options[..*i], b',').is_ok())
        .ok_or_else(|| DbError::msg("Invalid table body."))?;
    let body = &body_and_options[..end];
    let mut design = TableDesign {
        name: name.into(),
        unlogged,
        options: body_and_options[end + 1..].trim().into(),
        ..Default::default()
    };
    let mut begin = 0;
    for finish in positions(body, b',')?
        .into_iter()
        .chain(std::iter::once(body.len()))
    {
        let part = body[begin..finish].trim();
        begin = finish + 1;
        let (head, rest) = ident(part)?;
        let constraint = !part.starts_with(['`', '"', '['])
            && matches!(
                head.to_uppercase().as_str(),
                "CONSTRAINT"
                    | "PRIMARY"
                    | "UNIQUE"
                    | "FOREIGN"
                    | "CHECK"
                    | "KEY"
                    | "INDEX"
                    | "FULLTEXT"
                    | "SPATIAL"
                    | "EXCLUDE"
            );
        if constraint {
            design.constraints.push(part.into());
        } else {
            let mut c = DesignColumn::new(design.columns.len() as u64 + 1, head.clone(), rest);
            c.original = Some(head);
            design.columns.push(c);
        }
    }
    for s in statements.iter().skip(1) {
        design.indexes.push(s.text.clone());
    }
    Ok(design)
}
