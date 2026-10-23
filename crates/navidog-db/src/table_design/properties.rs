//! Structured edits replace only the requested SQL clause; unrecognized attributes survive.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnProperty {
    Type,
    Length,
    Scale,
    NotNull,
    Comment,
    Default,
    Collation,
    Charset,
    AutoIncrement,
    Unsigned,
    Generated,
    GeneratedExpression,
    GeneratedStorage,
    PrimaryKey,
    OnUpdate,
    Zerofill,
    Identity,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnProperties {
    pub data_type: String,
    pub length: String,
    pub scale: String,
    pub not_null: bool,
    pub primary_key: bool,
    pub generated: bool,
    pub generated_expression: String,
    pub generated_storage: String,
    pub comment: String,
    pub default: String,
    pub collation: String,
    pub charset: String,
    pub auto_increment: bool,
    pub unsigned: bool,
    pub zerofill: bool,
    pub on_update: String,
    pub identity: String,
}

fn words(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut depth, mut quote) = (0, 0, 0);
    let mut start = None;
    while i < b.len() {
        if quote != 0 {
            if b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == quote {
                if b.get(i + 1) == Some(&quote) {
                    i += 2;
                    continue;
                }
                quote = 0;
            }
        } else {
            match b[i] {
                b'\'' | b'"' | b'`' => quote = b[i],
                b'(' => depth += 1,
                b')' => depth -= 1,
                c if c.is_ascii_whitespace() && depth == 0 => {
                    if let Some(a) = start.take() {
                        out.push((a, i));
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        start.get_or_insert(i);
        i += 1;
    }
    if let Some(a) = start {
        out.push((a, s.len()));
    }
    out
}
fn clause(word: &str) -> bool {
    matches!(
        word.to_ascii_uppercase().as_str(),
        "NOT"
            | "NULL"
            | "DEFAULT"
            | "COMMENT"
            | "COLLATE"
            | "CHARACTER"
            | "AUTO_INCREMENT"
            | "AUTOINCREMENT"
            | "UNSIGNED"
            | "ZEROFILL"
            | "PRIMARY"
            | "UNIQUE"
            | "CHECK"
            | "REFERENCES"
            | "CONSTRAINT"
            | "GENERATED"
            | "AS"
            | "ON"
            | "IDENTITY"
            | "STORAGE"
            | "COLUMN_FORMAT"
            | "INVISIBLE"
            | "VISIBLE"
            | "BINARY"
            | "SRID"
            | "COMPRESSION"
    )
}
fn type_end(s: &str) -> usize {
    words(s)
        .into_iter()
        .skip(1)
        .find(|&(a, b)| clause(&s[a..b]))
        .map(|(a, _)| a)
        .unwrap_or(s.len())
}
fn span(s: &str, key: &str, values: usize) -> Option<(usize, usize)> {
    let w = words(s);
    let keys: Vec<_> = key.split_whitespace().collect();
    for i in 0..w.len() {
        if keys.iter().enumerate().all(|(j, k)| {
            w.get(i + j)
                .is_some_and(|&(a, b)| s[a..b].eq_ignore_ascii_case(k))
        }) {
            if key == "DEFAULT" && values == 1 {
                let first = i + 1;
                if first >= w.len() {
                    return None;
                }
                let end = (first + 1..w.len())
                    .find(|&j| {
                        clause(&s[w[j].0..w[j].1])
                            && !s[w[j].0..w[j].1].eq_ignore_ascii_case("NULL")
                    })
                    .map(|j| w[j].0)
                    .unwrap_or(s.len());
                return Some((w[i].0, s[..end].trim_end().len()));
            }
            let last = i + keys.len() + values - 1;
            if last < w.len() {
                return Some((w[i].0, w[last].1));
            }
        }
    }
    None
}
fn value(s: &str, key: &str) -> String {
    span(s, key, 1)
        .map(|(a, b)| s[a + key.len()..b].trim().to_string())
        .unwrap_or_default()
}
fn literal(s: &str) -> String {
    if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
        s[1..s.len() - 1]
            .replace("''", "'")
            .replace("\\'", "'")
            .replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}
fn generated(s: &str) -> Option<(usize, usize, String, String)> {
    let w = words(s);
    let i = w
        .iter()
        .position(|&(a, b)| s[a..b].eq_ignore_ascii_case("AS"))?;
    let &(a, b) = w.get(i + 1)?;
    let expr = s[a..b].strip_prefix('(')?.strip_suffix(')')?.to_string();
    let start = if i >= 2 && s[w[i - 2].0..w[i - 2].1].eq_ignore_ascii_case("GENERATED") {
        w[i - 2].0
    } else {
        w[i].0
    };
    let (end, storage) = if let Some(&(a, b)) = w.get(i + 2)
        && ["STORED", "VIRTUAL"]
            .iter()
            .any(|v| s[a..b].eq_ignore_ascii_case(v))
    {
        (b, s[a..b].to_string())
    } else {
        (b, String::new())
    };
    Some((start, end, expr, storage))
}
pub(super) fn identity(s: &str) -> Option<(usize, usize, String, String)> {
    for (key, mode) in [
        ("GENERATED ALWAYS AS IDENTITY", "ALWAYS"),
        ("GENERATED BY DEFAULT AS IDENTITY", "BY DEFAULT"),
    ] {
        if let Some((a, b)) = span(s, key, 0) {
            let rest = s[b..].trim_start();
            let options = if rest.starts_with('(') {
                words(rest)
                    .first()
                    .map(|(a, b)| rest[*a..*b].to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let end = if options.is_empty() {
                b
            } else {
                s.len() - rest.len() + options.len()
            };
            return Some((a, end, mode.into(), options));
        }
    }
    None
}
pub fn column_properties(s: &str) -> ColumnProperties {
    let ty = s[..type_end(s)].trim();
    let (data_type, length, scale) = if let Some(a) = ty.find('(') {
        let b = ty.rfind(')').unwrap_or(ty.len());
        let args = &ty[a + 1..b];
        // Enum/set values are a single editable length value, never split at literal commas.
        let split = parse::positions(args, b',').unwrap_or_default();
        let (length, scale) = if split.len() == 1 && !args.contains('\'') {
            (&args[..split[0]], &args[split[0] + 1..])
        } else {
            (args, "")
        };
        (
            format!("{}{}", ty[..a].trim(), ty.get(b + 1..).unwrap_or_default()),
            length.trim().to_string(),
            scale.trim().to_string(),
        )
    } else {
        (ty.to_string(), String::new(), String::new())
    };
    ColumnProperties {
        data_type,
        length,
        scale,
        not_null: span(s, "NOT NULL", 0).is_some(),
        primary_key: span(s, "PRIMARY KEY", 0).is_some(),
        generated: generated(s).is_some(),
        generated_expression: generated(s).map(|g| g.2).unwrap_or_default(),
        generated_storage: generated(s).map(|g| g.3).unwrap_or_default(),
        comment: literal(&value(s, "COMMENT")),
        default: if identity(s).is_some() {
            String::new()
        } else {
            value(s, "DEFAULT")
        },
        collation: value(s, "COLLATE"),
        charset: value(s, "CHARACTER SET"),
        auto_increment: span(s, "AUTO_INCREMENT", 0).is_some()
            || span(s, "AUTOINCREMENT", 0).is_some(),
        unsigned: span(s, "UNSIGNED", 0).is_some() || span(s, "ZEROFILL", 0).is_some(),
        zerofill: span(s, "ZEROFILL", 0).is_some(),
        on_update: value(s, "ON UPDATE"),
        identity: identity(s).map(|v| v.2).unwrap_or_default(),
    }
}

pub fn edit_column(
    s: &str,
    property: ColumnProperty,
    input: &str,
    dialect: Dialect,
) -> DbResult<String> {
    if property != ColumnProperty::Comment {
        parse::fragment(input)?;
    }
    let mut p = column_properties(s);
    if property == ColumnProperty::Identity {
        if dialect != Dialect::Postgres || !["", "ALWAYS", "BY DEFAULT"].contains(&input) {
            return Err(DbError::msg(
                "Identity mode must be ALWAYS, BY DEFAULT, or empty on PostgreSQL.",
            ));
        }
        if generated(s).is_some() || (!input.is_empty() && !p.default.is_empty()) {
            return Err(DbError::msg(
                "Remove the default or generated expression before adding identity.",
            ));
        }
        let old = identity(s);
        let replacement = if input.is_empty() {
            String::new()
        } else {
            format!(
                "GENERATED {input} AS IDENTITY {}",
                old.as_ref().map(|v| v.3.as_str()).unwrap_or("")
            )
            .trim()
            .to_string()
        };
        let mut result = s.to_string();
        if let Some((a, b, _, _)) = old {
            result.replace_range(a..b, &replacement);
        } else if !replacement.is_empty() {
            result.push(' ');
            result.push_str(&replacement);
        }
        return Ok(result.trim().into());
    }
    if matches!(
        property,
        ColumnProperty::Type | ColumnProperty::Length | ColumnProperty::Scale
    ) {
        match property {
            ColumnProperty::Type => {
                p.data_type = input.trim().into();
                if [
                    "TEXT",
                    "BLOB",
                    "BYTEA",
                    "JSON",
                    "JSONB",
                    "BOOLEAN",
                    "DATE",
                    "INTEGER",
                    "BIGINT",
                    "SMALLINT",
                    "REAL",
                    "DOUBLE PRECISION",
                ]
                .iter()
                .any(|t| p.data_type.eq_ignore_ascii_case(t))
                {
                    p.length.clear();
                    p.scale.clear();
                }
            }
            ColumnProperty::Length => p.length = input.trim().into(),
            _ => p.scale = input.trim().into(),
        }
        if p.data_type.is_empty() {
            return Err(DbError::msg("A field needs a type."));
        }
        let args = if p.length.is_empty() {
            String::new()
        } else if p.scale.is_empty() {
            format!("({})", p.length)
        } else {
            format!("({},{})", p.length, p.scale)
        };
        let original_type = s[..type_end(s)].trim();
        let ty = if property != ColumnProperty::Type
            && let Some(a) = original_type.find('(')
            && let Some(b) = original_type.rfind(')')
        {
            // A length edit changes only the arguments, preserving array and timezone suffixes.
            format!("{}{args}{}", &original_type[..a], &original_type[b + 1..])
        } else {
            let upper = p.data_type.to_ascii_uppercase();
            let at = [
                upper.find(" WITH TIME ZONE"),
                upper.find(" WITHOUT TIME ZONE"),
                upper.find('['),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(p.data_type.len());
            format!("{}{args}{}", &p.data_type[..at], &p.data_type[at..])
        };
        return Ok(format!("{ty} {}", s[type_end(s)..].trim())
            .trim()
            .to_string());
    }
    if matches!(
        property,
        ColumnProperty::Generated
            | ColumnProperty::GeneratedExpression
            | ColumnProperty::GeneratedStorage
    ) {
        let old = generated(s);
        if old.is_none() && span(s, "GENERATED", 0).is_some() {
            return Err(DbError::msg(
                "Identity attributes cannot be changed as a generated expression.",
            ));
        }
        let remove = property == ColumnProperty::Generated && input == "false";
        let expression = if property == ColumnProperty::GeneratedExpression {
            input.to_string()
        } else {
            old.as_ref()
                .map(|g| g.2.clone())
                .unwrap_or_else(|| "0".into())
        };
        let storage = if property == ColumnProperty::GeneratedStorage {
            input.to_ascii_uppercase()
        } else {
            old.as_ref()
                .map(|g| g.3.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if dialect == Dialect::Postgres {
                        "STORED"
                    } else {
                        "VIRTUAL"
                    }
                    .into()
                })
        };
        if !["STORED", "VIRTUAL"].contains(&storage.as_str()) {
            return Err(DbError::msg("Generated storage must be STORED or VIRTUAL."));
        }
        let replacement = if remove {
            String::new()
        } else {
            format!("GENERATED ALWAYS AS ({expression}) {storage}")
        };
        let mut result = s.to_string();
        if let Some((a, b, _, _)) = old {
            result.replace_range(a..b, &replacement);
        } else if !remove {
            result.push(' ');
            result.push_str(&replacement);
        }
        return Ok(result.trim().to_string());
    }
    let (key, count, replacement) = match property {
        ColumnProperty::PrimaryKey => (
            "PRIMARY KEY",
            0,
            if input == "true" {
                "PRIMARY KEY".into()
            } else {
                String::new()
            },
        ),
        ColumnProperty::NotNull => (
            "NOT NULL",
            0,
            if input == "true" {
                "NOT NULL".into()
            } else {
                String::new()
            },
        ),
        ColumnProperty::Comment => {
            if dialect != Dialect::MySql {
                return Err(DbError::msg(
                    "Column comments are edited through the database comment definition.",
                ));
            }
            (
                "COMMENT",
                1,
                if input.is_empty() {
                    String::new()
                } else {
                    format!(
                        "COMMENT '{}'",
                        input.replace('\\', "\\\\").replace('\'', "''")
                    )
                },
            )
        }
        ColumnProperty::Default => (
            "DEFAULT",
            1,
            if input.trim().is_empty() {
                String::new()
            } else {
                format!("DEFAULT {input}")
            },
        ),
        ColumnProperty::Collation => (
            "COLLATE",
            1,
            if input.trim().is_empty() {
                String::new()
            } else {
                format!("COLLATE {input}")
            },
        ),
        ColumnProperty::Charset => (
            "CHARACTER SET",
            1,
            if input.trim().is_empty() {
                String::new()
            } else {
                format!("CHARACTER SET {input}")
            },
        ),
        ColumnProperty::AutoIncrement => {
            let key = if dialect == Dialect::Sqlite {
                "AUTOINCREMENT"
            } else {
                "AUTO_INCREMENT"
            };
            (
                key,
                0,
                if input == "true" {
                    key.into()
                } else {
                    String::new()
                },
            )
        }
        ColumnProperty::OnUpdate => {
            if dialect != Dialect::MySql {
                return Err(DbError::msg("ON UPDATE is a MySQL field attribute."));
            }
            (
                "ON UPDATE",
                1,
                if input.trim().is_empty() {
                    String::new()
                } else {
                    format!("ON UPDATE {input}")
                },
            )
        }
        ColumnProperty::Zerofill => {
            if dialect != Dialect::MySql {
                return Err(DbError::msg("ZEROFILL is a MySQL field attribute."));
            }
            (
                "ZEROFILL",
                0,
                if input == "true" {
                    "ZEROFILL".into()
                } else {
                    String::new()
                },
            )
        }
        ColumnProperty::Unsigned => (
            "UNSIGNED",
            0,
            if input == "true" {
                "UNSIGNED".into()
            } else {
                String::new()
            },
        ),
        _ => unreachable!(),
    };
    let mut out = s.to_string();
    if let Some((a, b)) = span(s, key, count) {
        out.replace_range(a..b, &replacement);
    } else if !replacement.is_empty() {
        if matches!(
            property,
            ColumnProperty::Unsigned
                | ColumnProperty::Zerofill
                | ColumnProperty::Charset
                | ColumnProperty::Collation
        ) {
            let end = match property {
                ColumnProperty::Zerofill => span(s, "UNSIGNED", 0)
                    .map(|v| v.1)
                    .unwrap_or_else(|| type_end(s)),
                ColumnProperty::Collation => span(s, "CHARACTER SET", 1)
                    .map(|v| v.1)
                    .unwrap_or_else(|| type_end(s)),
                _ => type_end(s),
            };
            out = format!("{} {replacement} {}", s[..end].trim(), s[end..].trim());
        } else if property == ColumnProperty::NotNull {
            let end = type_end(s);
            let tail = s[end..].trim();
            let tail = if tail.eq_ignore_ascii_case("NULL") {
                ""
            } else if tail
                .get(..5)
                .is_some_and(|v| v.eq_ignore_ascii_case("NULL "))
            {
                &tail[5..]
            } else {
                tail
            };
            out = format!("{} {replacement} {}", s[..end].trim(), tail);
        } else {
            out.push(' ');
            out.push_str(&replacement);
        }
    }
    if property == ColumnProperty::Zerofill && input == "true" {
        return edit_column(out.trim(), ColumnProperty::Unsigned, "true", dialect);
    }
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn postgres_default_cast_and_generated_clauses_are_not_truncated() {
        assert_eq!(
            column_properties("timestamp(6) with time zone NOT NULL").data_type,
            "timestamp with time zone"
        );
        assert_eq!(
            edit_column(
                "timestamp(6) with time zone NOT NULL",
                ColumnProperty::Length,
                "3",
                Dialect::Postgres
            )
            .unwrap(),
            "timestamp(3) with time zone NOT NULL"
        );
        assert_eq!(
            edit_column(
                "character varying(80)[] DEFAULT '{}'::character varying[]",
                ColumnProperty::Length,
                "100",
                Dialect::Postgres
            )
            .unwrap(),
            "character varying(100)[] DEFAULT '{}'::character varying[]"
        );
        let s = "character varying(80) NOT NULL DEFAULT 'before'::character varying";
        let edited = edit_column(s, ColumnProperty::Default, "'after'", Dialect::Postgres).unwrap();
        assert_eq!(edited, "character varying(80) NOT NULL DEFAULT 'after'");
        assert_eq!(
            edit_column(
                "TEXT DEFAULT 'a'",
                ColumnProperty::NotNull,
                "true",
                Dialect::Postgres
            )
            .unwrap(),
            "TEXT NOT NULL DEFAULT 'a'"
        );
        let s = "INTEGER GENERATED ALWAYS AS (length('a b')) STORED NOT NULL";
        let p = column_properties(s);
        assert!(p.generated);
        assert_eq!(p.generated_expression, "length('a b')");
        assert_eq!(p.generated_storage, "STORED");
        let edited = edit_column(
            s,
            ColumnProperty::GeneratedExpression,
            "id + 1",
            Dialect::Sqlite,
        )
        .unwrap();
        assert_eq!(
            edited,
            "INTEGER GENERATED ALWAYS AS (id + 1) STORED NOT NULL"
        );
    }
    #[test]
    fn structured_edits_preserve_unrelated_attributes_and_literals() {
        let s = "DECIMAL(10,2) UNSIGNED NOT NULL DEFAULT (round(1.5, 2)) COMMENT 'NOT NULL DEFAULT x' INVISIBLE";
        let p = column_properties(s);
        assert_eq!(p.length, "10");
        assert_eq!(p.scale, "2");
        assert_eq!(p.default, "(round(1.5, 2))");
        let edited = edit_column(s, ColumnProperty::Length, "20", Dialect::MySql).unwrap();
        assert!(edited.starts_with("DECIMAL(20,2)"));
        assert!(edited.ends_with("INVISIBLE"));
        let edited =
            edit_column(&edited, ColumnProperty::NotNull, "false", Dialect::MySql).unwrap();
        assert!(!column_properties(&edited).not_null);
        assert!(edited.contains("'NOT NULL DEFAULT x'"));
        let edited =
            edit_column(&edited, ColumnProperty::Comment, "O'Brien", Dialect::MySql).unwrap();
        assert_eq!(column_properties(&edited).comment, "O'Brien");
        assert!(
            edit_column(
                s,
                ColumnProperty::Default,
                "0; DROP TABLE x",
                Dialect::MySql
            )
            .is_err()
        );
        assert_eq!(
            column_properties("ENUM('a,b','c') NOT NULL").length,
            "'a,b','c'"
        );
    }
}

#[cfg(test)]
mod dialect_tests {
    use super::*;
    #[test]
    fn identity_modes_preserve_sequence_options_and_mysql_attributes_are_independent() {
        let old = "BIGINT GENERATED ALWAYS AS IDENTITY (START WITH 20 INCREMENT BY 3) NOT NULL";
        assert_eq!(column_properties(old).identity, "ALWAYS");
        assert!(column_properties(old).default.is_empty());
        let new = edit_column(
            old,
            ColumnProperty::Identity,
            "BY DEFAULT",
            Dialect::Postgres,
        )
        .unwrap();
        assert_eq!(
            new,
            "BIGINT GENERATED BY DEFAULT AS IDENTITY (START WITH 20 INCREMENT BY 3) NOT NULL"
        );
        let new = edit_column(
            "TIMESTAMP(3) DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3)",
            ColumnProperty::OnUpdate,
            "CURRENT_TIMESTAMP(2)",
            Dialect::MySql,
        )
        .unwrap();
        assert_eq!(column_properties(&new).on_update, "CURRENT_TIMESTAMP(2)");
        assert!(new.contains("DEFAULT CURRENT_TIMESTAMP(3)"));
        let new = edit_column(
            "INT UNSIGNED",
            ColumnProperty::Zerofill,
            "true",
            Dialect::MySql,
        )
        .unwrap();
        assert!(column_properties(&new).zerofill);
        assert!(column_properties(&new).unsigned);
        assert!(
            edit_column(
                "INT NOT NULL",
                ColumnProperty::Zerofill,
                "true",
                Dialect::MySql
            )
            .unwrap()
            .starts_with("INT UNSIGNED ZEROFILL NOT NULL")
        );
        assert!(edit_column("INT", ColumnProperty::Zerofill, "true", Dialect::Postgres).is_err());
    }
}
