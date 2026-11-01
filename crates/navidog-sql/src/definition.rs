//! SQL for the shared object designer. Each product picks a [`DefinitionFamily`]; feature code
//! never compares [`crate::Dialect::id`].

use std::collections::BTreeMap;

use navidog_model::object::{ObjectKind, Scope};

use crate::dialect::Dialect;
use crate::span::{self, Lex};

/// Which statement shape a dialect emits. A new database adds a variant and a match arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionFamily {
    Mysql,
    Postgres,
    Sqlite,
}

/// One designer form. Database and schema are navigator nodes, not [`ObjectKind`]s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionKind {
    View,
    MaterializedView,
    Function,
    Procedure,
    Event,
    Database,
    Schema,
    Tablespace,
    Sequence,
    Type,
    Domain,
    Extension,
    ForeignServer,
    Aggregate,
    Cast,
    Operator,
    Collation,
    Index,
    Trigger,
    Attach,
    Detach,
}

impl DefinitionKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Event => "event",
            Self::Database => "database",
            Self::Schema => "schema",
            Self::Tablespace => "tablespace",
            Self::Sequence => "sequence",
            Self::Type => "type",
            Self::Domain => "domain",
            Self::Extension => "extension",
            Self::ForeignServer => "foreign_server",
            Self::Aggregate => "aggregate",
            Self::Cast => "cast",
            Self::Operator => "operator",
            Self::Collation => "collation",
            Self::Index => "index",
            Self::Trigger => "trigger",
            Self::Attach => "attach",
            Self::Detach => "detach",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "view" => Self::View,
            "materialized_view" => Self::MaterializedView,
            "function" => Self::Function,
            "procedure" => Self::Procedure,
            "event" => Self::Event,
            "database" => Self::Database,
            "schema" => Self::Schema,
            "tablespace" => Self::Tablespace,
            "sequence" => Self::Sequence,
            "type" => Self::Type,
            "domain" => Self::Domain,
            "extension" => Self::Extension,
            "foreign_server" => Self::ForeignServer,
            "aggregate" => Self::Aggregate,
            "cast" => Self::Cast,
            "operator" => Self::Operator,
            "collation" => Self::Collation,
            "index" => Self::Index,
            "trigger" => Self::Trigger,
            "attach" => Self::Attach,
            "detach" => Self::Detach,
            _ => return None,
        })
    }

    /// Catalog kind used to reload an existing definition. Procedures live in the function list.
    pub fn catalog_kind(self) -> Option<ObjectKind> {
        Some(match self {
            Self::View => ObjectKind::View,
            Self::MaterializedView => ObjectKind::MaterializedView,
            Self::Function | Self::Procedure => ObjectKind::Function,
            Self::Event => ObjectKind::Event,
            Self::Tablespace => ObjectKind::Tablespace,
            Self::Sequence => ObjectKind::Sequence,
            Self::Type => ObjectKind::Type,
            Self::Domain => ObjectKind::Domain,
            Self::Extension => ObjectKind::Extension,
            Self::ForeignServer => ObjectKind::ForeignServer,
            Self::Aggregate => ObjectKind::Aggregate,
            Self::Cast => ObjectKind::Cast,
            Self::Operator => ObjectKind::Operator,
            Self::Collation => ObjectKind::Collation,
            Self::Index => ObjectKind::Index,
            Self::Trigger => ObjectKind::Trigger,
            Self::Database | Self::Schema | Self::Attach | Self::Detach => return None,
        })
    }

    pub fn from_object(kind: ObjectKind) -> Option<Self> {
        Some(match kind {
            ObjectKind::View => Self::View,
            ObjectKind::MaterializedView => Self::MaterializedView,
            ObjectKind::Function => Self::Function,
            ObjectKind::Event => Self::Event,
            ObjectKind::Tablespace => Self::Tablespace,
            ObjectKind::Sequence => Self::Sequence,
            ObjectKind::Type => Self::Type,
            ObjectKind::Domain => Self::Domain,
            ObjectKind::Extension => Self::Extension,
            ObjectKind::ForeignServer => Self::ForeignServer,
            ObjectKind::Aggregate => Self::Aggregate,
            ObjectKind::Cast => Self::Cast,
            ObjectKind::Operator => Self::Operator,
            ObjectKind::Collation => Self::Collation,
            ObjectKind::Index => Self::Index,
            ObjectKind::Trigger => Self::Trigger,
            ObjectKind::Table | ObjectKind::Query => return None,
        })
    }
}

/// Draft the designer edits. `raw` means `body` is the whole statement loaded from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionDraft {
    pub kind: DefinitionKind,
    pub name: String,
    pub previous: Option<String>,
    pub body: String,
    pub raw: bool,
    pub loaded: String,
    /// Server DDL from the last load or save. The external-change guard compares this, not `body`.
    pub server_ddl: String,
    pub options: BTreeMap<String, String>,
}

impl DefinitionDraft {
    pub fn new(kind: DefinitionKind) -> Self {
        Self {
            kind,
            name: String::new(),
            previous: None,
            body: String::new(),
            raw: false,
            loaded: String::new(),
            server_ddl: String::new(),
            options: BTreeMap::new(),
        }
    }

    pub fn option(&self, key: &str) -> &str {
        self.options.get(key).map(String::as_str).unwrap_or("")
    }
}

pub fn plan(
    d: &dyn Dialect,
    family: DefinitionFamily,
    scope: &Scope,
    draft: &DefinitionDraft,
) -> Result<Vec<String>, String> {
    if draft.name.trim().is_empty() {
        return Err("A name is required.".into());
    }
    if draft.name.contains('\0') || draft.body.contains('\0') {
        return Err("A definition cannot contain NUL.".into());
    }
    if draft.raw {
        return raw_statements(d, family, scope, draft);
    }
    match family {
        DefinitionFamily::Mysql => mysql(d, scope, draft),
        DefinitionFamily::Postgres => postgres(d, scope, draft),
        DefinitionFamily::Sqlite => sqlite(d, scope, draft),
    }
}

fn raw_statements(
    d: &dyn Dialect,
    family: DefinitionFamily,
    scope: &Scope,
    draft: &DefinitionDraft,
) -> Result<Vec<String>, String> {
    if draft.body.trim() == draft.loaded.trim() {
        return Ok(Vec::new());
    }
    if draft.body.trim().is_empty() {
        return Err("The definition is empty.".into());
    }
    let body = draft.body.trim().to_string();
    match (family, draft.kind) {
        (DefinitionFamily::Mysql | DefinitionFamily::Postgres, DefinitionKind::View) => {
            Ok(vec![ensure_or_replace(&body, "VIEW")])
        }
        (DefinitionFamily::Postgres, DefinitionKind::Function) => {
            Ok(vec![ensure_or_replace(&body, "FUNCTION")])
        }
        (DefinitionFamily::Postgres, DefinitionKind::Procedure) => {
            Ok(vec![ensure_or_replace(&body, "PROCEDURE")])
        }
        (DefinitionFamily::Postgres, DefinitionKind::MaterializedView) => {
            Ok(drop_then(d, scope, draft, "MATERIALIZED VIEW", &body))
        }
        (DefinitionFamily::Sqlite, DefinitionKind::View) => {
            Ok(drop_then(d, scope, draft, "VIEW", &body))
        }
        (
            DefinitionFamily::Mysql,
            DefinitionKind::Function | DefinitionKind::Procedure | DefinitionKind::Event,
        )
        | (DefinitionFamily::Sqlite, DefinitionKind::Index | DefinitionKind::Trigger)
        | (DefinitionFamily::Postgres, DefinitionKind::Type | DefinitionKind::Domain) => {
            let Some(keyword) = plain_drop(draft.kind) else {
                return Ok(vec![body]);
            };
            Ok(drop_then(d, scope, draft, keyword, &body))
        }
        _ => Ok(vec![body]),
    }
}

/// `DROP` the loaded object, then run the edited `CREATE`. Kinds that need a
/// signature (aggregate, operator) stay a plain statement.
fn drop_then(
    d: &dyn Dialect,
    scope: &Scope,
    draft: &DefinitionDraft,
    keyword: &str,
    body: &str,
) -> Vec<String> {
    let Some(prev) = draft.previous.as_deref() else {
        return vec![body.to_string()];
    };
    vec![
        format!("DROP {keyword} IF EXISTS {}", d.qualified(scope, prev)),
        body.to_string(),
    ]
}

fn plain_drop(kind: DefinitionKind) -> Option<&'static str> {
    match kind {
        DefinitionKind::Function => Some("FUNCTION"),
        DefinitionKind::Procedure => Some("PROCEDURE"),
        DefinitionKind::Event => Some("EVENT"),
        DefinitionKind::Index => Some("INDEX"),
        DefinitionKind::Trigger => Some("TRIGGER"),
        DefinitionKind::Type => Some("TYPE"),
        DefinitionKind::Domain => Some("DOMAIN"),
        _ => None,
    }
}

fn ensure_or_replace(sql: &str, keyword: &str) -> String {
    let t = sql.trim_start();
    let replaced = format!("CREATE OR REPLACE {keyword}");
    if starts_with_ignore_ascii(t, &replaced) {
        return sql.trim().to_string();
    }
    let head = format!("CREATE {keyword}");
    if let Some(rest) = strip_ignore_ascii(t, &head) {
        return format!("CREATE OR REPLACE {keyword}{rest}");
    }
    // MySQL `SHOW CREATE VIEW` is `CREATE ALGORITHM=... VIEW`, not `CREATE VIEW`.
    if keyword.eq_ignore_ascii_case("VIEW")
        && let Some(rest) = strip_ignore_ascii(t, "CREATE")
        && has_word(rest, "VIEW")
    {
        return format!("CREATE OR REPLACE{rest}");
    }
    sql.trim().to_string()
}

fn starts_with_ignore_ascii(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn strip_ignore_ascii<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ignore_ascii(text, prefix).then(|| &text[prefix.len()..])
}

fn has_word(text: &str, word: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    let word = word.to_ascii_uppercase();
    upper
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|part| part == word)
}

fn opt<'a>(draft: &'a DefinitionDraft, key: &str) -> &'a str {
    draft.option(key).trim()
}

fn require_body(draft: &DefinitionDraft) -> Result<&str, String> {
    let body = draft.body.trim();
    if body.is_empty() {
        Err("A definition is required.".into())
    } else {
        Ok(body)
    }
}

fn one_of(value: &str, allowed: &[&str], label: &str) -> Result<(), String> {
    if value.is_empty() || allowed.iter().any(|a| a.eq_ignore_ascii_case(value)) {
        Ok(())
    } else {
        Err(format!("Invalid {label}."))
    }
}

fn mysql(d: &dyn Dialect, scope: &Scope, draft: &DefinitionDraft) -> Result<Vec<String>, String> {
    let name = d.qualified(scope, &draft.name);
    match draft.kind {
        DefinitionKind::View => {
            one_of(
                opt(draft, "algorithm"),
                &["UNDEFINED", "MERGE", "TEMPTABLE"],
                "algorithm",
            )?;
            one_of(opt(draft, "security"), &["DEFINER", "INVOKER"], "security")?;
            one_of(
                opt(draft, "check_option"),
                &["CASCADED", "LOCAL"],
                "check option",
            )?;
            let body = require_body(draft)?;
            // A new view must not replace an object that already has this name.
            let mut head = String::from(if draft.previous.is_some() {
                "CREATE OR REPLACE"
            } else {
                "CREATE"
            });
            let algorithm = opt(draft, "algorithm");
            if !algorithm.is_empty() {
                head.push_str(&format!(" ALGORITHM = {}", algorithm.to_ascii_uppercase()));
            }
            let security = opt(draft, "security");
            if !security.is_empty() {
                head.push_str(&format!(" SQL SECURITY {}", security.to_ascii_uppercase()));
            }
            let mut sql = format!("{head} VIEW {name} AS\n{body}");
            let check = opt(draft, "check_option");
            if !check.is_empty() {
                sql.push_str(&format!(
                    " WITH {} CHECK OPTION",
                    check.to_ascii_uppercase()
                ));
            }
            let mut out = Vec::new();
            if let Some(prev) = draft.previous.as_deref().filter(|p| *p != draft.name) {
                out.push(format!(
                    "RENAME TABLE {} TO {}",
                    d.qualified(scope, prev),
                    name
                ));
            }
            out.push(sql);
            Ok(out)
        }
        DefinitionKind::Function | DefinitionKind::Procedure => routine(d, scope, draft, true),
        DefinitionKind::Event => {
            let body = require_body(draft)?;
            let schedule = opt(draft, "schedule");
            if schedule.is_empty() || !single_statement(schedule, d.lex()) {
                return Err("An event needs a schedule.".into());
            }
            one_of(
                opt(draft, "status"),
                &["ENABLE", "DISABLE", "DISABLE ON SLAVE"],
                "status",
            )?;
            let status = opt(draft, "status");
            let status = if status.is_empty() {
                String::new()
            } else {
                format!(" {}", status.to_ascii_uppercase())
            };
            let create = format!("CREATE EVENT {name} ON SCHEDULE {schedule}{status} DO {body}");
            if let Some(prev) = &draft.previous {
                if prev == &draft.name {
                    Ok(vec![format!(
                        "ALTER EVENT {name} ON SCHEDULE {schedule}{status} DO {body}"
                    )])
                } else {
                    Ok(vec![
                        format!("DROP EVENT IF EXISTS {}", d.qualified(scope, prev)),
                        create,
                    ])
                }
            } else {
                Ok(vec![create])
            }
        }
        DefinitionKind::Database => {
            if draft.previous.as_ref().is_some_and(|p| p != &draft.name) {
                return Err("Renaming a database is not supported.".into());
            }
            let ident = d.quote_ident(&draft.name);
            let mut tail = String::new();
            if !opt(draft, "charset").is_empty() {
                tail.push_str(&format!(" CHARACTER SET {}", opt(draft, "charset")));
            }
            if !opt(draft, "collation").is_empty() {
                tail.push_str(&format!(" COLLATE {}", opt(draft, "collation")));
            }
            let verb = if draft.previous.is_some() {
                "ALTER"
            } else {
                "CREATE"
            };
            Ok(vec![format!("{verb} DATABASE {ident}{tail}")])
        }
        DefinitionKind::Tablespace => {
            let file = opt(draft, "datafile");
            if file.is_empty() {
                return Err("A tablespace needs a data file.".into());
            }
            let engine = opt(draft, "engine");
            let engine = if engine.is_empty() {
                String::new()
            } else {
                format!(" ENGINE = {}", engine)
            };
            let create = format!(
                "CREATE TABLESPACE {} ADD DATAFILE {}{engine}",
                d.quote_ident(&draft.name),
                d.string_literal(file)
            );
            replace_drop(d, "TABLESPACE", draft.previous.as_deref(), create)
        }
        DefinitionKind::Sequence => {
            let start = opt(draft, "start");
            let increment = opt(draft, "increment");
            let mut sql = format!("CREATE OR REPLACE SEQUENCE {}", d.quote_ident(&draft.name));
            if !start.is_empty() {
                sql.push_str(&format!(" START WITH {start}"));
            }
            if !increment.is_empty() {
                sql.push_str(&format!(" INCREMENT BY {increment}"));
            }
            Ok(vec![sql])
        }
        _ => Err("This database cannot change that object.".into()),
    }
}

fn routine(
    d: &dyn Dialect,
    scope: &Scope,
    draft: &DefinitionDraft,
    mysql: bool,
) -> Result<Vec<String>, String> {
    let procedure = draft.kind == DefinitionKind::Procedure
        || opt(draft, "routine").eq_ignore_ascii_case("procedure");
    let keyword = if procedure { "PROCEDURE" } else { "FUNCTION" };
    let params = opt(draft, "parameters");
    let params = if params.is_empty() { "()" } else { params };
    let params = if params.starts_with('(') {
        params.to_string()
    } else {
        format!("({params})")
    };
    let body = require_body(draft)?;
    let name = d.qualified(scope, &draft.name);
    let create = if mysql {
        if procedure {
            format!("CREATE PROCEDURE {name} {params}\n{body}")
        } else {
            let returns = opt(draft, "returns");
            if returns.is_empty() {
                return Err("A function needs a return type.".into());
            }
            format!("CREATE FUNCTION {name} {params} RETURNS {returns}\n{body}")
        }
    } else {
        let lang = {
            let lang = opt(draft, "language");
            if lang.is_empty() { "plpgsql" } else { lang }
        };
        let (tag, body) = dollar_quote(body);
        if procedure {
            format!(
                "CREATE OR REPLACE PROCEDURE {name}{params} LANGUAGE {lang} AS {tag}{body}{tag}"
            )
        } else {
            let returns = opt(draft, "returns");
            if returns.is_empty() {
                return Err("A function needs a return type.".into());
            }
            format!(
                "CREATE OR REPLACE FUNCTION {name}{params} RETURNS {returns} LANGUAGE {lang} AS {tag}{body}{tag}"
            )
        }
    };
    if mysql {
        let mut out = Vec::new();
        if let Some(prev) = &draft.previous {
            out.push(format!(
                "DROP {keyword} IF EXISTS {}",
                d.qualified(scope, prev)
            ));
        }
        if draft.previous.as_deref() != Some(draft.name.as_str()) {
            out.push(format!("DROP {keyword} IF EXISTS {name}"));
        }
        out.push(create);
        Ok(out)
    } else if let Some(prev) = draft.previous.as_deref().filter(|p| *p != draft.name) {
        Ok(vec![
            format!("DROP {keyword} {}", d.qualified(scope, prev)),
            create,
        ])
    } else {
        Ok(vec![create])
    }
}

fn dollar_quote(body: &str) -> (String, String) {
    if body.contains("$$") {
        ("$nav$".into(), body.to_string())
    } else {
        ("$$".into(), body.to_string())
    }
}

fn replace_drop(
    d: &dyn Dialect,
    keyword: &str,
    previous: Option<&str>,
    create: String,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    if let Some(prev) = previous {
        out.push(format!("DROP {keyword} {}", d.quote_ident(prev)));
    }
    out.push(create);
    Ok(out)
}

fn postgres(
    d: &dyn Dialect,
    scope: &Scope,
    draft: &DefinitionDraft,
) -> Result<Vec<String>, String> {
    let name = d.qualified(scope, &draft.name);
    match draft.kind {
        DefinitionKind::View => {
            let body = require_body(draft)?;
            let mut out = Vec::new();
            if let Some(prev) = draft.previous.as_deref().filter(|p| *p != draft.name) {
                out.push(format!(
                    "ALTER VIEW {} RENAME TO {}",
                    d.qualified(scope, prev),
                    d.quote_ident(&draft.name)
                ));
            }
            let verb = if draft.previous.is_some() {
                "CREATE OR REPLACE"
            } else {
                "CREATE"
            };
            out.push(format!("{verb} VIEW {name} AS\n{body}"));
            Ok(out)
        }
        DefinitionKind::MaterializedView => {
            let body = require_body(draft)?;
            let data = if opt(draft, "with_data") == "0" {
                "WITH NO DATA"
            } else {
                "WITH DATA"
            };
            let create = format!("CREATE MATERIALIZED VIEW {name} AS\n{body}\n{data}");
            if draft.previous.is_some() {
                Ok(vec![
                    format!(
                        "DROP MATERIALIZED VIEW {}",
                        d.qualified(scope, draft.previous.as_deref().unwrap_or(&draft.name))
                    ),
                    create,
                ])
            } else {
                Ok(vec![create])
            }
        }
        DefinitionKind::Function | DefinitionKind::Procedure => routine(d, scope, draft, false),
        DefinitionKind::Database => postgres_database(d, draft),
        DefinitionKind::Schema => {
            let ident = d.quote_ident(&draft.name);
            if draft.previous.is_none() {
                let mut sql = format!("CREATE SCHEMA {ident}");
                if !opt(draft, "owner").is_empty() {
                    sql.push_str(&format!(
                        " AUTHORIZATION {}",
                        d.quote_ident(opt(draft, "owner"))
                    ));
                }
                return Ok(vec![sql]);
            }
            let mut out = Vec::new();
            let current = draft.previous.as_deref().unwrap_or(&draft.name);
            if current != draft.name {
                out.push(format!(
                    "ALTER SCHEMA {} RENAME TO {ident}",
                    d.quote_ident(current)
                ));
            }
            if !opt(draft, "owner").is_empty() {
                out.push(format!(
                    "ALTER SCHEMA {ident} OWNER TO {}",
                    d.quote_ident(opt(draft, "owner"))
                ));
            }
            Ok(out)
        }
        DefinitionKind::Tablespace => {
            let file = opt(draft, "datafile");
            if draft.previous.is_none() {
                if file.is_empty() {
                    return Err("A tablespace needs a location.".into());
                }
                return Ok(vec![format!(
                    "CREATE TABLESPACE {} LOCATION {}",
                    d.quote_ident(&draft.name),
                    d.string_literal(file)
                )]);
            }
            if !file.is_empty() {
                return Err("Changing a tablespace location is not supported.".into());
            }
            let prev = draft.previous.as_deref().unwrap();
            if prev == draft.name {
                Ok(Vec::new())
            } else {
                Ok(vec![format!(
                    "ALTER TABLESPACE {} RENAME TO {}",
                    d.quote_ident(prev),
                    d.quote_ident(&draft.name)
                )])
            }
        }
        DefinitionKind::Sequence => {
            let start = opt(draft, "start");
            let increment = opt(draft, "increment");
            if draft.previous.is_none() {
                let mut sql = format!("CREATE SEQUENCE {name}");
                if !start.is_empty() {
                    sql.push_str(&format!(" START WITH {start}"));
                }
                if !increment.is_empty() {
                    sql.push_str(&format!(" INCREMENT BY {increment}"));
                }
                return Ok(vec![sql]);
            }
            let mut out = Vec::new();
            if draft.previous.as_deref() != Some(draft.name.as_str()) {
                out.push(format!(
                    "ALTER SEQUENCE {} RENAME TO {}",
                    d.qualified(scope, draft.previous.as_deref().unwrap()),
                    d.quote_ident(&draft.name)
                ));
            }
            if !increment.is_empty() {
                out.push(format!("ALTER SEQUENCE {name} INCREMENT BY {increment}"));
            }
            if !start.is_empty() {
                out.push(format!("ALTER SEQUENCE {name} RESTART WITH {start}"));
            }
            Ok(out)
        }
        DefinitionKind::Type => {
            let body = require_body(draft)?;
            let sql = match opt(draft, "type_kind").to_ascii_lowercase().as_str() {
                "enum" => format!("CREATE TYPE {name} AS ENUM ({body})"),
                "range" => format!("CREATE TYPE {name} AS RANGE ({body})"),
                "base" => format!("CREATE TYPE {name} ({body})"),
                "composite" | "" => format!("CREATE TYPE {name} AS ({body})"),
                _ => return Err("Unknown type kind.".into()),
            };
            let mut out = Vec::new();
            if let Some(prev) = &draft.previous {
                out.push(format!("DROP TYPE {}", d.qualified(scope, prev)));
            }
            out.push(sql);
            Ok(out)
        }
        DefinitionKind::Domain => {
            let base = opt(draft, "base_type");
            if base.is_empty() {
                return Err("A domain needs a base type.".into());
            }
            let mut sql = format!("CREATE DOMAIN {name} AS {base}");
            if !opt(draft, "default").is_empty() {
                sql.push_str(&format!(" DEFAULT {}", opt(draft, "default")));
            }
            if opt(draft, "not_null") == "1" {
                sql.push_str(" NOT NULL");
            }
            if draft.previous.is_some() {
                Ok(vec![
                    format!(
                        "DROP DOMAIN {}",
                        d.qualified(scope, draft.previous.as_deref().unwrap_or(&draft.name))
                    ),
                    sql,
                ])
            } else {
                Ok(vec![sql])
            }
        }
        DefinitionKind::Extension => {
            let ident = d.quote_ident(&draft.name);
            if opt(draft, "action").eq_ignore_ascii_case("drop") {
                Ok(vec![format!("DROP EXTENSION {ident}")])
            } else {
                Ok(vec![format!("CREATE EXTENSION IF NOT EXISTS {ident}")])
            }
        }
        DefinitionKind::ForeignServer => {
            let wrapper = opt(draft, "wrapper");
            if wrapper.is_empty() {
                return Err("A foreign server needs a wrapper.".into());
            }
            let mut sql = format!(
                "CREATE SERVER {} FOREIGN DATA WRAPPER {}",
                d.quote_ident(&draft.name),
                d.quote_ident(wrapper)
            );
            if !draft.body.trim().is_empty() {
                sql.push_str(&format!(" OPTIONS ({})", draft.body.trim()));
            }
            Ok(vec![sql])
        }
        DefinitionKind::Aggregate => {
            let (input, sfunc, stype) = (
                opt(draft, "input_type"),
                opt(draft, "state_func"),
                opt(draft, "state_type"),
            );
            if input.is_empty() || sfunc.is_empty() || stype.is_empty() {
                return Err(
                    "An aggregate needs an input type, a state function, and a state type.".into(),
                );
            }
            Ok(vec![format!(
                "CREATE AGGREGATE {name} ({input}) (SFUNC = {sfunc}, STYPE = {stype})"
            )])
        }
        DefinitionKind::Cast => {
            let (source, target, function) = (
                opt(draft, "source"),
                opt(draft, "target"),
                opt(draft, "function"),
            );
            if source.is_empty() || target.is_empty() || function.is_empty() {
                return Err("A cast needs a source, a target, and a function.".into());
            }
            Ok(vec![format!(
                "CREATE CAST ({source} AS {target}) WITH FUNCTION {function}"
            )])
        }
        DefinitionKind::Operator => {
            let function = opt(draft, "function");
            if function.is_empty() {
                return Err("An operator needs a function.".into());
            }
            let mut parts = Vec::new();
            if !opt(draft, "left").is_empty() {
                parts.push(format!("LEFTARG = {}", opt(draft, "left")));
            }
            if !opt(draft, "right").is_empty() {
                parts.push(format!("RIGHTARG = {}", opt(draft, "right")));
            }
            parts.push(format!("FUNCTION = {function}"));
            Ok(vec![format!(
                "CREATE OPERATOR {} ({})",
                operator_name(d, &draft.name),
                parts.join(", ")
            )])
        }
        DefinitionKind::Collation => {
            let provider = opt(draft, "provider");
            let locale = opt(draft, "locale");
            one_of(provider, &["libc", "icu"], "provider")?;
            if provider.is_empty() || locale.is_empty() {
                return Err("A collation needs a provider and a locale.".into());
            }
            Ok(vec![format!(
                "CREATE COLLATION {name} (PROVIDER = {}, LOCALE = {})",
                provider.to_ascii_lowercase(),
                d.string_literal(locale)
            )])
        }
        _ => Err("This database cannot change that object.".into()),
    }
}

fn postgres_database(d: &dyn Dialect, draft: &DefinitionDraft) -> Result<Vec<String>, String> {
    let ident = d.quote_ident(&draft.name);
    if draft.previous.is_none() {
        let template = {
            let template = opt(draft, "template");
            if template.is_empty() {
                "template1"
            } else {
                template
            }
        };
        let mut sql = format!(
            "CREATE DATABASE {ident} TEMPLATE {}",
            d.quote_ident(template)
        );
        if !opt(draft, "owner").is_empty() {
            sql.push_str(&format!(" OWNER {}", d.quote_ident(opt(draft, "owner"))));
        }
        if !opt(draft, "encoding").is_empty() {
            sql.push_str(&format!(
                " ENCODING {}",
                d.string_literal(opt(draft, "encoding"))
            ));
        }
        return Ok(vec![sql]);
    }
    if !opt(draft, "encoding").is_empty() {
        return Err("Changing a database encoding is not supported.".into());
    }
    let mut out = Vec::new();
    let prev = draft.previous.as_deref().unwrap();
    if prev != draft.name {
        out.push(format!(
            "ALTER DATABASE {} RENAME TO {ident}",
            d.quote_ident(prev)
        ));
    }
    if !opt(draft, "owner").is_empty() {
        out.push(format!(
            "ALTER DATABASE {ident} OWNER TO {}",
            d.quote_ident(opt(draft, "owner"))
        ));
    }
    Ok(out)
}

fn operator_name(d: &dyn Dialect, name: &str) -> String {
    if name.chars().all(|c| "+-*/<>=~!@#%^&|`?".contains(c)) {
        name.to_string()
    } else {
        d.quote_ident(name)
    }
}

fn sqlite(d: &dyn Dialect, scope: &Scope, draft: &DefinitionDraft) -> Result<Vec<String>, String> {
    match draft.kind {
        DefinitionKind::View => {
            let body = require_body(draft)?;
            let mut out = Vec::new();
            if let Some(prev) = &draft.previous {
                out.push(format!("DROP VIEW IF EXISTS {}", d.qualified(scope, prev)));
            }
            out.push(format!(
                "CREATE VIEW {} AS\n{body}",
                d.qualified(scope, &draft.name)
            ));
            Ok(out)
        }
        DefinitionKind::Index | DefinitionKind::Trigger => {
            let table = opt(draft, "table");
            let body = require_body(draft)?;
            let keyword = if draft.kind == DefinitionKind::Index {
                "INDEX"
            } else {
                "TRIGGER"
            };
            let mut out = Vec::new();
            if let Some(prev) = &draft.previous {
                out.push(format!(
                    "DROP {keyword} IF EXISTS {}",
                    d.qualified(scope, prev)
                ));
            }
            if body.to_ascii_uppercase().starts_with("CREATE ") {
                out.push(body.to_string());
            } else if draft.kind == DefinitionKind::Index {
                if table.is_empty() {
                    return Err("An index needs a table.".into());
                }
                out.push(format!(
                    "CREATE INDEX {} ON {} ({body})",
                    d.quote_ident(&draft.name),
                    d.qualified(scope, table)
                ));
            } else {
                if table.is_empty() {
                    return Err("A trigger needs a table.".into());
                }
                let timing = opt(draft, "timing");
                let event = opt(draft, "events");
                if timing.is_empty() || event.is_empty() {
                    return Err("A trigger needs timing and an event.".into());
                }
                out.push(format!(
                    "CREATE TRIGGER {} {timing} {event} ON {} BEGIN {body}; END",
                    d.quote_ident(&draft.name),
                    d.qualified(scope, table)
                ));
            }
            Ok(out)
        }
        DefinitionKind::Attach => {
            let path = opt(draft, "path");
            if path.is_empty() {
                return Err("An attached database needs a file path.".into());
            }
            Ok(vec![format!(
                "ATTACH DATABASE {} AS {}",
                d.string_literal(path),
                d.quote_ident(&draft.name)
            )])
        }
        DefinitionKind::Detach => {
            if draft.name.eq_ignore_ascii_case("main") {
                return Err("The main database cannot be detached.".into());
            }
            Ok(vec![format!(
                "DETACH DATABASE {}",
                d.quote_ident(&draft.name)
            )])
        }
        _ => Err("This database cannot change that object.".into()),
    }
}

/// MariaDB 10.3+ reports its own version, sometimes behind a `5.5.5-` compatibility prefix.
pub fn mariadb_sequences(driver: &str, version: &str) -> bool {
    driver == "mariadb" && version_at_least(version, 10, 3)
}

fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    let version = version.strip_prefix("5.5.5-").unwrap_or(version);
    let mut parts = version.split(|c: char| !c.is_ascii_digit());
    let got_major: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let got_minor: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    (got_major, got_minor) >= (major, minor)
}

/// True when `sql` is a single statement, optionally ending in one semicolon.
pub fn single_statement(sql: &str, lex: Lex) -> bool {
    let semis = span::separators(sql, lex, b';');
    match semis.as_slice() {
        [] => true,
        [at] => sql[at + 1..].trim().is_empty(),
        _ => false,
    }
}
