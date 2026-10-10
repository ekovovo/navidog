//! Visual filter and sort (F-DATA-006). The builder edits this model; the viewer stores the SQL it
//! emits in the WHERE / ORDER BY fields the query already uses.
//!
//! ponytail: one pair of parentheses around the whole condition, not a nested tree. A condition the
//! parser does not recognize stays in text mode (`parse_where` returns `None`).

use navidog_db::dialect::Dialect;
use navidog_util::starts_with_ignore_ascii_case;
use rust_i18n::t;

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

    pub fn choices(dialect: Dialect) -> Vec<Self> {
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
        if dialect == Dialect::Postgres {
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

pub fn where_sql(filter: &Filter, dialect: Dialect) -> String {
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

pub fn parse_where(sql: &str, dialect: Dialect) -> Option<Filter> {
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

pub fn order_sql(sorts: &[Sort], dialect: Dialect) -> String {
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

pub fn parse_order(sql: &str, dialect: Dialect) -> Option<Vec<Sort>> {
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

fn predicate(rule: &Rule, dialect: Dialect) -> Option<String> {
    if rule.op == Op::Custom {
        let text = rule.value.trim();
        if text.is_empty() {
            return None;
        }
        return Some(if rule.negate {
            format!("NOT ({text})")
        } else {
            text.to_string()
        });
    }
    if rule.field.is_empty() {
        return None;
    }
    let field = dialect.quote_ident(&rule.field);
    let body = match rule.op {
        Op::Eq => format!("{field} = {}", sql_value(dialect, &rule.value)),
        Op::Ne => format!("{field} != {}", sql_value(dialect, &rule.value)),
        Op::Lt => format!("{field} < {}", sql_value(dialect, &rule.value)),
        Op::Le => format!("{field} <= {}", sql_value(dialect, &rule.value)),
        Op::Gt => format!("{field} > {}", sql_value(dialect, &rule.value)),
        Op::Ge => format!("{field} >= {}", sql_value(dialect, &rule.value)),
        Op::Contains => like(&field, dialect, &rule.value, Like::Contains, false, false),
        Op::NotContains => like(&field, dialect, &rule.value, Like::Contains, true, false),
        Op::ContainsI => like(&field, dialect, &rule.value, Like::Contains, false, true),
        Op::NotContainsI => like(&field, dialect, &rule.value, Like::Contains, true, true),
        Op::Starts => like(&field, dialect, &rule.value, Like::Starts, false, false),
        Op::NotStarts => like(&field, dialect, &rule.value, Like::Starts, true, false),
        Op::Ends => like(&field, dialect, &rule.value, Like::Ends, false, false),
        Op::NotEnds => like(&field, dialect, &rule.value, Like::Ends, true, false),
        Op::IsNull => format!("{field} IS NULL"),
        Op::NotNull => format!("{field} IS NOT NULL"),
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
    dialect: Dialect,
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

fn in_list(dialect: Dialect, value: &str) -> String {
    value
        .split(',')
        .map(|part| sql_value(dialect, part))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_value(dialect: Dialect, raw: &str) -> String {
    let t = raw.trim();
    if is_number(t) {
        t.to_string()
    } else {
        dialect.string_literal(t)
    }
}

fn is_number(s: &str) -> bool {
    let s = s
        .strip_prefix('+')
        .or_else(|| s.strip_prefix('-'))
        .unwrap_or(s);
    let mut parts = s.split('.');
    let whole = parts.next().unwrap_or("");
    let frac = parts.next();
    parts.next().is_none()
        && !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

fn unwrap_group(sql: &str, dialect: Dialect) -> (String, bool, bool) {
    if let Some(inner) = strip_wrapping_not(sql)
        && split_joins(&inner, dialect).len() > 1
    {
        return (inner, true, true);
    }
    if let Some(inner) = strip_wrapping_paren(sql)
        && split_joins(&inner, dialect).len() > 1
    {
        return (inner, true, false);
    }
    (sql.to_string(), false, false)
}

fn strip_wrapping_not(sql: &str) -> Option<String> {
    let rest = eat_kw(sql.trim(), "NOT")?;
    strip_wrapping_paren(rest)
}

fn strip_wrapping_paren(sql: &str) -> Option<String> {
    let t = sql.trim();
    if !t.starts_with('(') || !t.ends_with(')') {
        return None;
    }
    if paren_end(t) != Some(t.len() - 1) {
        return None;
    }
    Some(t[1..t.len() - 1].trim().to_string())
}

/// `(or_before, piece)`.
fn split_joins(sql: &str, dialect: Dialect) -> Vec<(bool, String)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut or_flag = false;
    let mut i = 0;
    let mut depth = 0i32;
    let mut in_str = false;
    // `BETWEEN 1 AND 2` uses AND as its second value, not as a join.
    let mut pending_between = 0i32;
    let bytes = sql.as_bytes();
    while i < bytes.len() {
        if in_str {
            i = skip_string_body(sql, i, dialect);
            in_str = false;
            continue;
        }
        match bytes[i] {
            b'\'' => {
                in_str = true;
                i += 1;
            }
            b'`' | b'"' => i = skip_quoted(sql, i),
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0
                && let Some(consumed) = word_span(&sql[i..], "BETWEEN") =>
            {
                pending_between += 1;
                i += consumed;
            }
            _ if depth == 0
                && pending_between > 0
                && let Some(consumed) = word_span(&sql[i..], "AND") =>
            {
                pending_between -= 1;
                i += consumed;
            }
            _ if depth == 0 => {
                if let Some((consumed, or)) = match_join(&sql[i..]) {
                    parts.push((or_flag, sql[start..i].to_string()));
                    or_flag = or;
                    i += consumed;
                    start = i;
                    continue;
                }
                i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
            _ => i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1),
        }
    }
    parts.push((or_flag, sql[start..].to_string()));
    parts
}

fn skip_quoted(sql: &str, i: usize) -> usize {
    let quote = sql.as_bytes()[i];
    let mut i = i + 1;
    let bytes = sql.as_bytes();
    while i < bytes.len() {
        if bytes[i] == quote {
            if i + 1 < bytes.len() && bytes[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    i
}

fn word_span(s: &str, kw: &str) -> Option<usize> {
    if !match_word(s, kw) {
        return None;
    }
    let rest = s.trim_start();
    Some(s.len() - rest.len() + kw.len())
}

fn match_word(s: &str, kw: &str) -> bool {
    let rest = s.trim_start();
    if rest.len() == s.len() || !starts_with_ignore_ascii_case(rest, kw) {
        return false;
    }
    !rest[kw.len()..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whitespace, then AND / OR as a whole word, then the following whitespace.
fn match_join(s: &str) -> Option<(usize, bool)> {
    let rest = s.trim_start();
    if rest.len() == s.len() {
        return None;
    }
    let (word, or) = if starts_with_ignore_ascii_case(rest, "AND") {
        (3, false)
    } else if starts_with_ignore_ascii_case(rest, "OR") {
        (2, true)
    } else {
        return None;
    };
    let after = &rest[word..];
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    if !after.starts_with(|c: char| c.is_ascii_whitespace()) && !after.is_empty() {
        return None;
    }
    let tail = after.trim_start();
    Some((s.len() - tail.len(), or))
}

fn parse_pred(sql: &str, dialect: Dialect) -> Option<Rule> {
    let (sql, negate) = if let Some(inner) = strip_wrapping_not(sql) {
        if split_joins(&inner, dialect).len() != 1 {
            return None;
        }
        (inner, true)
    } else {
        (sql.to_string(), false)
    };
    let (field, rest) = parse_ident(sql.trim(), dialect)?;
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

fn parse_op(rest: &str, dialect: Dialect) -> Option<(Op, String, String, &str)> {
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

fn parse_like(
    rest: &str,
    dialect: Dialect,
    negated: bool,
    ilike: bool,
) -> Option<(Op, String, String, &str)> {
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

fn parse_between(
    rest: &str,
    dialect: Dialect,
    negated: bool,
) -> Option<(Op, String, String, &str)> {
    let (a, rest) = parse_sql_value(rest, dialect)?;
    let rest = eat_kw(rest.trim_start(), "AND")?;
    let (b, rest) = parse_sql_value(rest, dialect)?;
    let op = if negated { Op::NotBetween } else { Op::Between };
    Some((op, a, b, rest))
}

fn parse_in(rest: &str, dialect: Dialect, negated: bool) -> Option<(Op, String, String, &str)> {
    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return None;
    }
    let end = paren_end(rest)?;
    let inner = &rest[1..end];
    let after = &rest[end + 1..];
    let mut values = Vec::new();
    for part in split_commas(inner, dialect) {
        let (v, left) = parse_sql_value(part.trim(), dialect)?;
        if !left.trim().is_empty() {
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

fn parse_sql_value(s: &str, dialect: Dialect) -> Option<(String, &str)> {
    let t = s.trim_start();
    if t.starts_with('\'') {
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

fn parse_string(s: &str, dialect: Dialect) -> Option<(String, &str)> {
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
        if dialect == Dialect::MySql && bytes[i] == b'\\' && i + 1 < bytes.len() {
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

fn parse_ident(s: &str, dialect: Dialect) -> Option<(String, &str)> {
    let s = s.trim_start();
    let quote = match dialect {
        Dialect::MySql => '`',
        Dialect::Postgres | Dialect::Sqlite => '"',
    };
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

fn split_commas(sql: &str, dialect: Dialect) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut depth = 0i32;
    let mut in_str = false;
    let bytes = sql.as_bytes();
    while i < bytes.len() {
        if in_str {
            i = skip_string_body(sql, i, dialect);
            in_str = false;
            continue;
        }
        match bytes[i] {
            b'\'' => {
                in_str = true;
                i += 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            b',' if depth == 0 => {
                parts.push(&sql[start..i]);
                i += 1;
                start = i;
            }
            _ => i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1),
        }
    }
    parts.push(&sql[start..]);
    parts
}

/// `i` points at the first byte inside a quote that started on the previous byte. Returns the
/// index just after the closing quote.
fn skip_string_body(sql: &str, mut i: usize, dialect: Dialect) -> usize {
    let bytes = sql.as_bytes();
    while i < bytes.len() {
        if dialect == Dialect::MySql && bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1 + sql[i + 1..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            continue;
        }
        if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    i
}

fn paren_end(sql: &str) -> Option<usize> {
    if !sql.starts_with('(') {
        return None;
    }
    let bytes = sql.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        if in_str {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 1 + sql[i + 1..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                continue;
            }
            if bytes[i] == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_str = false;
            }
            i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            continue;
        }
        match bytes[i] {
            b'\'' => {
                in_str = true;
                i += 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1),
        }
    }
    None
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

    fn round_trip(op: Op, value: &str, value2: &str, dialect: Dialect) {
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
        for dialect in [Dialect::MySql, Dialect::Postgres, Dialect::Sqlite] {
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
        round_trip(Op::ContainsI, "ab", "", Dialect::Postgres);
        round_trip(Op::NotContainsI, "a_b", "", Dialect::Postgres);
    }

    #[test]
    fn joins_group_and_negate_round_trip() {
        let dialect = Dialect::Sqlite;
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
        let dialect = Dialect::MySql;
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
        let dialect = Dialect::Postgres;
        let filter = Filter {
            rules: vec![rule("a\"b", Op::Eq, "1", "")],
            ..Filter::default()
        };
        let sql = where_sql(&filter, dialect);
        assert_eq!(sql, "\"a\"\"b\" = 1");
        assert_eq!(parse_where(&sql, dialect).unwrap().rules[0].field, "a\"b");
        assert!(parse_where("1 = 1; DROP TABLE t", dialect).is_none());
        assert!(parse_where("name = 'x' OR 1", dialect).is_none());
        assert!(parse_order("name ASC LIMIT 1", dialect).is_none());
    }

    #[test]
    fn non_ascii_text_does_not_panic() {
        for dialect in [Dialect::MySql, Dialect::Postgres, Dialect::Sqlite] {
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
