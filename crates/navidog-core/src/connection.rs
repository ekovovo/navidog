//! Connection configuration model: connections.json from docs/design/11-connection.md §6.
//! Every struct keeps unknown fields, so files written by newer versions stay compatible.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::crypto::Secret;

pub type ConnectionId = String;
pub type GroupId = String;

pub fn new_connection_id() -> ConnectionId {
    format!("c-{}", uuid::Uuid::new_v4().simple())
}

pub fn new_group_id() -> GroupId {
    format!("g-{}", uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub parent: Option<GroupId>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    /// `DriverRegistry` key: mysql, mariadb, postgres, sqlite; plugin drivers carry the plugin prefix
    pub driver: String,
    #[serde(default)]
    pub group: Option<GroupId>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub advanced: Advanced,
    #[serde(default)]
    pub databases: Databases,
    #[serde(default)]
    pub ssl: Ssl,
    #[serde(default)]
    pub ssh: Ssh,
    #[serde(default)]
    pub remark: String,
    /// Starred in Manage Connections; the navigator's quick filter can show only these
    #[serde(default)]
    pub starred: bool,
    /// Hidden in Manage Connections: kept but not shown in the navigator
    #[serde(default)]
    pub hidden: bool,
    /// Name of the active profile ("" is the default one); its settings are the fields above
    #[serde(default)]
    pub profile: String,
    /// The inactive profiles (F-CONN-021)
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A named set of connection settings, e.g. for another database user. Secrets stay encrypted for
/// the connection's id, so they stay valid when a profile trades places with the active settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub general: General,
    pub advanced: Advanced,
    pub ssl: Ssl,
    pub ssh: Ssh,
}

impl Connection {
    /// Trades the active settings with `profiles[ix]`; swapping twice restores them.
    pub fn swap_profile(&mut self, ix: usize) {
        let p = &mut self.profiles[ix];
        std::mem::swap(&mut self.profile, &mut p.name);
        std::mem::swap(&mut self.general, &mut p.general);
        std::mem::swap(&mut self.advanced, &mut p.advanced);
        std::mem::swap(&mut self.ssl, &mut p.ssl);
        std::mem::swap(&mut self.ssh, &mut p.ssh);
    }

    pub fn switch_profile(&mut self, name: &str) -> bool {
        let Some(ix) = self.profiles.iter().position(|p| p.name == name) else {
            return false;
        };
        self.swap_profile(ix);
        true
    }

    /// Adds a profile that starts as a copy of the active settings, and makes it active.
    pub fn add_profile(&mut self, name: String) {
        let old = Profile {
            name: std::mem::replace(&mut self.profile, name),
            general: self.general.clone(),
            advanced: self.advanced.clone(),
            ssl: self.ssl.clone(),
            ssh: self.ssh.clone(),
        };
        self.profiles.push(old);
    }

    /// Drops the active profile and activates the first remaining one.
    pub fn remove_active_profile(&mut self) -> bool {
        if self.profiles.is_empty() {
            return false;
        }
        self.swap_profile(0);
        self.profiles.remove(0);
        true
    }

    /// All profile names, the active one first.
    pub fn profile_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.profile.as_str()).chain(self.profiles.iter().map(|p| p.name.as_str()))
    }

    /// Navicat's defaults for the given driver (11-connection.md §2).
    pub fn new(driver: &str) -> Self {
        let (port, user, database) = match driver {
            "postgres" => (Some(5432), "postgres", Some("postgres".to_string())),
            "sqlite" => (None, "", None),
            _ => (Some(3306), "root", None),
        };
        let host = if driver == "sqlite" { "" } else { "localhost" };
        let mut conn = Self {
            id: new_connection_id(),
            name: String::new(),
            driver: driver.to_string(),
            group: None,
            color: None,
            general: General {
                host: host.into(),
                port,
                user: user.into(),
                database,
                save_password: true,
                ..General::default()
            },
            advanced: Advanced::default(),
            databases: Databases::default(),
            ssl: Ssl::default(),
            ssh: Ssh::default(),
            remark: String::new(),
            starred: false,
            hidden: false,
            profile: String::new(),
            profiles: Vec::new(),
            extra: Map::new(),
        };
        conn.name = conn.suggested_name();
        conn
    }

    /// Navicat's default connection name: host_port; SQLite uses the file name.
    pub fn suggested_name(&self) -> String {
        if self.driver == "sqlite" {
            return self
                .general
                .file
                .as_deref()
                .and_then(|f| std::path::Path::new(f).file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sqlite".into());
        }
        match self.general.port {
            Some(port) => format!("{}_{}", self.general.host, port),
            None => self.general.host.clone(),
        }
    }

    pub fn supports_network(&self) -> bool {
        self.driver != "sqlite"
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    /// PostgreSQL initial database
    pub database: Option<String>,
    /// SQLite database file
    pub file: Option<String>,
    /// SQLite: create a new file when opening
    pub create_new: bool,
    pub save_password: bool,
    pub password: Option<Secret>,
    /// PostgreSQL multiple nodes: servers tried in order after `host`; empty for a single node
    pub members: Vec<Member>,
    /// PostgreSQL multiple nodes: which kind of server to settle on
    pub host_preference: HostPreference,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub host: String,
    pub port: Option<u16>,
}

impl Member {
    /// `host`, `host:port`, `[v6]` or `[v6]:port`.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (host, port) = match line.strip_prefix('[') {
            Some(rest) => {
                let (host, after) = rest.split_once(']')?;
                (host, after.strip_prefix(':'))
            }
            None => match line.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h, Some(p)),
                _ => (line, None),
            },
        };
        let port = match port {
            Some(p) => Some(p.parse().ok()?),
            None => None,
        };
        (!host.is_empty()).then(|| Self {
            host: host.into(),
            port,
        })
    }
}

impl std::fmt::Display for Member {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match self.port {
            Some(p) => write!(f, "{host}:{p}"),
            None => f.write_str(&host),
        }
    }
}

/// libpq `target_session_attrs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostPreference {
    #[default]
    Any,
    ReadWrite,
    ReadOnly,
}

impl HostPreference {
    pub const ALL: [Self; 3] = [Self::Any, Self::ReadWrite, Self::ReadOnly];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Advanced {
    pub auto_connect: bool,
    /// None disables keepalive
    pub keepalive_secs: Option<u32>,
    pub connect_timeout_secs: Option<u32>,
    pub read_timeout_secs: Option<u32>,
    pub write_timeout_secs: Option<u32>,
    pub compression: bool,
    pub socket: Option<String>,
    pub session_limit: Option<u32>,
    pub charset: String,
    pub encoding: String,
    /// PostgreSQL connection parameters, space-separated `key=value` pairs
    pub params: String,
    /// Run on every new session; multiple statements are separated by semicolons
    pub initial_queries: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            auto_connect: false,
            keepalive_secs: None,
            connect_timeout_secs: Some(30),
            read_timeout_secs: None,
            write_timeout_secs: None,
            compression: false,
            socket: None,
            session_limit: None,
            charset: "auto".into(),
            encoding: "utf8".into(),
            params: String::new(),
            initial_queries: String::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Databases {
    pub custom_list: bool,
    pub items: Vec<DatabaseItem>,
    /// Databases attached to SQLite
    pub attached: Vec<AttachedDatabase>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DatabaseItem {
    pub name: String,
    #[serde(default)]
    pub auto_open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttachedDatabase {
    pub name: String,
    pub file: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Allow,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 5] = [
        Self::Allow,
        Self::Prefer,
        Self::Require,
        Self::VerifyCa,
        Self::VerifyFull,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ssl {
    pub enabled: bool,
    pub mode: SslMode,
    pub use_auth: bool,
    pub client_key: Option<String>,
    pub client_cert: Option<String>,
    pub ca_cert: Option<String>,
    pub verify_server_cert: bool,
    pub cipher: Option<String>,
    pub crl: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuth {
    #[default]
    Password,
    PublicKey,
    Both,
}

impl SshAuth {
    pub fn uses_password(self) -> bool {
        matches!(self, Self::Password | Self::Both)
    }

    pub fn uses_key(self) -> bool {
        matches!(self, Self::PublicKey | Self::Both)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ssh {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub save_password: bool,
    pub password: Option<Secret>,
    pub private_key: Option<String>,
    pub save_passphrase: bool,
    pub passphrase: Option<Secret>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Ssh {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: SshAuth::Password,
            save_password: true,
            password: None,
            private_key: None,
            save_passphrase: true,
            passphrase: None,
            extra: Map::new(),
        }
    }
}

/// Secret field name, also used as the associated data for encryption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretField {
    Password,
    SshPassword,
    SshPassphrase,
}

impl SecretField {
    pub fn key(self) -> &'static str {
        match self {
            Self::Password => "general.password",
            Self::SshPassword => "ssh.password",
            Self::SshPassphrase => "ssh.passphrase",
        }
    }

    pub fn slot(self, conn: &Connection) -> &Option<Secret> {
        match self {
            Self::Password => &conn.general.password,
            Self::SshPassword => &conn.ssh.password,
            Self::SshPassphrase => &conn.ssh.passphrase,
        }
    }

    pub fn slot_mut(self, conn: &mut Connection) -> &mut Option<Secret> {
        match self {
            Self::Password => &mut conn.general.password,
            Self::SshPassword => &mut conn.ssh.password,
            Self::SshPassphrase => &mut conn.ssh.passphrase,
        }
    }

    pub const ALL: [SecretField; 3] = [Self::Password, Self::SshPassword, Self::SshPassphrase];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navicat_defaults_per_driver() {
        let my = Connection::new("mysql");
        assert_eq!(
            (my.general.port, my.general.user.as_str()),
            (Some(3306), "root")
        );
        assert!(my.general.save_password);
        assert_eq!(my.name, "localhost_3306");
        let pg = Connection::new("postgres");
        assert_eq!(pg.general.database.as_deref(), Some("postgres"));
        let mut lite = Connection::new("sqlite");
        lite.general.file = Some("/tmp/app.db".into());
        assert_eq!(lite.suggested_name(), "app");
        assert!(!lite.supports_network());
    }

    #[test]
    fn members_parse_and_print() {
        let m = |s| Member::parse(s).map(|m| (m.host, m.port));
        assert_eq!(m(" db1:6543 "), Some(("db1".into(), Some(6543))));
        assert_eq!(m("db2"), Some(("db2".into(), None)));
        assert_eq!(m("[::1]:5433"), Some(("::1".into(), Some(5433))));
        assert_eq!(
            m("fe80::1"),
            Some(("fe80::1".into(), None)),
            "a bare IPv6 address has no port"
        );
        assert_eq!(m("db:x"), None);
        assert_eq!(m(""), None);
        let v6 = Member {
            host: "::1".into(),
            port: Some(5433),
        };
        assert_eq!(Member::parse(&v6.to_string()), Some(v6));
    }

    #[test]
    fn profiles_trade_places_with_active_settings() {
        let mut c = Connection::new("mysql");
        c.add_profile("readonly".into());
        c.general.user = "reader".into();
        assert_eq!(c.profile_names().collect::<Vec<_>>(), ["readonly", ""]);
        assert!(c.switch_profile(""));
        assert_eq!((c.profile.as_str(), c.general.user.as_str()), ("", "root"));
        assert!(!c.switch_profile("missing"));
        assert!(c.switch_profile("readonly"));
        assert_eq!(c.general.user, "reader");
        assert!(c.remove_active_profile());
        assert_eq!(
            (
                c.profile.as_str(),
                c.general.user.as_str(),
                c.profiles.len()
            ),
            ("", "root", 0)
        );
        assert!(!c.remove_active_profile());
    }

    #[test]
    fn unknown_fields_round_trip() {
        let json = r#"{"id":"c-1","name":"a","driver":"mysql","future":{"x":1},
            "ssh":{"enabled":true,"jump":"bastion"}}"#;
        let conn: Connection = serde_json::from_str(json).unwrap();
        assert!(conn.ssh.enabled);
        assert_eq!(conn.ssh.port, 22);
        let back = serde_json::to_value(&conn).unwrap();
        assert_eq!(back["future"]["x"], 1);
        assert_eq!(back["ssh"]["jump"], "bastion");
    }
}
