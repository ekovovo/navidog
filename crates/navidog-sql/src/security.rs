//! Users, grants, maintenance, server monitor, find, and dependency SQL.
//! Product differences are a [`DefinitionFamily`] match here. Callers ask the [`Dialect`] methods.

use std::collections::{BTreeMap, BTreeSet};

use navidog_model::object::{ObjectKind, ObjectRef, Scope};

use crate::definition::DefinitionFamily;
use crate::dialect::Dialect;

/// Shown when SQLite user authentication cannot run. This build does not include the extension.
pub const SQLITE_AUTH_REASON: &str =
    "SQLite user authentication needs the user-auth extension, which this build does not include.";

/// Which account form to show. The panel matches this, not a product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountForm {
    Mysql,
    Postgres,
    /// No statements. The text is the reason, and nothing is executed.
    Unavailable(&'static str),
}

/// A user, role, or group draft. Unused fields are ignored for the other products.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDraft {
    pub name: String,
    pub host: String,
    pub previous_name: Option<String>,
    pub previous_host: Option<String>,
    pub password: String,
    /// MySQL role (no host) rather than a user.
    pub role: bool,
    pub locked: bool,
    /// Empty, `never`, or a number of days.
    pub expire: String,
    pub max_queries: String,
    pub max_updates: String,
    pub max_connections: String,
    pub max_user_connections: String,
    /// PostgreSQL. `false` is a group (`NOLOGIN`).
    pub login: bool,
    pub superuser: bool,
    pub createdb: bool,
    pub createrole: bool,
    pub inherit: bool,
    pub replication: bool,
    pub connection_limit: String,
    pub valid_until: String,
    /// Authentication plugin. Empty keeps the server default.
    pub plugin: String,
    /// `NONE`, `ANY`, `X509`, `SPECIFIED`, or empty to leave SSL alone.
    pub ssl_type: String,
    pub ssl_cipher: String,
    pub ssl_issuer: String,
    pub ssl_subject: String,
    /// Set when the account was loaded. `None` is a new account.
    pub previous_plugin: Option<String>,
    pub previous_ssl: Option<String>,
    pub previous_cipher: Option<String>,
    pub previous_issuer: Option<String>,
    pub previous_subject: Option<String>,
    pub previous_locked: Option<bool>,
}

impl Default for AccountDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            previous_name: None,
            previous_host: None,
            password: String::new(),
            role: false,
            locked: false,
            expire: String::new(),
            max_queries: String::new(),
            max_updates: String::new(),
            max_connections: String::new(),
            max_user_connections: String::new(),
            login: true,
            superuser: false,
            createdb: false,
            createrole: false,
            inherit: true,
            replication: false,
            connection_limit: String::new(),
            valid_until: String::new(),
            plugin: String::new(),
            ssl_type: String::new(),
            ssl_cipher: String::new(),
            ssl_issuer: String::new(),
            ssl_subject: String::new(),
            previous_plugin: None,
            previous_ssl: None,
            previous_cipher: None,
            previous_issuer: None,
            previous_subject: None,
            previous_locked: None,
        }
    }
}

/// Global privileges shown on the server page, in Navicat's order.
/// `GRANT OPTION` is the `WITH GRANT OPTION` clause, not a name in the privilege list.
pub const SERVER_PRIVILEGES: &[&str] = &[
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROUTINE",
    "CREATE TEMPORARY TABLES",
    "CREATE USER",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "EVENT",
    "EXECUTE",
    "FILE",
    "GRANT OPTION",
    "INDEX",
    "INSERT",
    "LOCK TABLES",
    "PROCESS",
    "REFERENCES",
    "RELOAD",
    "REPLICATION CLIENT",
    "REPLICATION SLAVE",
    "SELECT",
    "SHOW DATABASES",
    "SHOW VIEW",
    "SHUTDOWN",
    "SUPER",
    "SYSTEM_USER",
    "TRIGGER",
    "UPDATE",
];

/// Privileges that apply to a database object. Same `GRANT OPTION` rule as the server list.
pub const OBJECT_PRIVILEGES: &[&str] = &[
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROUTINE",
    "CREATE TEMPORARY TABLES",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "EVENT",
    "EXECUTE",
    "GRANT OPTION",
    "INDEX",
    "INSERT",
    "LOCK TABLES",
    "REFERENCES",
    "SELECT",
    "SHOW VIEW",
    "TRIGGER",
    "UPDATE",
];

/// What `SHOW GRANTS` said, plus default roles the designer loaded separately.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrantSet {
    pub server: BTreeSet<String>,
    pub server_grant_option: bool,
    pub roles: BTreeSet<(String, String)>,
    pub defaults: BTreeSet<(String, String)>,
    pub objects: BTreeMap<(String, String), ObjectGrant>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectGrant {
    pub privileges: BTreeSet<String>,
    pub grant_option: bool,
}

#[derive(Clone, Debug)]
pub struct GrantDraft {
    pub privilege: String,
    pub object: String,
    pub grantee: String,
    pub host: String,
    pub grant_option: bool,
    pub revoke: bool,
}

impl Default for GrantDraft {
    fn default() -> Self {
        Self {
            privilege: "SELECT".into(),
            object: String::new(),
            grantee: String::new(),
            host: String::new(),
            grant_option: false,
            revoke: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceJob {
    pub key: &'static str,
    pub sql: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    Contains,
    WholeWord,
    Prefix,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindPlan {
    pub structure: Vec<String>,
    /// Columns to probe when data search is on. `None` when it is off.
    pub text_columns: Option<String>,
    pub needle: String,
    pub mode: MatchMode,
    pub case_sensitive: bool,
}

pub struct FindRequest<'a> {
    pub needle: &'a str,
    pub mode: MatchMode,
    pub case_sensitive: bool,
    pub kinds: &'a [ObjectKind],
    pub data: bool,
}

pub struct Probe<'a> {
    pub table: &'a str,
    pub columns: &'a [String],
    pub needle: &'a str,
    pub mode: MatchMode,
    pub case_sensitive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Uses,
    UsedBy,
    Members,
    MemberOf,
    Privileges,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedSql {
    pub section: Section,
    pub sql: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationPlan {
    /// When set, show this and do not run SQL.
    pub note: Option<&'static str>,
    pub queries: Vec<NamedSql>,
}

pub fn account_form(family: Option<DefinitionFamily>) -> AccountForm {
    match family {
        Some(DefinitionFamily::Mysql) => AccountForm::Mysql,
        Some(DefinitionFamily::Postgres) => AccountForm::Postgres,
        Some(DefinitionFamily::Sqlite) => AccountForm::Unavailable(SQLITE_AUTH_REASON),
        None => AccountForm::Unavailable("This connection cannot manage users."),
    }
}

pub fn account_statements(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    draft: &AccountDraft,
) -> Result<Vec<String>, String> {
    match family {
        Some(DefinitionFamily::Mysql) => mysql_account(dialect, draft),
        Some(DefinitionFamily::Postgres) => postgres_account(dialect, draft),
        Some(DefinitionFamily::Sqlite) => Err(SQLITE_AUTH_REASON.into()),
        None => Err("This connection cannot manage users.".into()),
    }
}

pub fn account_list_sql(family: Option<DefinitionFamily>) -> Option<&'static str> {
    match family {
        Some(DefinitionFamily::Mysql) => Some(
            "SELECT CONCAT(User, '@', Host) AS name, User AS user_name, Host AS host, \
             IFNULL(plugin, '') AS plugin, \
             IF(password_expired = 'Y', 'EXPIRED', 'DEFAULT') AS expire, \
             max_questions AS max_queries, max_updates, max_connections, max_user_connections \
             FROM mysql.user ORDER BY User, Host",
        ),
        Some(DefinitionFamily::Postgres) => Some(
            "SELECT rolname AS name, \
             CASE WHEN rolcanlogin THEN 'login' ELSE 'group' END AS kind, \
             rolsuper AS superuser, rolcreatedb AS createdb, rolcreaterole AS createrole, \
             rolconnlimit AS connection_limit \
             FROM pg_roles ORDER BY rolname",
        ),
        _ => None,
    }
}

pub fn grant_statements(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    draft: &GrantDraft,
) -> Result<Vec<String>, String> {
    match family {
        Some(DefinitionFamily::Mysql) => mysql_grant(dialect, draft),
        Some(DefinitionFamily::Postgres) => postgres_grant(dialect, draft),
        Some(DefinitionFamily::Sqlite) | None => {
            Err("This connection has no privilege manager.".into())
        }
    }
}

pub fn manages_privileges(family: Option<DefinitionFamily>) -> bool {
    matches!(
        family,
        Some(DefinitionFamily::Mysql | DefinitionFamily::Postgres)
    )
}

pub fn monitors_server(family: Option<DefinitionFamily>) -> bool {
    manages_privileges(family)
}

pub fn maintenance(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    scope: &Scope,
    target: Option<&str>,
) -> Vec<MaintenanceJob> {
    let Some(family) = family else {
        return Vec::new();
    };
    let object = target.map(|name| dialect.qualified(scope, name));
    match family {
        DefinitionFamily::Mysql => {
            let Some(object) = object else {
                return Vec::new();
            };
            ["analyze", "check", "optimize", "repair"]
                .into_iter()
                .map(|key| MaintenanceJob {
                    key,
                    sql: format!(
                        "{} TABLE {object}",
                        match key {
                            "analyze" => "ANALYZE",
                            "check" => "CHECK",
                            "optimize" => "OPTIMIZE",
                            _ => "REPAIR",
                        }
                    ),
                })
                .collect()
        }
        DefinitionFamily::Postgres => {
            let target_sql = object.unwrap_or_default();
            let vacuum = if target_sql.is_empty() {
                "VACUUM".into()
            } else {
                format!("VACUUM {target_sql}")
            };
            let analyze = if target_sql.is_empty() {
                "ANALYZE".into()
            } else {
                format!("ANALYZE {target_sql}")
            };
            let reindex = if target_sql.is_empty() {
                format!("REINDEX DATABASE {}", dialect.quote_ident(&scope.database))
            } else {
                format!("REINDEX TABLE {target_sql}")
            };
            vec![
                MaintenanceJob {
                    key: "vacuum",
                    sql: vacuum,
                },
                MaintenanceJob {
                    key: "analyze",
                    sql: analyze,
                },
                MaintenanceJob {
                    key: "reindex",
                    sql: reindex,
                },
            ]
        }
        DefinitionFamily::Sqlite => {
            let mut jobs = Vec::new();
            // VACUUM of an attached database is refused; only the main database.
            if target.is_none() && scope.database.eq_ignore_ascii_case("main") {
                jobs.push(MaintenanceJob {
                    key: "vacuum",
                    sql: "VACUUM".into(),
                });
            }
            let analyze = match &object {
                Some(name) => format!("ANALYZE {name}"),
                None => "ANALYZE".into(),
            };
            let reindex = match &object {
                Some(name) => format!("REINDEX {name}"),
                None => "REINDEX".into(),
            };
            jobs.push(MaintenanceJob {
                key: "analyze",
                sql: analyze,
            });
            jobs.push(MaintenanceJob {
                key: "reindex",
                sql: reindex,
            });
            if target.is_none() {
                jobs.push(MaintenanceJob {
                    key: "integrity",
                    sql: "PRAGMA integrity_check".into(),
                });
            }
            jobs
        }
    }
}

pub fn process_sql(family: Option<DefinitionFamily>) -> Option<&'static str> {
    match family {
        Some(DefinitionFamily::Mysql) => Some("SHOW FULL PROCESSLIST"),
        Some(DefinitionFamily::Postgres) => Some(
            "SELECT pid, usename, datname, state, left(query, 200) FROM pg_stat_activity ORDER BY pid",
        ),
        _ => None,
    }
}

pub fn kill_sql(family: Option<DefinitionFamily>, id: &str) -> Result<Option<String>, String> {
    let Some(family) = family else {
        return Ok(None);
    };
    if !monitors_server(Some(family)) {
        return Ok(None);
    }
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Enter a numeric process id.".into());
    }
    let sql = match family {
        DefinitionFamily::Mysql => format!("KILL {id}"),
        DefinitionFamily::Postgres => format!("SELECT pg_cancel_backend({id})"),
        DefinitionFamily::Sqlite => return Ok(None),
    };
    Ok(Some(sql))
}

pub fn variables_sql(family: Option<DefinitionFamily>) -> Option<&'static str> {
    match family {
        Some(DefinitionFamily::Mysql) => Some("SHOW GLOBAL VARIABLES"),
        Some(DefinitionFamily::Postgres) => {
            Some("SELECT name, setting, unit FROM pg_settings ORDER BY name")
        }
        _ => None,
    }
}

pub fn set_variable_sql(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    name: &str,
    value: &str,
) -> Result<Vec<String>, String> {
    let Some(family) = family else {
        return Ok(Vec::new());
    };
    if !monitors_server(Some(family)) {
        return Ok(Vec::new());
    }
    let name = name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    {
        return Err("Enter a variable name.".into());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err("Enter a value.".into());
    }
    let literal = if value.chars().all(|c| c.is_ascii_digit()) {
        value.to_string()
    } else {
        dialect.string_literal(value)
    };
    let sql = match family {
        DefinitionFamily::Mysql => vec![format!("SET GLOBAL {name} = {literal}")],
        // Spec 43 §5: PostgreSQL variables are read-only. `ALTER SYSTEM` writes the server config.
        DefinitionFamily::Postgres => {
            return Err("PostgreSQL variables are read-only.".into());
        }
        DefinitionFamily::Sqlite => Vec::new(),
    };
    Ok(sql)
}

pub fn status_sql(family: Option<DefinitionFamily>) -> Option<&'static str> {
    match family {
        Some(DefinitionFamily::Mysql) => Some("SHOW GLOBAL STATUS"),
        Some(DefinitionFamily::Postgres) => {
            Some("SELECT * FROM pg_stat_database WHERE datname = current_database()")
        }
        _ => None,
    }
}

pub fn find_plan(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    scope: &Scope,
    request: FindRequest<'_>,
) -> Result<FindPlan, String> {
    let family = family.ok_or_else(|| "This connection cannot be searched.".to_string())?;
    let needle = request.needle.trim();
    if needle.is_empty() {
        return Err("Enter text to find.".into());
    }
    if request.mode == MatchMode::Regex {
        regex::Regex::new(needle).map_err(|e| format!("Invalid regular expression: {e}"))?;
    }
    let structure = structure_queries(
        dialect,
        family,
        scope,
        needle,
        request.mode,
        request.case_sensitive,
        request.kinds,
    )?;
    let text_columns = if request.data {
        Some(text_columns_sql(dialect, family, scope))
    } else {
        None
    };
    Ok(FindPlan {
        structure,
        text_columns,
        needle: needle.to_string(),
        mode: request.mode,
        case_sensitive: request.case_sensitive,
    })
}

/// `SELECT * FROM table WHERE text columns match LIMIT 100`.
pub fn data_probe(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    scope: &Scope,
    probe: Probe<'_>,
) -> Result<String, String> {
    let family = family.ok_or_else(|| "This connection cannot be searched.".to_string())?;
    if probe.columns.is_empty() {
        return Err("No text columns.".into());
    }
    let parts: Result<Vec<_>, _> = probe
        .columns
        .iter()
        .map(|column| {
            predicate(
                dialect,
                family,
                &dialect.quote_ident(column),
                probe.needle,
                probe.mode,
                probe.case_sensitive,
                true,
            )
        })
        .collect();
    Ok(format!(
        "SELECT * FROM {} WHERE {} LIMIT 100",
        dialect.qualified(scope, probe.table),
        parts?.join(" OR ")
    ))
}

pub fn relation_plan(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    object: &ObjectRef,
    membership: bool,
) -> RelationPlan {
    let Some(family) = family else {
        return RelationPlan {
            note: Some("This connection has no dependency information."),
            queries: Vec::new(),
        };
    };
    if membership && family == DefinitionFamily::Sqlite {
        return RelationPlan {
            note: Some(SQLITE_AUTH_REASON),
            queries: Vec::new(),
        };
    }
    let name = dialect.string_literal(&object.name);
    let schema = dialect.string_literal(object.scope.schema.as_deref().unwrap_or("public"));
    let database = dialect.string_literal(&object.scope.database);
    let queries = match (family, membership) {
        (DefinitionFamily::Postgres, false) => vec![
            NamedSql {
                section: Section::Uses,
                sql: format!(
                    "SELECT DISTINCT n.nspname, c.relname FROM pg_depend d \
                     JOIN pg_class c ON c.oid = d.refobjid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE d.objid = (SELECT c2.oid FROM pg_class c2 \
                     JOIN pg_namespace n2 ON n2.oid = c2.relnamespace \
                     WHERE n2.nspname = {schema} AND c2.relname = {name})"
                ),
            },
            NamedSql {
                section: Section::UsedBy,
                sql: format!(
                    "SELECT DISTINCT n.nspname, c.relname FROM pg_depend d \
                     JOIN pg_class c ON c.oid = d.objid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE d.refobjid = (SELECT c2.oid FROM pg_class c2 \
                     JOIN pg_namespace n2 ON n2.oid = c2.relnamespace \
                     WHERE n2.nspname = {schema} AND c2.relname = {name})"
                ),
            },
        ],
        (DefinitionFamily::Postgres, true) => vec![
            NamedSql {
                section: Section::Members,
                sql: format!(
                    "SELECT r.rolname FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member \
                     WHERE m.roleid = (SELECT oid FROM pg_roles WHERE rolname = {name})"
                ),
            },
            NamedSql {
                section: Section::MemberOf,
                sql: format!(
                    "SELECT r.rolname FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid \
                     WHERE m.member = (SELECT oid FROM pg_roles WHERE rolname = {name})"
                ),
            },
            NamedSql {
                section: Section::Privileges,
                sql: format!(
                    "SELECT table_schema, table_name, privilege_type \
                     FROM information_schema.role_table_grants WHERE grantee = {name}"
                ),
            },
        ],
        (DefinitionFamily::Mysql, false) => vec![
            NamedSql {
                section: Section::Uses,
                sql: format!(
                    "SELECT REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, CONSTRAINT_NAME \
                     FROM information_schema.KEY_COLUMN_USAGE \
                     WHERE TABLE_SCHEMA = {database} AND TABLE_NAME = {name} \
                     AND REFERENCED_TABLE_NAME IS NOT NULL"
                ),
            },
            NamedSql {
                section: Section::UsedBy,
                sql: format!(
                    "SELECT TABLE_SCHEMA, TABLE_NAME, CONSTRAINT_NAME \
                     FROM information_schema.KEY_COLUMN_USAGE \
                     WHERE REFERENCED_TABLE_SCHEMA = {database} AND REFERENCED_TABLE_NAME = {name}"
                ),
            },
        ],
        (DefinitionFamily::Mysql, true) => {
            let grantee = dialect.string_literal(&format!(
                "'{}'@'{}'",
                object.name,
                object.scope.schema.as_deref().unwrap_or("%")
            ));
            vec![
                NamedSql {
                    section: Section::Members,
                    sql: format!(
                        "SELECT TO_USER, TO_HOST FROM mysql.role_edges WHERE FROM_USER = {name}"
                    ),
                },
                NamedSql {
                    section: Section::MemberOf,
                    sql: format!(
                        "SELECT FROM_USER, FROM_HOST FROM mysql.role_edges WHERE TO_USER = {name}"
                    ),
                },
                NamedSql {
                    section: Section::Privileges,
                    sql: format!(
                        "SELECT TABLE_SCHEMA, TABLE_NAME, PRIVILEGE_TYPE \
                         FROM information_schema.TABLE_PRIVILEGES WHERE GRANTEE = {grantee}"
                    ),
                },
            ]
        }
        (DefinitionFamily::Sqlite, false) => {
            let master = format!(
                "{}.sqlite_master",
                dialect.quote_ident(&object.scope.database)
            );
            let like = dialect.string_literal(&format!("%{}%", object.name.replace('%', "\\%")));
            vec![NamedSql {
                section: Section::UsedBy,
                sql: format!(
                    "SELECT type, name FROM {master} WHERE name != {name} AND sql LIKE {like} ESCAPE '\\'"
                ),
            }]
        }
        (DefinitionFamily::Sqlite, true) => Vec::new(),
    };
    RelationPlan {
        note: None,
        queries,
    }
}

fn mysql_account(dialect: &dyn Dialect, draft: &AccountDraft) -> Result<Vec<String>, String> {
    let name = account_name(&draft.name)?;
    let host = host_or_default(draft.host.trim());
    let previous = draft
        .previous_name
        .as_deref()
        .map(account_name)
        .transpose()?;
    let mut sql = Vec::new();
    if draft.role {
        let ident = dialect.quote_ident(name);
        if let Some(previous) = previous.filter(|previous| *previous != name) {
            let host = dialect.quote_ident("%");
            sql.push(format!(
                "RENAME USER {}@{host} TO {ident}@{host}",
                dialect.quote_ident(previous)
            ));
        } else if draft.previous_name.is_none() {
            sql.push(format!("CREATE ROLE {ident}"));
        }
        return Ok(sql);
    }
    let user = format!(
        "{}@{}",
        dialect.quote_ident(name),
        dialect.quote_ident(host)
    );
    let mut alter = String::new();
    // Grammar: auth option, REQUIRE, WITH limits, PASSWORD EXPIRE, then ACCOUNT LOCK.
    alter.push_str(&mysql_identified(dialect, draft)?);
    alter.push_str(&mysql_require(dialect, draft)?);
    alter.push_str(&mysql_limits(draft)?);
    alter.push_str(&mysql_expire(draft.expire.trim())?);
    if draft.locked {
        alter.push_str(" ACCOUNT LOCK");
    } else if draft.previous_locked == Some(true) {
        alter.push_str(" ACCOUNT UNLOCK");
    }
    if let Some(previous) = previous {
        let previous_host = host_or_default(draft.previous_host.as_deref().unwrap_or("").trim());
        let old = format!(
            "{}@{}",
            dialect.quote_ident(previous),
            dialect.quote_ident(previous_host)
        );
        if previous != name || previous_host != host {
            sql.push(format!("RENAME USER {old} TO {user}"));
        }
        if !alter.trim().is_empty() {
            sql.push(format!("ALTER USER {user}{}", alter.trim_end()));
        }
    } else {
        sql.push(format!("CREATE USER {user}{}", alter.trim_end()));
    }
    Ok(sql)
}

fn postgres_account(dialect: &dyn Dialect, draft: &AccountDraft) -> Result<Vec<String>, String> {
    let name = account_name(&draft.name)?;
    let ident = dialect.quote_ident(name);
    let mut options = vec![if draft.login { "LOGIN" } else { "NOLOGIN" }];
    options.push(if draft.superuser {
        "SUPERUSER"
    } else {
        "NOSUPERUSER"
    });
    options.push(if draft.createdb {
        "CREATEDB"
    } else {
        "NOCREATEDB"
    });
    options.push(if draft.createrole {
        "CREATEROLE"
    } else {
        "NOCREATEROLE"
    });
    options.push(if draft.inherit {
        "INHERIT"
    } else {
        "NOINHERIT"
    });
    options.push(if draft.replication {
        "REPLICATION"
    } else {
        "NOREPLICATION"
    });
    let mut clause = options.join(" ");
    if let Some(password) = optional_password(&draft.password) {
        clause.push_str(" PASSWORD ");
        clause.push_str(&dialect.string_literal(password));
    }
    if let Some(limit) = number("connection limit", &draft.connection_limit)? {
        clause.push_str(" CONNECTION LIMIT ");
        clause.push_str(&limit);
    }
    let until = draft.valid_until.trim();
    if !until.is_empty() {
        clause.push_str(" VALID UNTIL ");
        clause.push_str(&dialect.string_literal(until));
    }
    let mut sql = Vec::new();
    if let Some(previous) = draft
        .previous_name
        .as_deref()
        .map(account_name)
        .transpose()?
    {
        let old = dialect.quote_ident(previous);
        if previous != name {
            sql.push(format!("ALTER ROLE {old} RENAME TO {ident}"));
        }
        sql.push(format!("ALTER ROLE {ident} WITH {clause}"));
    } else {
        sql.push(format!("CREATE ROLE {ident} WITH {clause}"));
    }
    Ok(sql)
}

fn mysql_grant(dialect: &dyn Dialect, draft: &GrantDraft) -> Result<Vec<String>, String> {
    let privilege = privilege(&draft.privilege)?;
    let object = quote_grant_object(dialect, &draft.object, true)?;
    let grantee = account_name(&draft.grantee)?;
    let host = host_or_default(draft.host.trim());
    let user = format!(
        "{}@{}",
        dialect.quote_ident(grantee),
        dialect.quote_ident(host)
    );
    let sql = if draft.revoke {
        format!("REVOKE {privilege} ON {object} FROM {user}")
    } else if draft.grant_option {
        format!("GRANT {privilege} ON {object} TO {user} WITH GRANT OPTION")
    } else {
        format!("GRANT {privilege} ON {object} TO {user}")
    };
    Ok(vec![sql])
}

fn postgres_grant(dialect: &dyn Dialect, draft: &GrantDraft) -> Result<Vec<String>, String> {
    let privilege = privilege(&draft.privilege)?;
    let object = quote_grant_object(dialect, &draft.object, false)?;
    let grantee = dialect.quote_ident(account_name(&draft.grantee)?);
    let sql = if draft.revoke {
        format!("REVOKE {privilege} ON TABLE {object} FROM {grantee}")
    } else if draft.grant_option {
        format!("GRANT {privilege} ON TABLE {object} TO {grantee} WITH GRANT OPTION")
    } else {
        format!("GRANT {privilege} ON TABLE {object} TO {grantee}")
    };
    Ok(vec![sql])
}

fn structure_queries(
    dialect: &dyn Dialect,
    family: DefinitionFamily,
    scope: &Scope,
    needle: &str,
    mode: MatchMode,
    case_sensitive: bool,
    kinds: &[ObjectKind],
) -> Result<Vec<String>, String> {
    let wanted = |kind: ObjectKind| kinds.is_empty() || kinds.contains(&kind);
    match family {
        DefinitionFamily::Mysql => {
            let schema = dialect.string_literal(&scope.database);
            let mut sql = Vec::new();
            let pred = |column: &str| {
                predicate(dialect, family, column, needle, mode, case_sensitive, false)
            };
            if wanted(ObjectKind::Table) {
                sql.push(format!(
                    "SELECT 'table', TABLE_NAME FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = {schema} AND TABLE_TYPE = 'BASE TABLE' AND {}",
                    pred("TABLE_NAME")?
                ));
            }
            if wanted(ObjectKind::View) {
                sql.push(format!(
                    "SELECT 'view', TABLE_NAME FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = {schema} AND TABLE_TYPE = 'VIEW' AND {}",
                    pred("TABLE_NAME")?
                ));
            }
            if wanted(ObjectKind::Function) {
                sql.push(format!(
                    "SELECT ROUTINE_TYPE, ROUTINE_NAME FROM information_schema.ROUTINES \
                     WHERE ROUTINE_SCHEMA = {schema} AND {}",
                    pred("ROUTINE_NAME")?
                ));
            }
            if wanted(ObjectKind::Event) {
                sql.push(format!(
                    "SELECT 'event', EVENT_NAME FROM information_schema.EVENTS \
                     WHERE EVENT_SCHEMA = {schema} AND {}",
                    pred("EVENT_NAME")?
                ));
            }
            if wanted(ObjectKind::Trigger) {
                sql.push(format!(
                    "SELECT 'trigger', TRIGGER_NAME FROM information_schema.TRIGGERS \
                     WHERE TRIGGER_SCHEMA = {schema} AND {}",
                    pred("TRIGGER_NAME")?
                ));
            }
            Ok(sql)
        }
        DefinitionFamily::Postgres => {
            let schema = dialect.string_literal(scope.schema.as_deref().unwrap_or("public"));
            let pred = predicate(
                dialect,
                family,
                "c.relname",
                needle,
                mode,
                case_sensitive,
                false,
            )?;
            let mut kinds_sql = Vec::new();
            if wanted(ObjectKind::Table) {
                kinds_sql.push("'r'");
            }
            if wanted(ObjectKind::View) {
                kinds_sql.push("'v'");
            }
            if wanted(ObjectKind::MaterializedView) {
                kinds_sql.push("'m'");
            }
            if wanted(ObjectKind::Index) {
                kinds_sql.push("'i'");
            }
            if wanted(ObjectKind::Sequence) {
                kinds_sql.push("'S'");
            }
            let mut sql = Vec::new();
            if !kinds_sql.is_empty() {
                sql.push(format!(
                    "SELECT c.relkind, c.relname FROM pg_class c \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = {schema} AND c.relkind IN ({}) AND {pred}",
                    kinds_sql.join(", ")
                ));
            }
            if wanted(ObjectKind::Function) {
                let p = predicate(
                    dialect,
                    family,
                    "p.proname",
                    needle,
                    mode,
                    case_sensitive,
                    false,
                )?;
                sql.push(format!(
                    "SELECT 'function', p.proname FROM pg_proc p \
                     JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = {schema} AND {p}"
                ));
            }
            Ok(sql)
        }
        DefinitionFamily::Sqlite => {
            let master = format!("{}.sqlite_master", dialect.quote_ident(&scope.database));
            let pred = predicate(dialect, family, "name", needle, mode, case_sensitive, false)?;
            let mut types = Vec::new();
            if wanted(ObjectKind::Table) {
                types.push("'table'");
            }
            if wanted(ObjectKind::View) {
                types.push("'view'");
            }
            if wanted(ObjectKind::Index) {
                types.push("'index'");
            }
            if wanted(ObjectKind::Trigger) {
                types.push("'trigger'");
            }
            if types.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![format!(
                "SELECT type, name FROM {master} WHERE type IN ({}) AND {pred}",
                types.join(", ")
            )])
        }
    }
}

fn text_columns_sql(dialect: &dyn Dialect, family: DefinitionFamily, scope: &Scope) -> String {
    match family {
        DefinitionFamily::Mysql => {
            let schema = dialect.string_literal(&scope.database);
            format!(
                "SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = {schema} AND DATA_TYPE IN \
                 ('char','varchar','tinytext','text','mediumtext','longtext','enum','set','json')"
            )
        }
        DefinitionFamily::Postgres => {
            let schema = dialect.string_literal(scope.schema.as_deref().unwrap_or("public"));
            format!(
                "SELECT table_name, column_name FROM information_schema.columns \
                 WHERE table_schema = {schema} AND data_type IN \
                 ('text','character varying','character','json','jsonb')"
            )
        }
        DefinitionFamily::Sqlite => {
            let master = format!("{}.sqlite_master", dialect.quote_ident(&scope.database));
            let schema = dialect.string_literal(&scope.database);
            format!(
                "SELECT m.name, p.name FROM {master} m \
                 JOIN pragma_table_info(m.name, {schema}) p \
                 WHERE m.type = 'table' AND (p.type = '' OR upper(p.type) LIKE '%CHAR%' \
                 OR upper(p.type) LIKE '%TEXT%' OR upper(p.type) LIKE '%CLOB%')"
            )
        }
    }
}

fn predicate(
    dialect: &dyn Dialect,
    family: DefinitionFamily,
    column: &str,
    needle: &str,
    mode: MatchMode,
    case_sensitive: bool,
    data: bool,
) -> Result<String, String> {
    if mode == MatchMode::Regex {
        if family == DefinitionFamily::Sqlite {
            return Err("This SQLite build has no REGEXP function.".into());
        }
        // Lowercasing the pattern turns `\S` into `\s`. PostgreSQL has `~*`;
        // MySQL `REGEXP` follows the column collation.
        let literal = dialect.string_literal(needle);
        return Ok(match (family, case_sensitive) {
            (DefinitionFamily::Postgres, false) => format!("{column} ~* {literal}"),
            (DefinitionFamily::Postgres, true) => format!("{column} ~ {literal}"),
            (DefinitionFamily::Mysql, true) => {
                // MySQL 8.0.22 rejects a binary argument to REGEXP. `(?-i)` forces a
                // case-sensitive match without leaving the column's character set.
                // MariaDB's PCRE engine accepts the same flag.
                let literal = dialect.string_literal(&format!("(?-i){needle}"));
                format!("{column} REGEXP {literal}")
            }
            (DefinitionFamily::Mysql, false) | (DefinitionFamily::Sqlite, _) => {
                format!("{column} REGEXP {literal}")
            }
        });
    }
    if mode == MatchMode::WholeWord && data {
        let word = escape_like(needle);
        let mid = like_cmp(
            dialect,
            family,
            column,
            &format!("% {word} %"),
            case_sensitive,
        );
        let start = like_cmp(
            dialect,
            family,
            column,
            &format!("{word} %"),
            case_sensitive,
        );
        let end = like_cmp(
            dialect,
            family,
            column,
            &format!("% {word}"),
            case_sensitive,
        );
        let exact = like_cmp(dialect, family, column, &word, case_sensitive);
        return Ok(format!("({mid} OR {start} OR {end} OR {exact})"));
    }
    let pattern = match mode {
        MatchMode::Contains => format!("%{}%", escape_like(needle)),
        MatchMode::Prefix => format!("{}%", escape_like(needle)),
        MatchMode::WholeWord | MatchMode::Regex => escape_like(needle),
    };
    Ok(like_cmp(dialect, family, column, &pattern, case_sensitive))
}

fn like_cmp(
    dialect: &dyn Dialect,
    family: DefinitionFamily,
    column: &str,
    pattern: &str,
    case_sensitive: bool,
) -> String {
    let literal = dialect.string_literal(pattern);
    let escape = like_escape(dialect, family);
    match (family, case_sensitive) {
        (DefinitionFamily::Postgres, false) => format!("{column} ILIKE {literal} ESCAPE {escape}"),
        (DefinitionFamily::Postgres, true) | (DefinitionFamily::Mysql, false) => {
            format!("{column} LIKE {literal} ESCAPE {escape}")
        }
        (DefinitionFamily::Mysql, true) => {
            // `COLLATE utf8mb4_bin` fails when the connection character set is latin1.
            format!("{column} LIKE BINARY {literal} ESCAPE {escape}")
        }
        (DefinitionFamily::Sqlite, false) => format!("{column} LIKE {literal} ESCAPE {escape}"),
        (DefinitionFamily::Sqlite, true) => {
            // SQLite LIKE is case-insensitive for ASCII. GLOB is case-sensitive.
            format!(
                "{column} GLOB {}",
                dialect.string_literal(&like_to_glob(pattern))
            )
        }
    }
}

fn like_escape(dialect: &dyn Dialect, family: DefinitionFamily) -> String {
    match family {
        // A SQLite string does not treat `\` as an escape, so one backslash is the character.
        DefinitionFamily::Sqlite => "'\\'".into(),
        _ => dialect.string_literal("\\"),
    }
}

/// LIKE pattern (`%`, `_`, `\` escapes) to a SQLite GLOB pattern.
fn like_to_glob(pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str(&glob_literal(chars.next().unwrap_or('\\'))),
            '%' => out.push('*'),
            '_' => out.push('?'),
            other => out.push_str(&glob_literal(other)),
        }
    }
    out
}

fn glob_literal(c: char) -> String {
    match c {
        '*' | '?' | '[' | ']' => format!("[{c}]"),
        c => c.to_string(),
    }
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn account_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.contains('\0') || name.contains(';') {
        return Err("Enter a name.".into());
    }
    Ok(name)
}

fn optional_password(password: &str) -> Option<&str> {
    let password = password.trim();
    if password.is_empty() {
        None
    } else {
        Some(password)
    }
}

fn host_or_default(host: &str) -> &str {
    if host.is_empty() { "%" } else { host }
}

fn number(label: &str, value: &str) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if !value.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{label} must be a number."));
    }
    Ok(Some(value.to_string()))
}

fn mysql_expire(expire: &str) -> Result<String, String> {
    if expire.is_empty() {
        return Ok(String::new());
    }
    if expire.eq_ignore_ascii_case("never") {
        return Ok(" PASSWORD EXPIRE NEVER".into());
    }
    if expire.chars().all(|c| c.is_ascii_digit()) {
        return Ok(format!(" PASSWORD EXPIRE INTERVAL {expire} DAY"));
    }
    Err("Expire must be empty, never, or a number of days.".into())
}

fn mysql_limits(draft: &AccountDraft) -> Result<String, String> {
    let queries = number("max queries", &draft.max_queries)?;
    let updates = number("max updates", &draft.max_updates)?;
    let connections = number("max connections", &draft.max_connections)?;
    let user_connections = number("max user connections", &draft.max_user_connections)?;
    if queries.is_none() && updates.is_none() && connections.is_none() && user_connections.is_none()
    {
        return Ok(String::new());
    }
    Ok(format!(
        " WITH MAX_QUERIES_PER_HOUR {} MAX_UPDATES_PER_HOUR {} MAX_CONNECTIONS_PER_HOUR {} MAX_USER_CONNECTIONS {}",
        queries.unwrap_or_else(|| "0".into()),
        updates.unwrap_or_else(|| "0".into()),
        connections.unwrap_or_else(|| "0".into()),
        user_connections.unwrap_or_else(|| "0".into())
    ))
}

fn privilege(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("all") || raw.eq_ignore_ascii_case("all privileges") {
        return Ok("ALL PRIVILEGES".into());
    }
    if raw.is_empty()
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ' || c == ',')
    {
        return Err("Enter a privilege such as SELECT.".into());
    }
    Ok(raw.to_ascii_uppercase())
}

fn mysql_identified(dialect: &dyn Dialect, draft: &AccountDraft) -> Result<String, String> {
    let plugin = draft.plugin.trim();
    if !plugin.is_empty()
        && !plugin
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("Plugin name must be letters, digits, or underscores.".into());
    }
    let previous = draft.previous_plugin.as_deref();
    if let Some(password) = optional_password(&draft.password) {
        let by = dialect.string_literal(password);
        if plugin.is_empty() {
            return Ok(format!(" IDENTIFIED BY {by}"));
        }
        return Ok(format!(" IDENTIFIED WITH {plugin} BY {by}"));
    }
    // A loaded account keeps its plugin until the field actually changes.
    let changed = match previous {
        Some(previous) => previous != plugin && !plugin.is_empty(),
        None => !plugin.is_empty() && draft.previous_name.is_none(),
    };
    // `IDENTIFIED WITH plugin` and no password replaces the hash. An existing
    // account has to supply a new password; a new account may omit it.
    if changed && draft.previous_name.is_some() {
        return Err("Enter a password to change the authentication plugin.".into());
    }
    if changed {
        Ok(format!(" IDENTIFIED WITH {plugin}"))
    } else {
        Ok(String::new())
    }
}

fn mysql_require(dialect: &dyn Dialect, draft: &AccountDraft) -> Result<String, String> {
    let ssl = draft.ssl_type.trim();
    if ssl.is_empty() {
        return Ok(String::new());
    }
    let same = draft.previous_ssl.as_deref().unwrap_or("") == ssl
        && draft.previous_cipher.as_deref().unwrap_or("") == draft.ssl_cipher.trim()
        && draft.previous_issuer.as_deref().unwrap_or("") == draft.ssl_issuer.trim()
        && draft.previous_subject.as_deref().unwrap_or("") == draft.ssl_subject.trim();
    if same {
        return Ok(String::new());
    }
    let clause = match ssl.to_ascii_uppercase().as_str() {
        "NONE" => "NONE".to_string(),
        "ANY" => "SSL".to_string(),
        "X509" => "X509".to_string(),
        "SPECIFIED" => {
            let mut parts = Vec::new();
            if !draft.ssl_cipher.trim().is_empty() {
                parts.push(format!(
                    "CIPHER {}",
                    dialect.string_literal(draft.ssl_cipher.trim())
                ));
            }
            if !draft.ssl_issuer.trim().is_empty() {
                parts.push(format!(
                    "ISSUER {}",
                    dialect.string_literal(draft.ssl_issuer.trim())
                ));
            }
            if !draft.ssl_subject.trim().is_empty() {
                parts.push(format!(
                    "SUBJECT {}",
                    dialect.string_literal(draft.ssl_subject.trim())
                ));
            }
            if parts.is_empty() {
                return Err("SSL SPECIFIED needs a cipher, issuer, or subject.".into());
            }
            parts.join(" AND ")
        }
        _ => return Err("SSL type must be NONE, ANY, X509, or SPECIFIED.".into()),
    };
    Ok(format!(" REQUIRE {clause}"))
}

/// Queries that fill the user designer. `tables` is omitted when no database is open.
pub struct UserLoadSql {
    pub profile: String,
    pub grants: String,
    pub accounts: String,
    pub tables: Option<String>,
    pub defaults: String,
    pub members: String,
}

/// One row of PostgreSQL role attributes for the designer.
pub fn role_profile_sql(dialect: &dyn Dialect, name: &str) -> Result<String, String> {
    let name = dialect.string_literal(account_name(name)?);
    Ok(format!(
        "SELECT rolcanlogin, rolsuper, rolcreatedb, rolcreaterole, rolinherit, \
         rolreplication, rolconnlimit, COALESCE(rolvaliduntil::text, '') \
         FROM pg_roles WHERE rolname = {name}"
    ))
}

pub fn user_load_sql(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    name: &str,
    host: &str,
    database: &str,
) -> Result<UserLoadSql, String> {
    if family != Some(DefinitionFamily::Mysql) {
        return Err("This connection has no user designer.".into());
    }
    let name_lit = dialect.string_literal(account_name(name)?);
    let host_lit = dialect.string_literal(host_or_default(host.trim()));
    let user = format!(
        "{}@{}",
        dialect.quote_ident(account_name(name)?),
        dialect.quote_ident(host_or_default(host.trim()))
    );
    let tables = (!database.trim().is_empty()).then(|| {
        format!(
            "SELECT TABLE_SCHEMA, TABLE_NAME FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = {} AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
            dialect.string_literal(database.trim())
        )
    });
    Ok(UserLoadSql {
        profile: format!(
            "SELECT IFNULL(plugin, ''), IF(account_locked = 'Y', '1', '0'), \
             IF(password_expired = 'Y', 'EXPIRED', 'DEFAULT'), \
             max_questions, max_updates, max_connections, max_user_connections, \
             IFNULL(ssl_type, ''), IFNULL(ssl_cipher, ''), IFNULL(x509_issuer, ''), \
             IFNULL(x509_subject, '') \
             FROM mysql.user WHERE User = {name_lit} AND Host = {host_lit}"
        ),
        grants: format!("SHOW GRANTS FOR {user}"),
        accounts: account_list_sql(family)
            .unwrap_or("SELECT User AS user_name, Host AS host FROM mysql.user")
            .to_string(),
        tables,
        defaults: format!(
            "SELECT DEFAULT_ROLE_USER, DEFAULT_ROLE_HOST FROM mysql.default_roles \
             WHERE USER = {name_lit} AND HOST = {host_lit}"
        ),
        members: format!(
            "SELECT TO_USER, TO_HOST FROM mysql.role_edges \
             WHERE FROM_USER = {name_lit} AND FROM_HOST = {host_lit}"
        ),
    })
}

/// Reads `SHOW GRANTS` lines. `USAGE` is ignored. `ALL PRIVILEGES` stays literal;
/// the designer expands it onto the checkboxes it knows.
pub fn parse_show_grants(lines: &[String]) -> GrantSet {
    let mut set = GrantSet::default();
    for line in lines {
        let Some(piece) = parse_grant_line(line) else {
            continue;
        };
        match piece {
            GrantPiece::Server {
                privileges,
                grant_option,
            } => {
                for privilege in privileges {
                    if privilege != "USAGE" {
                        set.server.insert(privilege);
                    }
                }
                set.server_grant_option |= grant_option;
            }
            GrantPiece::Role { name, host } => {
                set.roles.insert((name, host));
            }
            GrantPiece::Object {
                schema,
                name,
                privileges,
                grant_option,
            } => {
                let entry = set.objects.entry((schema, name)).or_default();
                entry.privileges.extend(privileges);
                entry.grant_option |= grant_option;
            }
        }
    }
    set
}

enum GrantPiece {
    Server {
        privileges: Vec<String>,
        grant_option: bool,
    },
    Role {
        name: String,
        host: String,
    },
    Object {
        schema: String,
        name: String,
        privileges: Vec<String>,
        grant_option: bool,
    },
}

fn parse_grant_line(line: &str) -> Option<GrantPiece> {
    let line = line.trim().trim_end_matches(';');
    let rest = line.strip_prefix("GRANT ")?;
    let grant_option = rest.contains("WITH GRANT OPTION");
    let rest = rest
        .split(" WITH GRANT OPTION")
        .next()
        .unwrap_or(rest)
        .trim();
    let to = rest.rfind(" TO ")?;
    let left = rest[..to].trim();
    if let Some(on) = left.rfind(" ON ") {
        let privileges = split_privs(&left[..on]);
        let object = left[on + 4..].trim();
        if object == "*.*" {
            return Some(GrantPiece::Server {
                privileges,
                grant_option,
            });
        }
        let (schema, name) = parse_grant_object(object)?;
        return Some(GrantPiece::Object {
            schema,
            name,
            privileges,
            grant_option,
        });
    }
    let (name, host) = parse_role_ref(left)?;
    Some(GrantPiece::Role { name, host })
}

fn split_privs(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|part| part.trim().to_ascii_uppercase())
        .filter(|part| !part.is_empty() && part != "USAGE")
        .collect()
}

fn parse_grant_object(raw: &str) -> Option<(String, String)> {
    let (schema, rest) = take_ident(raw.trim())?;
    let rest = rest.trim().strip_prefix('.')?.trim();
    if rest == "*" {
        return Some((schema, "*".into()));
    }
    let (name, _) = take_ident(rest)?;
    Some((schema, name))
}

fn parse_role_ref(raw: &str) -> Option<(String, String)> {
    let (name, rest) = take_ident(raw.trim())?;
    let rest = rest.trim().strip_prefix('@')?.trim();
    let (host, _) = take_ident(rest)?;
    Some((name, host))
}

fn take_ident(raw: &str) -> Option<(String, &str)> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('`') {
        let end = rest.find('`')?;
        return Some((rest[..end].to_string(), &rest[end + 1..]));
    }
    let end = raw.find(['@', ' ', '.', ',']).unwrap_or(raw.len());
    if end == 0 {
        return None;
    }
    Some((raw[..end].to_string(), &raw[end..]))
}

/// The two snapshots a privilege diff is taken from.
pub struct PrivilegeEdit<'a> {
    pub grantee: &'a str,
    pub host: &'a str,
    pub before: &'a GrantSet,
    pub after: &'a GrantSet,
    pub before_members: &'a BTreeSet<(String, String)>,
    pub after_members: &'a BTreeSet<(String, String)>,
}

/// `GRANT` / `REVOKE` for the difference between two snapshots. Privileges that are in
/// `before` but not in the designer's checklist stay if the caller copied them into `after`.
pub fn privilege_statements(
    dialect: &dyn Dialect,
    family: Option<DefinitionFamily>,
    edit: PrivilegeEdit<'_>,
) -> Result<Vec<String>, String> {
    match family {
        Some(DefinitionFamily::Mysql) => mysql_privilege_statements(dialect, &edit),
        Some(DefinitionFamily::Postgres) => {
            postgres_privilege_statements(dialect, edit.grantee, edit.before, edit.after)
        }
        Some(DefinitionFamily::Sqlite) | None => {
            Err("This connection has no privilege manager.".into())
        }
    }
}

fn mysql_privilege_statements(
    dialect: &dyn Dialect,
    edit: &PrivilegeEdit<'_>,
) -> Result<Vec<String>, String> {
    let PrivilegeEdit {
        grantee,
        host,
        before,
        after,
        before_members,
        after_members,
    } = edit;
    let user = mysql_account_ref(dialect, grantee, host)?;
    let mut sql = Vec::new();
    push_priv_diff(
        &mut sql,
        "*.*",
        &user,
        &before.server,
        before.server_grant_option,
        &after.server,
        after.server_grant_option,
    );
    let mut keys: BTreeSet<&(String, String)> = before.objects.keys().collect();
    keys.extend(after.objects.keys());
    for key in keys {
        let empty = ObjectGrant::default();
        let old = before.objects.get(key).unwrap_or(&empty);
        let new = after.objects.get(key).unwrap_or(&empty);
        let object = if key.1 == "*" {
            format!("{}.*", dialect.quote_ident(&key.0))
        } else {
            format!(
                "{}.{}",
                dialect.quote_ident(&key.0),
                dialect.quote_ident(&key.1)
            )
        };
        push_priv_diff(
            &mut sql,
            &object,
            &user,
            &old.privileges,
            old.grant_option,
            &new.privileges,
            new.grant_option,
        );
    }
    for role in after.roles.difference(&before.roles) {
        sql.push(format!(
            "GRANT {} TO {user}",
            mysql_account_ref(dialect, &role.0, &role.1)?
        ));
    }
    for role in before.roles.difference(&after.roles) {
        sql.push(format!(
            "REVOKE {} FROM {user}",
            mysql_account_ref(dialect, &role.0, &role.1)?
        ));
    }
    if before.defaults != after.defaults {
        if after.defaults.is_empty() {
            sql.push(format!("SET DEFAULT ROLE NONE TO {user}"));
        } else {
            let roles = after
                .defaults
                .iter()
                .map(|(name, host)| mysql_account_ref(dialect, name, host))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            sql.push(format!("SET DEFAULT ROLE {roles} TO {user}"));
        }
    }
    for member in after_members.difference(before_members) {
        sql.push(format!(
            "GRANT {user} TO {}",
            mysql_account_ref(dialect, &member.0, &member.1)?
        ));
    }
    for member in before_members.difference(after_members) {
        sql.push(format!(
            "REVOKE {user} FROM {}",
            mysql_account_ref(dialect, &member.0, &member.1)?
        ));
    }
    Ok(sql)
}

fn push_priv_diff(
    sql: &mut Vec<String>,
    object: &str,
    user: &str,
    before: &BTreeSet<String>,
    before_option: bool,
    after: &BTreeSet<String>,
    after_option: bool,
) {
    let added: Vec<&String> = after.difference(before).collect();
    let removed: Vec<&String> = before.difference(after).collect();
    if !added.is_empty() {
        let mut statement = format!("GRANT {} ON {object} TO {user}", join_privs(&added));
        if after_option && !before_option {
            statement.push_str(" WITH GRANT OPTION");
        }
        sql.push(statement);
    } else if after_option && !before_option && !after.is_empty() {
        let held: Vec<&String> = after.iter().collect();
        sql.push(format!(
            "GRANT {} ON {object} TO {user} WITH GRANT OPTION",
            join_privs(&held)
        ));
    }
    if before_option && !after_option {
        sql.push(format!("REVOKE GRANT OPTION ON {object} FROM {user}"));
    }
    if !removed.is_empty() {
        sql.push(format!(
            "REVOKE {} ON {object} FROM {user}",
            join_privs(&removed)
        ));
    }
}

fn join_privs(privileges: &[&String]) -> String {
    privileges
        .iter()
        .map(|privilege| privilege.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn mysql_account_ref(dialect: &dyn Dialect, name: &str, host: &str) -> Result<String, String> {
    Ok(format!(
        "{}@{}",
        dialect.quote_ident(account_name(name)?),
        dialect.quote_ident(host_or_default(host.trim()))
    ))
}

fn postgres_privilege_statements(
    dialect: &dyn Dialect,
    grantee: &str,
    before: &GrantSet,
    after: &GrantSet,
) -> Result<Vec<String>, String> {
    let user = dialect.quote_ident(account_name(grantee)?);
    let mut sql = Vec::new();
    for role in after.roles.difference(&before.roles) {
        sql.push(format!(
            "GRANT {} TO {user}",
            dialect.quote_ident(account_name(&role.0)?)
        ));
    }
    for role in before.roles.difference(&after.roles) {
        sql.push(format!(
            "REVOKE {} FROM {user}",
            dialect.quote_ident(account_name(&role.0)?)
        ));
    }
    Ok(sql)
}

fn quote_grant_object(dialect: &dyn Dialect, raw: &str, mysql: bool) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains(';') || raw.contains('\0') {
        return Err("Enter the object to grant.".into());
    }
    if raw == "*.*" || raw == "*" {
        if mysql {
            return Ok(raw.to_string());
        }
        return Err("Name the table.".into());
    }
    Ok(raw
        .split('.')
        .map(|part| {
            let part = part.trim();
            if part == "*" {
                "*".to_string()
            } else {
                dialect.quote_ident(part)
            }
        })
        .collect::<Vec<_>>()
        .join("."))
}
