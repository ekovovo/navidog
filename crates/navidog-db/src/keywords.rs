//! Keywords and built-in functions offered by code completion, per dialect (30-query-editor.md §4).

use crate::dialect::Dialect;

const COMMON: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "DATABASE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FOREIGN",
    "FROM",
    "FULL",
    "GRANT",
    "GROUP",
    "HAVING",
    "IF",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "PROCEDURE",
    "REFERENCES",
    "RENAME",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "SAVEPOINT",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

const MYSQL: &[&str] = &[
    "AUTO_INCREMENT",
    "CALL",
    "CHANGE",
    "CHARACTER",
    "CHARSET",
    "COLLATE",
    "DATABASES",
    "DELAYED",
    "DELIMITER",
    "DESCRIBE",
    "DUPLICATE",
    "ENGINE",
    "EVENT",
    "FORCE",
    "FUNCTION",
    "IGNORE",
    "INTERVAL",
    "KILL",
    "LOCK",
    "MODIFY",
    "PARTITION",
    "REGEXP",
    "REPLACE",
    "SCHEMA",
    "SHOW",
    "SPATIAL",
    "STRAIGHT_JOIN",
    "TABLES",
    "TEMPORARY",
    "UNLOCK",
    "UNSIGNED",
    "USE",
    "ZEROFILL",
];

const POSTGRES: &[&str] = &[
    "ANALYZE",
    "ARRAY",
    "BIGSERIAL",
    "CONFLICT",
    "COPY",
    "DO",
    "EXTENSION",
    "FILTER",
    "FUNCTION",
    "ILIKE",
    "LANGUAGE",
    "LATERAL",
    "MATERIALIZED",
    "NOTHING",
    "NULLS",
    "OVER",
    "PARTITION",
    "RETURNING",
    "RETURNS",
    "SCHEMA",
    "SEQUENCE",
    "SERIAL",
    "SIMILAR",
    "TABLESPACE",
    "TEMPORARY",
    "TYPE",
    "UNLOGGED",
    "VACUUM",
    "WINDOW",
];

const SQLITE: &[&str] = &[
    "ABORT",
    "ATTACH",
    "AUTOINCREMENT",
    "CONFLICT",
    "DETACH",
    "FAIL",
    "GLOB",
    "IGNORE",
    "INDEXED",
    "PRAGMA",
    "RAISE",
    "REINDEX",
    "REPLACE",
    "ROWID",
    "TEMP",
    "TEMPORARY",
    "VACUUM",
    "VIRTUAL",
    "WITHOUT",
];

const COMMON_FUNCTIONS: &[&str] = &[
    "ABS", "AVG", "COALESCE", "COUNT", "LENGTH", "LOWER", "LTRIM", "MAX", "MIN", "NULLIF",
    "REPLACE", "ROUND", "RTRIM", "SUBSTR", "SUM", "TRIM", "UPPER",
];

const MYSQL_FUNCTIONS: &[&str] = &[
    "CONCAT",
    "CONCAT_WS",
    "CONVERT",
    "CURDATE",
    "CURRENT_TIMESTAMP",
    "CURTIME",
    "DATE",
    "DATE_ADD",
    "DATE_FORMAT",
    "DATE_SUB",
    "DATEDIFF",
    "DAY",
    "FIND_IN_SET",
    "FLOOR",
    "FROM_UNIXTIME",
    "GREATEST",
    "GROUP_CONCAT",
    "HOUR",
    "IFNULL",
    "INSTR",
    "JSON_EXTRACT",
    "JSON_OBJECT",
    "LAST_INSERT_ID",
    "LEAST",
    "LOCATE",
    "LPAD",
    "MD5",
    "MINUTE",
    "MONTH",
    "NOW",
    "RAND",
    "RPAD",
    "SHA2",
    "SLEEP",
    "STR_TO_DATE",
    "SUBSTRING",
    "SUBSTRING_INDEX",
    "TIMESTAMPDIFF",
    "UNIX_TIMESTAMP",
    "UUID",
    "VERSION",
    "YEAR",
];

const POSTGRES_FUNCTIONS: &[&str] = &[
    "AGE",
    "ARRAY_AGG",
    "ARRAY_LENGTH",
    "CONCAT",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "DATE_PART",
    "DATE_TRUNC",
    "EXTRACT",
    "FORMAT",
    "GENERATE_SERIES",
    "GREATEST",
    "JSON_AGG",
    "JSONB_BUILD_OBJECT",
    "LEAST",
    "LEFT",
    "LPAD",
    "MD5",
    "NOW",
    "POSITION",
    "REGEXP_REPLACE",
    "RIGHT",
    "ROW_NUMBER",
    "RPAD",
    "SPLIT_PART",
    "STRING_AGG",
    "SUBSTRING",
    "TO_CHAR",
    "TO_DATE",
    "TO_NUMBER",
    "TO_TIMESTAMP",
    "UNNEST",
    "VERSION",
];

const SQLITE_FUNCTIONS: &[&str] = &[
    "CHANGES",
    "CHAR",
    "DATE",
    "DATETIME",
    "GLOB",
    "GROUP_CONCAT",
    "HEX",
    "IFNULL",
    "INSTR",
    "JSON",
    "JSON_EXTRACT",
    "JULIANDAY",
    "LAST_INSERT_ROWID",
    "LIKELIHOOD",
    "PRINTF",
    "QUOTE",
    "RANDOM",
    "SQLITE_VERSION",
    "STRFTIME",
    "TIME",
    "TOTAL",
    "TYPEOF",
    "UNICODE",
    "ZEROBLOB",
];

impl Dialect {
    /// Keywords for completion; the common SQL ones and the dialect's own.
    pub fn keywords(self) -> impl Iterator<Item = &'static str> {
        let own = match self {
            Self::MySql => MYSQL,
            Self::Postgres => POSTGRES,
            Self::Sqlite => SQLITE,
        };
        COMMON.iter().chain(own).copied()
    }

    pub fn functions(self) -> impl Iterator<Item = &'static str> {
        let own = match self {
            Self::MySql => MYSQL_FUNCTIONS,
            Self::Postgres => POSTGRES_FUNCTIONS,
            Self::Sqlite => SQLITE_FUNCTIONS,
        };
        COMMON_FUNCTIONS.iter().chain(own).copied()
    }

    pub fn is_keyword(self, word: &str) -> bool {
        self.keywords().any(|k| k.eq_ignore_ascii_case(word))
    }

    /// A name as it must be typed: quoted when it is not a plain identifier, is a keyword, or (in
    /// PostgreSQL) has capitals that unquoted would fold away.
    pub fn typed_name(self, name: &str) -> String {
        let plain = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let folds = self == Self::Postgres && name.chars().any(|c| c.is_ascii_uppercase());
        if plain && !folds && !self.is_keyword(name) {
            name.to_string()
        } else {
            self.quote_ident(name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_quoted_when_needed() {
        assert_eq!(Dialect::MySql.typed_name("actor"), "actor");
        assert_eq!(Dialect::MySql.typed_name("order"), "`order`");
        assert_eq!(Dialect::MySql.typed_name("my table"), "`my table`");
        assert_eq!(Dialect::Postgres.typed_name("Actor"), "\"Actor\"");
        assert_eq!(Dialect::Sqlite.typed_name("名称"), "\"名称\"");
    }
}
