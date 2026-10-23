//! Lossless edits of known table options; unknown clauses remain in the SQL tail.
use super::*;
use std::collections::BTreeMap;

pub const MYSQL_OPTIONS: &[&str] = &[
    "engine",
    "charset",
    "collation",
    "auto_increment",
    "row_format",
    "avg_row_length",
    "max_rows",
    "min_rows",
    "key_block_size",
    "stats_auto_recalc",
    "stats_persistent",
    "stats_sample_pages",
    "checksum",
    "delay_key_write",
    "pack_keys",
    "insert_method",
    "connection",
    "data_directory",
    "index_directory",
    "tablespace",
    "compression",
    "encryption",
    "table_comment",
];
pub const POSTGRES_OPTIONS: &[&str] = &[
    "owner",
    "tablespace",
    "inherits",
    "fillfactor",
    "autovacuum_enabled",
    "table_comment",
];

#[derive(Clone, Debug)]
struct Token {
    start: usize,
    end: usize,
}
fn tokens(s: &str) -> DbResult<Vec<Token>> {
    parse::fragment(s)?;
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if b[i] == b'=' || b[i] == b',' {
            i += 1;
        } else {
            let mut quote = 0;
            let mut depth = 0;
            while i < b.len() {
                let c = b[i];
                if quote != 0 {
                    if c == b'\\' {
                        i += 2;
                        continue;
                    }
                    if c == quote {
                        if b.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        quote = 0;
                    }
                } else {
                    match c {
                        b'\'' | b'"' | b'`' => quote = c,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        c if depth == 0 && (c.is_ascii_whitespace() || c == b'=' || c == b',') => {
                            break;
                        }
                        _ => {}
                    }
                }
                i += 1;
            }
        }
        out.push(Token { start, end: i });
    }
    Ok(out)
}
fn text<'a>(s: &'a str, t: &Token) -> &'a str {
    &s[t.start..t.end]
}
fn decoded(s: &str) -> String {
    if s.len() >= 2
        && s.starts_with(['\'', '"', '`'])
        && s.as_bytes()[0] == s.as_bytes()[s.len() - 1]
    {
        let q = s.chars().next().unwrap();
        s[1..s.len() - 1]
            .replace(&format!("{q}{q}"), &q.to_string())
            .replace("\\'", "'")
            .replace("\\\\", "\\")
    } else {
        s.into()
    }
}
fn aliases(key: &str) -> Vec<&str> {
    match key {
        "charset" => vec![
            "DEFAULT CHARACTER SET",
            "DEFAULT CHARSET",
            "CHARACTER SET",
            "CHARSET",
        ],
        "collation" => vec!["DEFAULT COLLATE", "COLLATE"],
        "data_directory" => vec!["DATA DIRECTORY"],
        "index_directory" => vec!["INDEX DIRECTORY"],
        "table_comment" => vec!["COMMENT"],
        _ => vec![key],
    }
}
fn mysql_span(s: &str, key: &str) -> DbResult<Option<(usize, usize, String)>> {
    let ts = tokens(s)?;
    let mut found = None;
    let mut i = 0;
    while i < ts.len() {
        let mut matched = None;
        for name in MYSQL_OPTIONS
            .iter()
            .copied()
            .chain(std::iter::once("union"))
        {
            for alias in aliases(name) {
                let ws: Vec<_> = alias.split_whitespace().collect();
                if ws.iter().enumerate().all(|(j, w)| {
                    ts.get(i + j)
                        .is_some_and(|t| text(s, t).eq_ignore_ascii_case(w))
                }) {
                    let mut v = i + ws.len();
                    if ts.get(v).is_some_and(|t| text(s, t) == "=") {
                        v += 1;
                    }
                    let Some(value) = ts.get(v) else {
                        return Err(DbError::msg("Missing table option value."));
                    };
                    matched = Some((name, ts[i].start, value.end, decoded(text(s, value)), v + 1));
                    break;
                }
            }
            if matched.is_some() {
                break;
            }
        }
        if let Some((name, a, b, value, next)) = matched {
            if name == key {
                if found.is_some() {
                    return Err(DbError::msg(
                        "Duplicate table option; use Advanced to resolve it.",
                    ));
                }
                found = Some((a, b, value));
            }
            i = next;
        } else {
            i += 1;
        }
    }
    Ok(found)
}
fn pg_clause(s: &str, key: &str) -> DbResult<Option<(usize, usize, String)>> {
    let ts = tokens(s)?;
    let mut found = None;
    for (i, t) in ts.iter().enumerate() {
        if text(s, t).eq_ignore_ascii_case(key) {
            let v = ts
                .get(i + 1)
                .ok_or_else(|| DbError::msg("Missing PostgreSQL option value."))?;
            if found.is_some() {
                return Err(DbError::msg("Duplicate PostgreSQL option."));
            }
            found = Some((t.start, v.end, text(s, v).to_string()));
        }
    }
    Ok(found)
}
pub(super) fn pg_storage(s: &str) -> DbResult<BTreeMap<String, String>> {
    let Some((_, _, group)) = pg_clause(s, "WITH")? else {
        return Ok(BTreeMap::new());
    };
    let body = group
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .ok_or_else(|| DbError::msg("Invalid PostgreSQL storage parameters."))?;
    let mut values = BTreeMap::new();
    for field in split_field_list(body)? {
        let (k, v) = field
            .split_once('=')
            .ok_or_else(|| DbError::msg("Invalid PostgreSQL storage parameter."))?;
        let k = k.trim().to_ascii_lowercase();
        if k.is_empty()
            || !k
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            || values.insert(k, v.trim().into()).is_some()
        {
            return Err(DbError::msg(
                "Invalid or duplicate PostgreSQL storage parameter.",
            ));
        }
    }
    Ok(values)
}
pub fn table_option(s: &str, key: &str, d: Dialect) -> DbResult<String> {
    match d {
        Dialect::MySql => Ok(mysql_span(s, key)?.map(|v| v.2).unwrap_or_default()),
        Dialect::Postgres => match key {
            "fillfactor" | "autovacuum_enabled" => {
                Ok(pg_storage(s)?.remove(key).unwrap_or_default())
            }
            "tablespace" => Ok(pg_clause(s, "TABLESPACE")?
                .map(|v| decoded(&v.2))
                .unwrap_or_default()),
            "inherits" => Ok(pg_clause(s, "INHERITS")?
                .map(|v| {
                    v.2.trim_start_matches('(')
                        .trim_end_matches(')')
                        .to_string()
                })
                .unwrap_or_default()),
            _ => Err(DbError::msg("Unsupported PostgreSQL option.")),
        },
        Dialect::Sqlite => {
            let ts = tokens(s)?;
            let normalized = ts
                .iter()
                .map(|t| text(s, t).to_ascii_uppercase())
                .filter(|s| s != ",")
                .collect::<Vec<_>>()
                .join(" ");
            if ![
                "",
                "STRICT",
                "WITHOUT ROWID",
                "STRICT WITHOUT ROWID",
                "WITHOUT ROWID STRICT",
            ]
            .contains(&normalized.as_str())
            {
                return Err(DbError::msg("Unknown SQLite table options; use Advanced."));
            }
            Ok(match key {
                "strict" => normalized.contains("STRICT"),
                "without_rowid" => normalized.contains("WITHOUT ROWID"),
                _ => return Err(DbError::msg("Unsupported SQLite option.")),
            }
            .to_string())
        }
    }
}
fn replace(s: &str, span: Option<(usize, usize, String)>, replacement: &str) -> String {
    if let Some((a, b, _)) = span {
        format!("{}{}{}", &s[..a], replacement, &s[b..])
            .trim()
            .into()
    } else {
        format!("{} {replacement}", s.trim()).trim().into()
    }
}
fn number(value: &str, min: u64, max: u64) -> DbResult<()> {
    if !value.is_empty()
        && (!value.bytes().all(|b| b.is_ascii_digit())
            || value.parse::<u64>().ok().is_none_or(|n| n < min || n > max))
    {
        return Err(DbError::msg(format!(
            "Expected an integer from {min} to {max}."
        )));
    }
    Ok(())
}
pub fn edit_table_option(s: &str, key: &str, value: &str, d: Dialect) -> DbResult<String> {
    match d {
        Dialect::MySql => {
            if !MYSQL_OPTIONS.contains(&key) {
                return Err(DbError::msg("Unsupported MySQL table option."));
            }
            let old = mysql_span(s, key)?;
            let literal = matches!(
                key,
                "connection"
                    | "data_directory"
                    | "index_directory"
                    | "compression"
                    | "encryption"
                    | "table_comment"
            );
            let value = if literal { value } else { value.trim() };
            let rendered = if literal {
                d.string_literal(value)
            } else if key == "tablespace" {
                d.quote_ident(value)
            } else {
                if !value.is_empty()
                    && !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(DbError::msg("Invalid table option value."));
                }
                if matches!(
                    key,
                    "auto_increment"
                        | "avg_row_length"
                        | "max_rows"
                        | "min_rows"
                        | "key_block_size"
                        | "stats_sample_pages"
                ) {
                    number(value, 0, u64::MAX)?;
                }
                value.into()
            };
            let sqlkey = match key {
                "charset" => "DEFAULT CHARACTER SET",
                "collation" => "COLLATE",
                "data_directory" => "DATA DIRECTORY",
                "index_directory" => "INDEX DIRECTORY",
                "table_comment" => "COMMENT",
                _ => key,
            };
            let replacement = if value.is_empty() && !literal {
                String::new()
            } else {
                format!("{}={rendered}", sqlkey.to_ascii_uppercase())
            };
            Ok(replace(s, old, &replacement))
        }
        Dialect::Postgres => match key {
            "fillfactor" | "autovacuum_enabled" => {
                if key == "fillfactor" {
                    number(value, 10, 100)?;
                } else if !["", "true", "false"].contains(&value) {
                    return Err(DbError::msg("Autovacuum must be true, false, or empty."));
                }
                let mut storage = pg_storage(s)?;
                if value.is_empty() {
                    storage.remove(key);
                } else {
                    storage.insert(key.into(), value.into());
                }
                let replacement = if storage.is_empty() {
                    String::new()
                } else {
                    format!(
                        "WITH ({})",
                        storage
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let old = pg_clause(s, "WITH")?;
                // Storage parameters follow INHERITS and precede TABLESPACE.
                if old.is_none() {
                    let at = pg_clause(s, "INHERITS")?.map(|v| v.1).unwrap_or(0);
                    Ok(format!(
                        "{} {replacement} {}",
                        s[..at].trim_end(),
                        s[at..].trim_start()
                    )
                    .trim()
                    .into())
                } else {
                    Ok(replace(s, old, &replacement))
                }
            }
            "tablespace" => Ok(replace(
                s,
                pg_clause(s, "TABLESPACE")?,
                &if value.is_empty() {
                    String::new()
                } else {
                    format!("TABLESPACE {}", d.quote_ident(value))
                },
            )),
            "inherits" => {
                let names = pg_parents(value)?;
                let old = pg_clause(s, "INHERITS")?;
                let replacement = if names.is_empty() {
                    String::new()
                } else {
                    format!("INHERITS ({})", names.join(", "))
                };
                if old.is_none() {
                    Ok(format!("{replacement} {}", s.trim()).trim().into())
                } else {
                    Ok(replace(s, old, &replacement))
                }
            }
            _ => Err(DbError::msg("Unsupported PostgreSQL option.")),
        },
        Dialect::Sqlite => {
            let mut strict = table_option(s, "strict", d)? == "true";
            let mut without = table_option(s, "without_rowid", d)? == "true";
            let enabled = match value {
                "true" => true,
                "false" => false,
                _ => return Err(DbError::msg("Expected a boolean option.")),
            };
            match key {
                "strict" => strict = enabled,
                "without_rowid" => without = enabled,
                _ => return Err(DbError::msg("Unsupported SQLite option.")),
            };
            Ok([
                if without { "WITHOUT ROWID" } else { "" },
                if strict { "STRICT" } else { "" },
            ]
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(", "))
        }
    }
}
pub(super) fn pg_parents(s: &str) -> DbResult<Vec<String>> {
    split_field_list(s)?
        .into_iter()
        .map(|v| {
            let (head, rest) = parse::ident(&v)?;
            let d = Dialect::Postgres;
            if let Some(rest) = rest.strip_prefix('.') {
                let (name, tail) = parse::ident(rest)?;
                if !tail.is_empty() {
                    return Err(DbError::msg("Expected a referenced table name."));
                }
                Ok(format!("{}.{}", d.quote_ident(&head), d.quote_ident(&name)))
            } else if rest.is_empty() {
                if !v.starts_with('"') && head.contains('.') {
                    let pieces: Vec<_> = head.split('.').collect();
                    if pieces.len() != 2 {
                        return Err(DbError::msg("Expected schema.table."));
                    }
                    Ok(format!(
                        "{}.{}",
                        d.quote_ident(pieces[0]),
                        d.quote_ident(pieces[1])
                    ))
                } else {
                    Ok(d.quote_ident(&head))
                }
            } else {
                Err(DbError::msg("Expected a referenced table name."))
            }
        })
        .collect()
}

pub(super) fn option_alters(
    d: Dialect,
    table: &str,
    old: &str,
    new: &str,
) -> DbResult<Vec<String>> {
    if old == new {
        return Ok(Vec::new());
    }
    if d == Dialect::MySql {
        let charset_changed = table_option(old, "charset", d)? != table_option(new, "charset", d)?;
        let mut changes = Vec::new();
        let mut old_tail = old.to_string();
        let mut new_tail = new.to_string();
        for key in MYSQL_OPTIONS {
            let before = mysql_span(old, key)?;
            let after = mysql_span(new, key)?;
            if before.is_some() && after.is_none() {
                return Err(DbError::msg(
                    "Removing an explicit MySQL option does not reset the server value. Enter its replacement value.",
                ));
            }
            if (before.as_ref().map(|v| &v.2) != after.as_ref().map(|v| &v.2)
                || (*key == "collation" && charset_changed))
                && let Some((a, b, _)) = &after
            {
                changes.push(new[*a..*b].to_string());
            }
            old_tail = replace(&old_tail, mysql_span(&old_tail, key)?, "");
            new_tail = replace(&new_tail, mysql_span(&new_tail, key)?, "");
        }
        let normalize = |s: &str| -> DbResult<Vec<String>> {
            Ok(tokens(s)?.iter().map(|t| text(s, t).to_string()).collect())
        };
        if normalize(&old_tail)? != normalize(&new_tail)? {
            return Err(DbError::msg(
                "Changing unknown MySQL table options requires a dedicated migration.",
            ));
        }
        // Charset and collation must be validated together, not applied as two independent changes.
        return Ok(if changes.is_empty() {
            Vec::new()
        } else {
            vec![format!("ALTER TABLE {table} {}", changes.join(" "))]
        });
    }
    let mut old_tail = old.to_string();
    let mut new_tail = new.to_string();
    for key in ["WITH", "TABLESPACE", "INHERITS"] {
        old_tail = replace(&old_tail, pg_clause(&old_tail, key)?, "");
        new_tail = replace(&new_tail, pg_clause(&new_tail, key)?, "");
    }
    if old_tail.trim() != new_tail.trim() {
        return Err(DbError::msg(
            "Changing unknown PostgreSQL options is not supported.",
        ));
    }
    let mut out = Vec::new();
    let old_store = pg_storage(old)?;
    let new_store = pg_storage(new)?;
    for (key, value) in &new_store {
        if old_store.get(key) != Some(value) {
            out.push(format!("ALTER TABLE {table} SET ({key}={value})"));
        }
    }
    for key in old_store.keys().filter(|k| !new_store.contains_key(*k)) {
        out.push(format!("ALTER TABLE {table} RESET ({key})"));
    }
    let old_space = table_option(old, "tablespace", d)?;
    let new_space = table_option(new, "tablespace", d)?;
    if old_space != new_space {
        out.push(format!(
            "ALTER TABLE {table} SET TABLESPACE {}",
            d.quote_ident(if new_space.is_empty() {
                "pg_default"
            } else {
                &new_space
            })
        ));
    }
    let old_parents = pg_parents(&table_option(old, "inherits", d)?)?;
    let new_parents = pg_parents(&table_option(new, "inherits", d)?)?;
    let achievable = old_parents
        .iter()
        .filter(|p| new_parents.contains(p))
        .chain(new_parents.iter().filter(|p| !old_parents.contains(p)))
        .cloned()
        .collect::<Vec<_>>();
    if achievable != new_parents {
        return Err(DbError::msg(
            "Reordering inherited tables on an existing table is not supported; append new parents after retained ones.",
        ));
    }
    for p in old_parents.iter().filter(|p| !new_parents.contains(p)) {
        out.push(format!("ALTER TABLE {table} NO INHERIT {p}"));
    }
    for p in new_parents.iter().filter(|p| !old_parents.contains(p)) {
        out.push(format!("ALTER TABLE {table} INHERIT {p}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mysql_edits_preserve_unknown_options_and_do_not_replay_unchanged_values() {
        let d = Dialect::MySql;
        let old = "ENGINE=InnoDB AUTO_INCREMENT=101 DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin COMMENT='ENGINE=bad; a,b' UNION=(a,b)";
        assert_eq!(
            table_option(old, "table_comment", d).unwrap(),
            "ENGINE=bad; a,b"
        );
        let new = edit_table_option(old, "row_format", "DYNAMIC", d).unwrap();
        assert!(new.contains("UNION=(a,b)"));
        assert!(new.contains("COMMENT='ENGINE=bad; a,b'"));
        assert_eq!(
            option_alters(d, "`t`", old, &new).unwrap(),
            vec!["ALTER TABLE `t` ROW_FORMAT=DYNAMIC"]
        );
        let charset = edit_table_option(old, "charset", "latin1", d).unwrap();
        let statements = option_alters(d, "`t`", old, &charset).unwrap();
        assert_eq!(
            statements,
            vec!["ALTER TABLE `t` DEFAULT CHARACTER SET=latin1 COLLATE=utf8mb4_bin"]
        );
        let new = edit_table_option(old, "auto_increment", "", d).unwrap();
        assert!(option_alters(d, "`t`", old, &new).is_err());
        assert!(table_option("ENGINE=a ENGINE=b", "engine", d).is_err());
        assert!(edit_table_option(old, "max_rows", "2; DROP TABLE t", d).is_err());
        let comment = edit_table_option(old, "table_comment", "O'Brien \\ path", d).unwrap();
        assert_eq!(
            table_option(&comment, "table_comment", d).unwrap(),
            "O'Brien \\ path"
        );
    }
    #[test]
    fn postgres_preserves_storage_and_plans_explicit_resets_and_inheritance() {
        let d = Dialect::Postgres;
        let old = "INHERITS (\"public\".\"parent\") WITH (fillfactor=70, toast.autovacuum_enabled=false) TABLESPACE \"custom\"";
        let new = edit_table_option(old, "fillfactor", "90", d).unwrap();
        assert!(new.contains("toast.autovacuum_enabled=false"));
        assert_eq!(
            option_alters(d, "t", old, &new).unwrap(),
            vec!["ALTER TABLE t SET (fillfactor=90)"]
        );
        let reset = edit_table_option(&new, "fillfactor", "", d).unwrap();
        assert_eq!(
            option_alters(d, "t", &new, &reset).unwrap(),
            vec!["ALTER TABLE t RESET (fillfactor)"]
        );
        assert!(edit_table_option(old, "fillfactor", "9", d).is_err());
        let new = edit_table_option(
            "INHERITS (public.parent) TABLESPACE \"space\"",
            "fillfactor",
            "80",
            d,
        )
        .unwrap();
        assert!(new.starts_with("INHERITS (public.parent) WITH (fillfactor=80) TABLESPACE"));
        assert!(pg_parents("public.parent, \"schema.with.dot\".\"table space\"").is_ok());
        assert!(pg_parents("parent; DROP TABLE t").is_err());
        assert!(option_alters(d, "t", "INHERITS (a,b)", "INHERITS (b,a)").is_err());
    }
    #[test]
    fn sqlite_flags_round_trip_without_guessing_unknown_clauses() {
        let d = Dialect::Sqlite;
        let sql = edit_table_option("", "strict", "true", d).unwrap();
        let sql = edit_table_option(&sql, "without_rowid", "true", d).unwrap();
        assert_eq!(sql, "WITHOUT ROWID, STRICT");
        assert_eq!(table_option(&sql, "strict", d).unwrap(), "true");
        assert_eq!(
            edit_table_option(&sql, "strict", "false", d).unwrap(),
            "WITHOUT ROWID"
        );
        assert!(edit_table_option("UNKNOWN", "strict", "true", d).is_err());
    }
}
