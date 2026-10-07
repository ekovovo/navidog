//! Interpret a SQL type name. Icons and colors stay in the feature that paints them.

/// Which family a column type belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Number,
    Text,
    Time,
    Bool,
    Binary,
    Json,
    Other,
}

/// `varchar(255)`, `bigint unsigned`, `integer[]` and `MYSQL_TYPE_LONG` all classify by the base name.
pub fn type_kind(type_name: &str) -> TypeKind {
    let t = type_name.trim().to_ascii_lowercase();
    let base = t
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_end_matches("[]");
    const JSON: &[&str] = &["json", "jsonb", "mysql_type_json"];
    const TIME: &[&str] = &[
        "date",
        "time",
        "timetz",
        "datetime",
        "datetime2",
        "smalldatetime",
        "timestamp",
        "timestamptz",
        "interval",
        "mysql_type_timestamp",
        "mysql_type_timestamp2",
        "mysql_type_datetime",
        "mysql_type_datetime2",
        "mysql_type_date",
        "mysql_type_newdate",
        "mysql_type_time",
        "mysql_type_time2",
    ];
    const BOOL: &[&str] = &["bool", "boolean"];
    const BINARY: &[&str] = &[
        "blob",
        "tinyblob",
        "mediumblob",
        "longblob",
        "binary",
        "varbinary",
        "bytea",
        "image",
        "bit",
        "varbit",
        "raw",
        "mysql_type_blob",
        "mysql_type_tiny_blob",
        "mysql_type_medium_blob",
        "mysql_type_long_blob",
        "mysql_type_bit",
    ];
    const NUMBER: &[&str] = &[
        "int",
        "integer",
        "tinyint",
        "smallint",
        "mediumint",
        "bigint",
        "int2",
        "int4",
        "int8",
        "serial",
        "smallserial",
        "bigserial",
        "decimal",
        "numeric",
        "number",
        "float",
        "float4",
        "float8",
        "double",
        "real",
        "money",
        "smallmoney",
        "year",
        "oid",
        "dec",
        "fixed",
        "mysql_type_decimal",
        "mysql_type_tiny",
        "mysql_type_short",
        "mysql_type_long",
        "mysql_type_float",
        "mysql_type_double",
        "mysql_type_longlong",
        "mysql_type_int24",
        "mysql_type_year",
        "mysql_type_newdecimal",
    ];
    const TEXT: &[&str] = &[
        "char",
        "varchar",
        "nchar",
        "nvarchar",
        "character",
        "bpchar",
        "text",
        "tinytext",
        "mediumtext",
        "longtext",
        "ntext",
        "clob",
        "citext",
        "string",
        "enum",
        "set",
        "uuid",
        "name",
        "xml",
        "mysql_type_varchar",
        "mysql_type_var_string",
        "mysql_type_string",
        "mysql_type_enum",
        "mysql_type_set",
    ];
    if JSON.contains(&base) {
        TypeKind::Json
    } else if TIME.contains(&base) {
        TypeKind::Time
    } else if BOOL.contains(&base) {
        TypeKind::Bool
    } else if BINARY.contains(&base) {
        TypeKind::Binary
    } else if NUMBER.contains(&base) {
        TypeKind::Number
    } else if TEXT.contains(&base) {
        TypeKind::Text
    } else {
        TypeKind::Other
    }
}

/// Character types are quoted in raw data mode. Accepts SQL names (`varchar(45)`,
/// `character varying`) and the MySQL protocol names the driver puts on a result column.
pub fn is_string_type(type_name: &str) -> bool {
    let t = type_name.to_ascii_lowercase();
    if t.contains("mysql_type_string")
        || t.contains("mysql_type_var_string")
        || t.contains("mysql_type_varchar")
        || t.contains("mysql_type_enum")
        || t.contains("mysql_type_set")
        || t.contains("mysql_type_json")
    {
        return true;
    }
    let base = t
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or("");
    matches!(
        base,
        "char"
            | "varchar"
            | "nchar"
            | "nvarchar"
            | "character"
            | "bpchar"
            | "text"
            | "tinytext"
            | "mediumtext"
            | "longtext"
            | "ntext"
            | "clob"
            | "citext"
            | "string"
            | "enum"
            | "set"
            | "json"
            | "jsonb"
            | "xml"
            | "uuid"
            | "name"
    )
}

/// Date, time, or timestamp. `None` for every other type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Temporal {
    Date,
    Time,
    DateTime,
}

/// Column types that open a date picker. `None` keeps a plain text editor.
pub fn temporal_kind(type_name: &str) -> Option<Temporal> {
    let t = type_name.to_ascii_lowercase();
    if t.contains("mysql_type_datetime") || t.contains("mysql_type_timestamp") {
        return Some(Temporal::DateTime);
    }
    if t.contains("mysql_type_date") {
        return Some(Temporal::Date);
    }
    if t.contains("mysql_type_time") {
        return Some(Temporal::Time);
    }
    let base = t
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or("");
    match base {
        "datetime" | "timestamp" | "timestamptz" | "datetime2" | "smalldatetime" => {
            Some(Temporal::DateTime)
        }
        "date" => Some(Temporal::Date),
        "time" | "timetz" => Some(Temporal::Time),
        _ => None,
    }
}

/// Values listed on a MySQL `enum` or `set` type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Choices {
    /// MySQL `enum`: one value.
    One(Vec<String>),
    /// MySQL `set`: a comma-separated subset.
    Many(Vec<String>),
}

/// Parses `enum(...)` / `set(...)` only when the type string itself lists the values.
pub fn choices(type_name: &str) -> Option<Choices> {
    let t = type_name.trim();
    let (many, rest) = if let Some(rest) = strip_kind(t, "set") {
        (true, rest)
    } else {
        let rest = strip_kind(t, "enum")?;
        (false, rest)
    };
    let values = parse_sql_list(rest)?;
    if values.is_empty() {
        return None;
    }
    Some(if many {
        Choices::Many(values)
    } else {
        Choices::One(values)
    })
}

fn strip_kind<'a>(type_name: &'a str, kind: &str) -> Option<&'a str> {
    if type_name.len() < kind.len() || !type_name[..kind.len()].eq_ignore_ascii_case(kind) {
        return None;
    }
    let rest = type_name[kind.len()..].trim_start();
    let rest = rest.strip_prefix('(')?.trim_end();
    rest.strip_suffix(')')
}

fn parse_sql_list(s: &str) -> Option<Vec<String>> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'\'' {
            return None;
        }
        i += 1;
        let mut value = String::new();
        let mut closed = false;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    value.push('\'');
                    i += 2;
                    continue;
                }
                i += 1;
                closed = true;
                break;
            }
            let ch = s[i..].chars().next()?;
            value.push(ch);
            i += ch.len_utf8();
        }
        if !closed {
            return None;
        }
        out.push(value);
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() {
            if bytes[i] != b',' {
                return None;
            }
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_base_name() {
        assert_eq!(type_kind("integer"), TypeKind::Number);
        assert_eq!(type_kind("bigint unsigned"), TypeKind::Number);
        assert_eq!(type_kind("numeric(10,2)"), TypeKind::Number);
        assert_eq!(type_kind("MYSQL_TYPE_LONG"), TypeKind::Number);
        assert_eq!(type_kind("varchar(255)"), TypeKind::Text);
        assert_eq!(type_kind("character varying(45)"), TypeKind::Text);
        assert_eq!(type_kind("MYSQL_TYPE_VAR_STRING"), TypeKind::Text);
        assert_eq!(type_kind("timestamp without time zone"), TypeKind::Time);
        assert_eq!(type_kind("boolean"), TypeKind::Bool);
        assert_eq!(type_kind("bytea"), TypeKind::Binary);
        assert_eq!(type_kind("jsonb"), TypeKind::Json);
        assert_eq!(type_kind("integer[]"), TypeKind::Number);
        assert_eq!(type_kind("geometry"), TypeKind::Other);
    }

    #[test]
    fn string_types_are_the_ones_raw_mode_quotes() {
        assert!(is_string_type("varchar(45)"));
        assert!(is_string_type("character varying(20)"));
        assert!(is_string_type("TEXT"));
        assert!(is_string_type("MYSQL_TYPE_VAR_STRING"));
        assert!(!is_string_type("int"));
        assert!(!is_string_type("timestamp without time zone"));
        assert!(!is_string_type("MYSQL_TYPE_LONG"));
    }

    #[test]
    fn temporal_kinds_follow_the_base_name() {
        assert_eq!(temporal_kind("varchar(45)"), None);
        assert_eq!(temporal_kind("date"), Some(Temporal::Date));
        assert_eq!(temporal_kind("time"), Some(Temporal::Time));
        assert_eq!(
            temporal_kind("timestamp without time zone"),
            Some(Temporal::DateTime)
        );
        assert_eq!(
            temporal_kind("MYSQL_TYPE_DATETIME"),
            Some(Temporal::DateTime)
        );
    }

    #[test]
    fn enum_and_set_lists_come_from_the_type_string() {
        assert_eq!(
            choices("enum('a','b','it''s')"),
            Some(Choices::One(vec!["a".into(), "b".into(), "it's".into()]))
        );
        assert_eq!(
            choices("set('red','blue')"),
            Some(Choices::Many(vec!["red".into(), "blue".into()]))
        );
        assert!(choices("varchar(20)").is_none());
        assert!(choices("enum").is_none());
        assert!(choices("ENUM('x', y)").is_none());
    }
}
