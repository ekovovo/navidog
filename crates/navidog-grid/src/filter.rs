//! Visual filter and sort (F-DATA-006). The builder edits this model; the viewer stores the SQL it
//! emits in the WHERE / ORDER BY fields the query already uses.
//!
//! ponytail: one pair of parentheses around the whole condition, not a nested tree. A condition the
//! parser does not recognize stays in text mode (`parse_where` returns `None`).

use navidog_sql::dialect::Dialect;
use navidog_sql::span;
use navidog_util::starts_with_ignore_ascii_case;
use rust_i18n::t;
use span::SpanKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    NotContains,
    /// PostgreSQL `ILIKE`. Hidden for other dialects.
    ContainsI,
    NotContainsI,
    Starts,
    NotStarts,
    Ends,
    NotEnds,
    IsNull,
    NotNull,
    Empty,
    NotEmpty,
    Between,
    NotBetween,
    InList,
    NotInList,
    /// Inserted as written. Does not round-trip through the parser.
    Custom,
}

impl Op {
    pub fn key(self) -> &'static str {
        match self {
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Lt => "lt",
            Self::Le => "le",
            Self::Gt => "gt",
            Self::Ge => "ge",
            Self::Contains => "contains",
            Self::NotContains => "not_contains",
            Self::ContainsI => "contains_i",
            Self::NotContainsI => "not_contains_i",
            Self::Starts => "starts",
            Self::NotStarts => "not_starts",
            Self::Ends => "ends",
            Self::NotEnds => "not_ends",
            Self::IsNull => "is_null",
            Self::NotNull => "not_null",
            Self::Empty => "empty",
            Self::NotEmpty => "not_empty",
            Self::Between => "between",
            Self::NotBetween => "not_between",
            Self::InList => "in_list",
            Self::NotInList => "not_in_list",
            Self::Custom => "custom",
        }
    }

    pub fn parse_key(key: &str) -> Option<Self> {
        Some(match key {
            "eq" => Self::Eq,
            "ne" => Self::Ne,
            "lt" => Self::Lt,
            "le" => Self::Le,
            "gt" => Self::Gt,
            "ge" => Self::Ge,
            "contains" => Self::Contains,
            "not_contains" => Self::NotContains,
            "contains_i" => Self::ContainsI,
            "not_contains_i" => Self::NotContainsI,
            "starts" => Self::Starts,
            "not_starts" => Self::NotStarts,
            "ends" => Self::Ends,
            "not_ends" => Self::NotEnds,
            "is_null" => Self::IsNull,
            "not_null" => Self::NotNull,
            "empty" => Self::Empty,
            "not_empty" => Self::NotEmpty,
            "between" => Self::Between,
            "not_between" => Self::NotBetween,
            "in_list" => Self::InList,
            "not_in_list" => Self::NotInList,
            "custom" => Self::Custom,
            _ => return None,
        })
    }

    pub fn needs_value(self) -> bool {
        !matches!(
            self,
            Self::IsNull | Self::NotNull | Self::Empty | Self::NotEmpty
        )
    }

    pub fn needs_second(self) -> bool {
        matches!(self, Self::Between | Self::NotBetween)
    }

    pub fn choices(dialect: &dyn Dialect) -> Vec<Self> {
        let mut ops = vec![
            Self::Eq,
            Self::Ne,
            Self::Lt,
            Self::Le,
            Self::Gt,
            Self::Ge,
            Self::Contains,
            Self::NotContains,
        ];
        if dialect.features().case_insensitive_like {
            ops.push(Self::ContainsI);
            ops.push(Self::NotContainsI);
        }
        ops.extend([
            Self::Starts,
            Self::NotStarts,
            Self::Ends,
            Self::NotEnds,
            Self::IsNull,
            Self::NotNull,
            Self::Empty,
            Self::NotEmpty,
            Self::Between,
            Self::NotBetween,
            Self::InList,
            Self::NotInList,
            Self::Custom,
        ]);
        ops
    }

    pub fn label(self) -> String {
        match self {
            Self::Eq => t!("viewer.op_eq").to_string(),
            Self::Ne => t!("viewer.op_ne").to_string(),
            Self::Lt => t!("viewer.op_lt").to_string(),
            Self::Gt => t!("viewer.op_gt").to_string(),
            Self::Le => t!("viewer.op_le").to_string(),
            Self::Ge => t!("viewer.op_ge").to_string(),
            Self::Contains => t!("viewer.op_contains").to_string(),
            Self::NotContains => t!("viewer.op_not_contains").to_string(),
            Self::ContainsI => t!("viewer.op_contains_i").to_string(),
            Self::NotContainsI => t!("viewer.op_not_contains_i").to_string(),
            Self::Starts => t!("viewer.op_starts").to_string(),
            Self::NotStarts => t!("viewer.op_not_starts").to_string(),
            Self::Ends => t!("viewer.op_ends").to_string(),
            Self::NotEnds => t!("viewer.op_not_ends").to_string(),
            Self::IsNull => t!("viewer.op_is_null").to_string(),
            Self::NotNull => t!("viewer.op_not_null").to_string(),
            Self::Empty => t!("viewer.op_empty").to_string(),
            Self::NotEmpty => t!("viewer.op_not_empty").to_string(),
            Self::Between => t!("viewer.op_between").to_string(),
            Self::NotBetween => t!("viewer.op_not_between").to_string(),
            Self::InList => t!("viewer.op_in_list").to_string(),
            Self::NotInList => t!("viewer.op_not_in_list").to_string(),
            Self::Custom => t!("viewer.op_custom").to_string(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    pub field: String,
    pub op: Op,
    pub value: String,
    pub value2: String,
    /// Join to the previous rule with OR. Ignored on the first rule.
    pub or: bool,
    pub negate: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Filter {
    pub rules: Vec<Rule>,
    /// Wrap every rule in one pair of parentheses.
    pub grouped: bool,
    pub group_negate: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sort {
    pub field: String,
    pub asc: bool,
}

pub fn where_sql(filter: &Filter, dialect: &dyn Dialect) -> String {
    let mut body = String::new();
    for rule in &filter.rules {
        let Some(pred) = predicate(rule, dialect) else {
            continue;
        };
        if !body.is_empty() {
            body.push_str(if rule.or { " OR " } else { " AND " });
        }
        body.push_str(&pred);
    }
    if body.is_empty() {
        return String::new();
    }
    if filter.grouped && filter.group_negate {
        format!("NOT ({body})")
    } else if filter.grouped {
        format!("({body})")
    } else {
        body
    }
}

pub fn parse_where(sql: &str, dialect: &dyn Dialect) -> Option<Filter> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Some(Filter::default());
    }
    let (body, grouped, group_negate) = unwrap_group(sql, dialect);
    let pieces = split_joins(&body, dialect);
    if pieces.is_empty() {
        return None;
    }
    let mut rules = Vec::new();
    for (or, piece) in pieces {
        if piece.trim().is_empty() {
            return None;
        }
        let mut rule = parse_pred(piece.trim(), dialect)?;
        rule.or = or;
        rules.push(rule);
    }
    Some(Filter {
        rules,
        grouped,
        group_negate,
    })
}

pub fn order_sql(sorts: &[Sort], dialect: &dyn Dialect) -> String {
    sorts
        .iter()
        .filter(|s| !s.field.is_empty())
        .map(|s| {
            format!(
                "{} {}",
                dialect.quote_ident(&s.field),
                if s.asc { "ASC" } else { "DESC" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn parse_order(sql: &str, dialect: &dyn Dialect) -> Option<Vec<Sort>> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Some(Vec::new());
    }
    let mut sorts = Vec::new();
    for part in split_commas(sql, dialect) {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let (field, rest) = parse_ident(part, dialect)?;
        let rest = rest.trim();
        let asc = if rest.is_empty() || rest.eq_ignore_ascii_case("ASC") {
            true
        } else if rest.eq_ignore_ascii_case("DESC") {
            false
        } else {
            return None;
        };
        sorts.push(Sort { field, asc });
    }
    Some(sorts)
}

fn predicate(rule: &Rule, dialect: &dyn Dialect) -> Option<String> {
    if rule.op == Op::Custom {
        let text = rule.value.trim();
        if text.is_empty() {
            return None;
        }
        // Parenthesized, so an OR inside cannot regroup the rules around it
        let not = if rule.negate { "NOT " } else { "" };
        return Some(format!("{not}({text})"));
    }
    if rule.field.is_empty() {
        return None;
    }
    let field = dialect.quote_ident(&rule.field);
    // PostgreSQL has no LIKE for numbers or dates; the cast makes "contains" work on any column
    let text_field = if dialect.features().like_text_cast {
        format!("{field}::text")
    } else {
        field.clone()
    };
    let body = match rule.op {
        Op::Eq => format!("{field} = {}", sql_value(dialect, &rule.value)),
        Op::Ne => format!("{field} != {}", sql_value(dialect, &rule.value)),
        Op::Lt => format!("{field} < {}", sql_value(dialect, &rule.value)),
        Op::Le => format!("{field} <= {}", sql_value(dialect, &rule.value)),
        Op::Gt => format!("{field} > {}", sql_value(dialect, &rule.value)),
        Op::Ge => format!("{field} >= {}", sql_value(dialect, &rule.value)),
        Op::Contains => like(
            &text_field,
            dialect,
            &rule.value,
            Like::Contains,
            false,
            false,
        ),
        Op::NotContains => like(
            &text_field,
            dialect,
            &rule.value,
            Like::Contains,
            true,
            false,
        ),
        Op::ContainsI => like(
            &text_field,
            dialect,
            &rule.value,
            Like::Contains,
            false,
            true,
        ),
        Op::NotContainsI => like(
            &text_field,
            dialect,
            &rule.value,
            Like::Contains,
            true,
            true,
        ),
        Op::Starts => like(
            &text_field,
            dialect,
            &rule.value,
            Like::Starts,
            false,
            false,
        ),
        Op::NotStarts => like(&text_field, dialect, &rule.value, Like::Starts, true, false),
        Op::Ends => like(&text_field, dialect, &rule.value, Like::Ends, false, false),
        Op::NotEnds => like(&text_field, dialect, &rule.value, Like::Ends, true, false),
        Op::IsNull => format!("{field} IS NULL"),
        Op::NotNull => format!("{field} IS NOT NULL"),
        // A server that treats '' as NULL (Dameng COMPATIBLE_MODE=2) never matches an
        // empty-string comparison (140-dameng.md §5.2).
        Op::Empty if dialect.features().empty_string_is_null => format!("{field} IS NULL"),
        Op::NotEmpty if dialect.features().empty_string_is_null => {
            format!("{field} IS NOT NULL")
        }
        Op::Empty => format!("{field} = ''"),
        Op::NotEmpty => format!("{field} <> ''"),
        Op::Between => format!(
            "{field} BETWEEN {} AND {}",
            sql_value(dialect, &rule.value),
            sql_value(dialect, &rule.value2)
        ),
        Op::NotBetween => format!(
            "{field} NOT BETWEEN {} AND {}",
            sql_value(dialect, &rule.value),
            sql_value(dialect, &rule.value2)
        ),
        Op::InList => format!("{field} IN ({})", in_list(dialect, &rule.value)),
        Op::NotInList => format!("{field} NOT IN ({})", in_list(dialect, &rule.value)),
        Op::Custom => unreachable!(),
    };
    Some(if rule.negate {
        format!("NOT ({body})")
    } else {
        body
    })
}

#[derive(Clone, Copy)]
enum Like {
    Contains,
    Starts,
    Ends,
}

fn like(
    field: &str,
    dialect: &dyn Dialect,
    value: &str,
    kind: Like,
    negated: bool,
    ilike: bool,
) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = match kind {
        Like::Contains => format!("%{escaped}%"),
        Like::Starts => format!("{escaped}%"),
        Like::Ends => format!("%{escaped}"),
    };
    let op = if ilike { "ILIKE" } else { "LIKE" };
    let not = if negated { "NOT " } else { "" };
    format!(
        "{field} {not}{op} {} ESCAPE {}",
        dialect.string_literal(&pattern),
        dialect.string_literal("\\")
    )
}

fn in_list(dialect: &dyn Dialect, value: &str) -> String {
    value
        .split(',')
        .map(|part| sql_value(dialect, part))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Always a string literal: all three databases convert it to the column's type, while a bare
/// number fails on a PostgreSQL text column, matches every non-numeric string in MySQL, and turns
/// `007` into 7.
fn sql_value(dialect: &dyn Dialect, raw: &str) -> String {
    dialect.string_literal(raw.trim())
}

fn unwrap_group(sql: &str, dialect: &dyn Dialect) -> (String, bool, bool) {
    if let Some(inner) = strip_wrapping_not(sql, dialect)
        && split_joins(&inner, dialect).len() > 1
    {
        return (inner, true, true);
    }
    if let Some(inner) = strip_wrapping_paren(sql, dialect)
        && split_joins(&inner, dialect).len() > 1
    {
        return (inner, true, false);
    }
    (sql.to_string(), false, false)
}

fn strip_wrapping_not(sql: &str, dialect: &dyn Dialect) -> Option<String> {
    let rest = eat_kw(sql.trim(), "NOT")?;
    strip_wrapping_paren(rest, dialect)
}

fn strip_wrapping_paren(sql: &str, dialect: &dyn Dialect) -> Option<String> {
    let t = sql.trim();
    if !t.starts_with('(') || !t.ends_with(')') {
        return None;
    }
    if paren_end(t, dialect) != Some(t.len() - 1) {
        return None;
    }
    Some(t[1..t.len() - 1].trim().to_string())
}

/// `(or_before, piece)`: splits `WHERE` text at the top-level `AND` / `OR` joins.
fn split_joins(sql: &str, dialect: &dyn Dialect) -> Vec<(bool, String)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut or_flag = false;
    // `BETWEEN 1 AND 2` uses AND as its second value, not as a join.
    let mut pending_between = 0i32;
    for span in span::spans(sql, dialect.lex()) {
        if span.kind != SpanKind::Word || span.depth != 0 {
            continue;
        }
        // A join is a keyword on its own line of text, not the tail of `1AND` or `AND(`.
        if !sql[..span.range.start].ends_with(char::is_whitespace) {
            continue;
        }
        let word = span.text(sql);
        if word.eq_ignore_ascii_case("BETWEEN") {
            pending_between += 1;
            continue;
        }
        if pending_between > 0 && word.eq_ignore_ascii_case("AND") {
            pending_between -= 1;
            continue;
        }
        let or = if word.eq_ignore_ascii_case("AND") {
            false
        } else if word.eq_ignore_ascii_case("OR") {
            true
        } else {
            continue;
        };
        let after = &sql[span.range.end..];
        if !(after.is_empty() || after.starts_with(char::is_whitespace)) {
            continue;
        }
        parts.push((or_flag, sql[start..span.range.start].trim().to_string()));
        or_flag = or;
        start = span.range.end;
    }
    parts.push((or_flag, sql[start..].trim().to_string()));
    parts
}

fn parse_pred(sql: &str, dialect: &dyn Dialect) -> Option<Rule> {
    let (sql, negate) = if let Some(inner) = strip_wrapping_not(sql, dialect) {
        if split_joins(&inner, dialect).len() != 1 {
            return None;
        }
        (inner, true)
    } else {
        (sql.to_string(), false)
    };
    let (field, rest) = parse_ident(sql.trim(), dialect)?;
    // The cast `predicate` writes before LIKE on PostgreSQL
    let rest = match rest.strip_prefix("::") {
        Some(cast) if dialect.features().like_text_cast => eat_kw(cast, "text")?,
        _ => rest,
    };
    let rest = rest.trim_start();
    let (op, value, value2, rest) = parse_op(rest, dialect)?;
    if !rest.trim().is_empty() {
        return None;
    }
    Some(Rule {
        field,
        op,
        value,
        value2,
        or: false,
        negate,
    })
}

fn parse_op<'a>(rest: &'a str, dialect: &dyn Dialect) -> Option<(Op, String, String, &'a str)> {
    if let Some(after) = eat_kw(rest, "IS NOT NULL") {
        return Some((Op::NotNull, String::new(), String::new(), after));
    }
    if let Some(after) = eat_kw(rest, "IS NULL") {
        return Some((Op::IsNull, String::new(), String::new(), after));
    }
    if let Some(after) = eat_kw(rest, "NOT ILIKE") {
        return parse_like(after, dialect, true, true);
    }
    if let Some(after) = eat_kw(rest, "NOT LIKE") {
        return parse_like(after, dialect, true, false);
    }
    if let Some(after) = eat_kw(rest, "ILIKE") {
        return parse_like(after, dialect, false, true);
    }
    if let Some(after) = eat_kw(rest, "LIKE") {
        return parse_like(after, dialect, false, false);
    }
    if let Some(after) = eat_kw(rest, "NOT BETWEEN") {
        return parse_between(after, dialect, true);
    }
    if let Some(after) = eat_kw(rest, "BETWEEN") {
        return parse_between(after, dialect, false);
    }
    if let Some(after) = eat_kw(rest, "NOT IN") {
        return parse_in(after, dialect, true);
    }
    if let Some(after) = eat_kw(rest, "IN") {
        return parse_in(after, dialect, false);
    }
    let (op, after) = if let Some(after) = rest.strip_prefix("!=") {
        (Op::Ne, after)
    } else if let Some(after) = rest.strip_prefix("<>") {
        (Op::Ne, after)
    } else if let Some(after) = rest.strip_prefix("<=") {
        (Op::Le, after)
    } else if let Some(after) = rest.strip_prefix(">=") {
        (Op::Ge, after)
    } else if let Some(after) = rest.strip_prefix('<') {
        (Op::Lt, after)
    } else if let Some(after) = rest.strip_prefix('>') {
        (Op::Gt, after)
    } else {
        let after = rest.strip_prefix('=')?;
        (Op::Eq, after)
    };
    let (value, after) = parse_sql_value(after, dialect)?;
    let op = if value.is_empty() && op == Op::Eq {
        Op::Empty
    } else if value.is_empty() && op == Op::Ne {
        Op::NotEmpty
    } else {
        op
    };
    Some((op, value, String::new(), after))
}

fn parse_like<'a>(
    rest: &'a str,
    dialect: &dyn Dialect,
    negated: bool,
    ilike: bool,
) -> Option<(Op, String, String, &'a str)> {
    let (pattern, rest) = parse_string(rest.trim_start(), dialect)?;
    let rest = if let Some(after) = eat_kw(rest.trim_start(), "ESCAPE") {
        let (_, after) = parse_string(after.trim_start(), dialect)?;
        after
    } else {
        rest
    };
    let (op, value) = classify_like(&pattern, negated, ilike)?;
    Some((op, value, String::new(), rest))
}

fn classify_like(pattern: &str, negated: bool, ilike: bool) -> Option<(Op, String)> {
    let both = pattern.starts_with('%') && pattern.ends_with('%') && pattern.len() >= 2;
    let (body, kind) = if both {
        (
            unescape_like(&pattern[1..pattern.len() - 1])?,
            Like::Contains,
        )
    } else if pattern.ends_with('%') && !pattern.starts_with('%') {
        (unescape_like(&pattern[..pattern.len() - 1])?, Like::Starts)
    } else if pattern.starts_with('%') && !pattern.ends_with('%') {
        (unescape_like(&pattern[1..])?, Like::Ends)
    } else {
        return None;
    };
    let op = match (kind, negated, ilike) {
        (Like::Contains, false, false) => Op::Contains,
        (Like::Contains, true, false) => Op::NotContains,
        (Like::Contains, false, true) => Op::ContainsI,
        (Like::Contains, true, true) => Op::NotContainsI,
        (Like::Starts, false, false) => Op::Starts,
        (Like::Starts, true, false) => Op::NotStarts,
        (Like::Ends, false, false) => Op::Ends,
        (Like::Ends, true, false) => Op::NotEnds,
        _ => return None,
    };
    Some((op, body))
}

fn unescape_like(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(chars.next()?);
        } else if c == '%' || c == '_' {
            return None;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn parse_between<'a>(
    rest: &'a str,
    dialect: &dyn Dialect,
    negated: bool,
) -> Option<(Op, String, String, &'a str)> {
    let (a, rest) = parse_sql_value(rest, dialect)?;
    let rest = eat_kw(rest.trim_start(), "AND")?;
    let (b, rest) = parse_sql_value(rest, dialect)?;
    let op = if negated { Op::NotBetween } else { Op::Between };
    Some((op, a, b, rest))
}

fn parse_in<'a>(
    rest: &'a str,
    dialect: &dyn Dialect,
    negated: bool,
) -> Option<(Op, String, String, &'a str)> {
    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return None;
    }
    let end = paren_end(rest, dialect)?;
    let inner = &rest[1..end];
    let after = &rest[end + 1..];
    let mut values = Vec::new();
    for part in split_commas(inner, dialect) {
        let (v, left) = parse_sql_value(part.trim(), dialect)?;
        // The builder separates list values with commas; a comma inside one stays in text mode
        if !left.trim().is_empty() || v.contains(',') {
            return None;
        }
        values.push(v);
    }
    if values.is_empty() {
        return None;
    }
    let op = if negated { Op::NotInList } else { Op::InList };
    Some((op, values.join(", "), String::new(), after))
}

fn parse_sql_value<'a>(s: &'a str, dialect: &dyn Dialect) -> Option<(String, &'a str)> {
    let t = s.trim_start();
    if t.starts_with('\'') || postgres_escape_string(t, dialect).is_some() {
        return parse_string(t, dialect);
    }
    let bytes = t.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = usize::from(bytes[0] == b'+' || bytes[0] == b'-');
    let digits_at = i;
    let mut dot = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            i += 1;
        } else if bytes[i] == b'.' && !dot {
            dot = true;
            i += 1;
        } else {
            break;
        }
    }
    if i == digits_at || bytes[i - 1] == b'.' {
        return None;
    }
    Some((t[..i].to_string(), &t[i..]))
}

/// The rest after `E` when `s` is a PostgreSQL escape string (`E'a\\b'`), which `string_literal`
/// writes for values with a backslash.
fn postgres_escape_string<'a>(s: &'a str, dialect: &dyn Dialect) -> Option<&'a str> {
    let rest = s.strip_prefix(['E', 'e'])?;
    (dialect.lex().escape_string_prefix && rest.starts_with('\'')).then_some(rest)
}

fn parse_string<'a>(s: &'a str, dialect: &dyn Dialect) -> Option<(String, &'a str)> {
    let (s, backslash_escapes) = match postgres_escape_string(s, dialect) {
        Some(rest) => (rest, true),
        None => (s, dialect.lex().backslash_strings),
    };
    if !s.starts_with('\'') {
        return None;
    }
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                out.push('\'');
                i += 2;
                continue;
            }
            return Some((out, &s[i + 1..]));
        }
        if backslash_escapes && bytes[i] == b'\\' && i + 1 < bytes.len() {
            let ch = s[i + 1..].chars().next()?;
            out.push(ch);
            i += 1 + ch.len_utf8();
            continue;
        }
        let ch = s[i..].chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    None
}

fn parse_ident<'a>(s: &'a str, dialect: &dyn Dialect) -> Option<(String, &'a str)> {
    let s = s.trim_start();
    let quote = dialect.ident_quote();
    let bytes = s.as_bytes();
    if bytes.first().copied() == Some(quote as u8) {
        let mut out = String::new();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == quote as u8 {
                if i + 1 < bytes.len() && bytes[i + 1] == quote as u8 {
                    out.push(quote);
                    i += 2;
                    continue;
                }
                return Some((out, &s[i + 1..]));
            }
            let ch = s[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
        return None;
    }
    let first = s.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let end = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    Some((s[..end].to_string(), &s[end..]))
}

fn split_commas<'a>(sql: &'a str, dialect: &dyn Dialect) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for pos in span::separators(sql, dialect.lex(), b',') {
        parts.push(&sql[start..pos]);
        start = pos + 1;
    }
    parts.push(&sql[start..]);
    parts
}

fn paren_end(sql: &str, dialect: &dyn Dialect) -> Option<usize> {
    if !sql.starts_with('(') {
        return None;
    }
    let spans = span::spans(sql, dialect.lex());
    let level = spans
        .iter()
        .find(|s| s.kind == SpanKind::OpenParen && s.range.start == 0)?
        .depth
        + 1;
    spans
        .iter()
        .find(|s| s.kind == SpanKind::CloseParen && s.depth == level)
        .map(|s| s.range.start)
}

fn eat_kw<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let s = s.trim_start();
    if !starts_with_ignore_ascii_case(s, kw) {
        return None;
    }
    let after = &s[kw.len()..];
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    Some(after)
}

#[cfg(test)]
mod consistency_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(field: &str, op: Op, value: &str, value2: &str) -> Rule {
        Rule {
            field: field.into(),
            op,
            value: value.into(),
            value2: value2.into(),
            or: false,
            negate: false,
        }
    }

    #[test]
    fn empty_comparisons_become_null_checks_when_the_server_stores_empty_as_null() {
        // Dameng COMPATIBLE_MODE=2: '' IS NULL, so = '' and <> '' never match.
        let dialect = &navidog_dameng::DamengDialect {
            case_sensitive: false,
            compatible_mode: 2,
        };
        let filter = Filter {
            rules: vec![rule("s", Op::Empty, "", "")],
            ..Filter::default()
        };
        assert_eq!(where_sql(&filter, dialect), "\"s\" IS NULL");
        let filter = Filter {
            rules: vec![rule("s", Op::NotEmpty, "", "")],
            ..Filter::default()
        };
        assert_eq!(where_sql(&filter, dialect), "\"s\" IS NOT NULL");
        // Other servers keep the empty-string comparison.
        let filter = Filter {
            rules: vec![rule("s", Op::Empty, "", "")],
            ..Filter::default()
        };
        assert_eq!(where_sql(&filter, &navidog_mysql::MysqlDialect), "`s` = ''");
    }

    fn round_trip(op: Op, value: &str, value2: &str, dialect: &dyn Dialect) {
        let filter = Filter {
            rules: vec![rule("name", op, value, value2)],
            ..Filter::default()
        };
        let sql = where_sql(&filter, dialect);
        let parsed = parse_where(&sql, dialect).unwrap_or_else(|| panic!("parse {sql}"));
        assert_eq!(parsed.rules.len(), 1, "{sql}");
        assert_eq!(parsed.rules[0].op, op, "{sql}");
        assert_eq!(parsed.rules[0].field, "name", "{sql}");
        assert_eq!(parsed.rules[0].value, value, "{sql}");
        assert_eq!(parsed.rules[0].value2, value2, "{sql}");
        assert!(!parsed.rules[0].negate, "{sql}");
        assert_eq!(where_sql(&parsed, dialect), sql);
    }

    #[test]
    fn every_operator_round_trips() {
        for dialect in [
            &navidog_mysql::MysqlDialect as &dyn Dialect,
            &navidog_postgres::PostgresDialect,
            &navidog_sqlite::SqliteDialect,
        ] {
            for op in [
                Op::Eq,
                Op::Ne,
                Op::Lt,
                Op::Le,
                Op::Gt,
                Op::Ge,
                Op::Contains,
                Op::NotContains,
                Op::Starts,
                Op::NotStarts,
                Op::Ends,
                Op::NotEnds,
                Op::Between,
                Op::NotBetween,
                Op::InList,
                Op::NotInList,
            ] {
                let (value, value2) = match op {
                    Op::Between | Op::NotBetween => ("1", "10"),
                    Op::InList | Op::NotInList => ("a, 2", ""),
                    Op::Contains
                    | Op::NotContains
                    | Op::Starts
                    | Op::NotStarts
                    | Op::Ends
                    | Op::NotEnds => ("a%b_c\\d", ""),
                    _ => ("PENELOPE", ""),
                };
                round_trip(op, value, value2, dialect);
            }
            round_trip(Op::IsNull, "", "", dialect);
            round_trip(Op::NotNull, "", "", dialect);
            round_trip(Op::Empty, "", "", dialect);
            round_trip(Op::NotEmpty, "", "", dialect);
        }
        round_trip(Op::ContainsI, "ab", "", &navidog_postgres::PostgresDialect);
        round_trip(
            Op::NotContainsI,
            "a_b",
            "",
            &navidog_postgres::PostgresDialect,
        );
    }

    #[test]
    fn joins_group_and_negate_round_trip() {
        let dialect = &navidog_sqlite::SqliteDialect;
        let mut filter = Filter {
            rules: vec![
                rule("id", Op::Gt, "10", ""),
                Rule {
                    or: true,
                    ..rule("name", Op::Eq, "a'b", "")
                },
            ],
            grouped: true,
            group_negate: true,
        };
        filter.rules[0].negate = true;
        let sql = where_sql(&filter, dialect);
        let parsed = parse_where(&sql, dialect).expect(&sql);
        assert!(parsed.grouped && parsed.group_negate, "{sql}");
        assert!(parsed.rules[0].negate);
        assert!(parsed.rules[1].or);
        assert_eq!(parsed.rules[1].value, "a'b");
        assert_eq!(where_sql(&parsed, dialect), sql);
    }

    #[test]
    fn bare_text_and_order_parse() {
        let dialect = &navidog_mysql::MysqlDialect;
        let parsed = parse_where("id > 10 AND name = 'a'", dialect).unwrap();
        assert_eq!(parsed.rules.len(), 2);
        assert_eq!(parsed.rules[0].op, Op::Gt);
        assert_eq!(parsed.rules[1].value, "a");
        let sorts = parse_order("name DESC, id", dialect).unwrap();
        assert_eq!(
            sorts,
            vec![
                Sort {
                    field: "name".into(),
                    asc: false
                },
                Sort {
                    field: "id".into(),
                    asc: true
                }
            ]
        );
        assert_eq!(order_sql(&sorts, dialect), "`name` DESC, `id` ASC");
    }

    #[test]
    fn quoted_identifier_and_rejection() {
        let dialect = &navidog_postgres::PostgresDialect;
        let filter = Filter {
            rules: vec![rule("a\"b", Op::Eq, "1", "")],
            ..Filter::default()
        };
        let sql = where_sql(&filter, dialect);
        assert_eq!(sql, "\"a\"\"b\" = '1'");
        assert_eq!(parse_where(&sql, dialect).unwrap().rules[0].field, "a\"b");
        assert!(parse_where("1 = 1; DROP TABLE t", dialect).is_none());
        assert!(parse_where("name = 'x' OR 1", dialect).is_none());
        assert!(parse_order("name ASC LIMIT 1", dialect).is_none());
    }

    #[test]
    fn values_are_literals_and_postgres_like_casts() {
        let one = |rule: Rule, dialect| {
            where_sql(
                &Filter {
                    rules: vec![rule],
                    ..Filter::default()
                },
                dialect,
            )
        };
        // A number-looking value stays a literal, so '007' keeps its zeros through a round trip
        let sql = one(
            rule("code", Op::Eq, "007", ""),
            &navidog_mysql::MysqlDialect,
        );
        assert_eq!(sql, "`code` = '007'");
        assert_eq!(
            parse_where(&sql, &navidog_mysql::MysqlDialect)
                .unwrap()
                .rules[0]
                .value,
            "007"
        );
        // Text a user typed with a bare number still parses
        assert_eq!(
            parse_where("id = 5", &navidog_mysql::MysqlDialect)
                .unwrap()
                .rules[0]
                .value,
            "5"
        );
        // PostgreSQL casts before LIKE so integer columns work, and the cast parses back
        let sql = one(
            rule("id", Op::Contains, "1", ""),
            &navidog_postgres::PostgresDialect,
        );
        assert_eq!(sql, "\"id\"::text LIKE '%1%' ESCAPE E'\\\\'");
        let parsed = parse_where(&sql, &navidog_postgres::PostgresDialect).unwrap();
        assert_eq!(
            (parsed.rules[0].op, parsed.rules[0].field.as_str()),
            (Op::Contains, "id")
        );
        assert_eq!(
            one(
                rule("id", Op::Contains, "1", ""),
                &navidog_mysql::MysqlDialect
            ),
            "`id` LIKE '%1%' ESCAPE '\\\\'"
        );
        // A list value with a comma in it cannot come back as builder values
        assert!(parse_where("name IN ('a,b', 'c')", &navidog_mysql::MysqlDialect).is_none());
        // A custom condition keeps its own OR together
        let filter = Filter {
            rules: vec![
                rule("a", Op::Eq, "1", ""),
                rule("", Op::Custom, "x = 1 OR y = 2", ""),
            ],
            ..Filter::default()
        };
        assert_eq!(
            where_sql(&filter, &navidog_sqlite::SqliteDialect),
            "\"a\" = '1' AND (x = 1 OR y = 2)"
        );
    }

    #[test]
    fn non_ascii_text_does_not_panic() {
        for dialect in [
            &navidog_mysql::MysqlDialect as &dyn Dialect,
            &navidog_postgres::PostgresDialect,
            &navidog_sqlite::SqliteDialect,
        ] {
            for sql in [
                "名称 = 1",
                "NOT 名称 = 1",
                "col名称 = 1",
                "a = 1 和 b = 2",
                "\"名\" = 'é' OR 名 ILIKE 'x'",
            ] {
                let _ = parse_where(sql, dialect);
            }
        }
    }
}
