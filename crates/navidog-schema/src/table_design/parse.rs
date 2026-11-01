use super::*;
use navidog_sql::span::{self, LexError, SpanKind};

/// Top-level separators, ignoring quoted text, SQL comments and nested expressions.
pub(super) fn positions(text: &str, delimiter: u8, dialect: &dyn Dialect) -> DbResult<Vec<usize>> {
    validate(text, dialect)?;
    Ok(span::separators(text, dialect.lex(), delimiter))
}

/// Rejects unterminated strings and comments and unbalanced parentheses.
fn validate(text: &str, dialect: &dyn Dialect) -> DbResult<()> {
    span::check(text, dialect.lex()).map_err(|error| {
        DbError::msg(match error {
            LexError::UnclosedQuote => "Unclosed SQL quote.",
            LexError::UnclosedComment => "Unclosed SQL comment.",
            LexError::UnbalancedParentheses => "Unbalanced SQL parentheses.",
        })
    })
}

/// One SQL definition: no extra statements, no comments and no dollar quotes.
pub(super) fn fragment(text: &str, dialect: &dyn Dialect) -> DbResult<()> {
    if text.contains('\0') {
        return Err(DbError::msg(
            "A SQL definition cannot contain extra statements.",
        ));
    }
    validate(text, dialect)?;
    let spans = span::spans(text, dialect.lex());
    if spans.iter().any(|s| s.kind == SpanKind::StatementEnd) {
        return Err(DbError::msg(
            "A SQL definition cannot contain extra statements.",
        ));
    }
    if spans.iter().any(|s| s.kind.is_comment()) {
        return Err(DbError::msg(
            "SQL comments are not supported in definitions.",
        ));
    }
    if spans.iter().any(|s| s.kind == SpanKind::DollarQuoted) {
        return Err(DbError::msg(
            "Dollar quotes are not supported in definitions.",
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
pub(super) fn create(ddl: &str, name: &str, d: &dyn Dialect) -> DbResult<TableDesign> {
    let statements = navidog_db::sql::split_script(ddl, d.lex());
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
        .find(|i| positions(&body_and_options[..*i], b',', d).is_ok())
        .ok_or_else(|| DbError::msg("Invalid table body."))?;
    let body = &body_and_options[..end];
    let mut design = TableDesign {
        name: name.into(),
        unlogged,
        options: body_and_options[end + 1..].trim().into(),
        ..Default::default()
    };
    let options_upper = design.options.to_uppercase();
    if let Some(ix) = options_upper.find("PARTITION BY") {
        design.partition = design.options[ix..].trim().to_string();
        design.options = design.options[..ix].trim().to_string();
    }
    let mut begin = 0;
    for finish in positions(body, b',', d)?
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
