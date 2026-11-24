//! Connection configuration model: connections.json from docs/design/11-connection.md §6.
//! Every struct keeps unknown fields, so files written by newer versions stay compatible.

use std::ops::{Deref, DerefMut};

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

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    /// `DriverRegistry` key: mysql, mariadb, postgres, sqlite; plugin drivers carry the plugin prefix
    pub driver: String,
    pub group: Option<GroupId>,
    pub color: Option<String>,
    /// Tables/data options shared by every profile
    pub databases: Databases,
    pub remark: String,
    /// Starred in Manage Connections; the navigator's quick filter can show only these
    pub starred: bool,
    /// Hidden in Manage Connections: kept but not shown in the navigator
    pub hidden: bool,
    /// Every profile's settings, including the active one at `active`. The active profile is
    /// what the form edits and what opening the connection connects with (F-CONN-021).
    pub profiles: Vec<Profile>,
    /// Index of the active profile in `profiles`
    pub active: usize,
    pub extra: Map<String, Value>,
}

/// One named set of connection settings, e.g. for another database user. Secrets stay encrypted
/// for the connection's id, so they keep working when another profile becomes active.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub general: General,
    pub advanced: Advanced,
    pub ssl: Ssl,
    pub ssh: Ssh,
}

// On-disk compatibility: `connections.json` keeps its historical layout - the active profile's
// settings at the top level, its name in `profile`, and the other profiles in `profiles` - so an
// older build can still read a file this one writes. The deserializer also accepts the newer
// shape (`profiles` holds every profile and `active` names one), which a later release can switch
// to without another migration.
impl Serialize for Connection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct OnDisk<'a> {
            id: &'a ConnectionId,
            name: &'a str,
            driver: &'a str,
            group: &'a Option<GroupId>,
            color: &'a Option<String>,
            general: &'a General,
            advanced: &'a Advanced,
            databases: &'a Databases,
            ssl: &'a Ssl,
            ssh: &'a Ssh,
            remark: &'a str,
            starred: bool,
            hidden: bool,
            profile: &'a str,
            profiles: Vec<&'a Profile>,
            #[serde(flatten)]
            extra: &'a Map<String, Value>,
        }
        let active = self.active();
        OnDisk {
            id: &self.id,
            name: &self.name,
            driver: &self.driver,
            group: &self.group,
            color: &self.color,
            general: &active.general,
            advanced: &active.advanced,
            databases: &self.databases,
            ssl: &active.ssl,
            ssh: &active.ssh,
            remark: &self.remark,
            starred: self.starred,
            hidden: self.hidden,
            profile: &active.name,
            profiles: self
                .profiles
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != self.active)
                .map(|(_, profile)| profile)
                .collect(),
            extra: &self.extra,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Connection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            id: ConnectionId,
            name: String,
            driver: String,
            #[serde(default)]
            group: Option<GroupId>,
            #[serde(default)]
            color: Option<String>,
            #[serde(default)]
            general: Option<General>,
            #[serde(default)]
            advanced: Option<Advanced>,
            #[serde(default)]
            databases: Databases,
            #[serde(default)]
            ssl: Option<Ssl>,
            #[serde(default)]
            ssh: Option<Ssh>,
            #[serde(default)]
            remark: String,
            #[serde(default)]
            starred: bool,
            #[serde(default)]
            hidden: bool,
            #[serde(default)]
            profile: String,
            #[serde(default)]
            profiles: Vec<Profile>,
            #[serde(default)]
            active: usize,
            #[serde(flatten)]
            extra: Map<String, Value>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let legacy = raw.general.is_some()
            || raw.advanced.is_some()
            || raw.ssl.is_some()
            || raw.ssh.is_some();
        let (profiles, active) = if legacy {
            // The active settings sit at the top level; `profiles` holds the inactive ones.
            let mut profiles = raw.profiles;
            profiles.insert(
                0,
                Profile {
                    name: raw.profile,
                    general: raw.general.unwrap_or_default(),
                    advanced: raw.advanced.unwrap_or_default(),
                    ssl: raw.ssl.unwrap_or_default(),
                    ssh: raw.ssh.unwrap_or_default(),
                },
            );
            (profiles, 0)
        } else {
            let mut profiles = raw.profiles;
            if profiles.is_empty() {
                profiles.push(Profile::default());
            }
            let active = raw.active.min(profiles.len() - 1);
            (profiles, active)
        };
        Ok(Connection {
            id: raw.id,
            name: raw.name,
            driver: raw.driver,
            group: raw.group,
            color: raw.color,
            databases: raw.databases,
            remark: raw.remark,
            starred: raw.starred,
            hidden: raw.hidden,
            profiles,
            active,
            extra: raw.extra,
        })
    }
}

/// The connection's settings are its active profile: `connection.general` and friends keep
/// working, but every field now lives in `profiles[active]`.
impl Deref for Connection {
    type Target = Profile;

    fn deref(&self) -> &Profile {
        self.active()
    }
}

impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut Profile {
        self.active_mut()
    }
}

/// Navicat's connection defaults. Kept here (not in `DriverInfo`) because
/// `Connection::new` lives in core; drivers reuse the port through this table.
pub struct DriverDefaults {
    pub host: &'static str,
    pub port: Option<u16>,
    pub user: &'static str,
    pub database: Option<&'static str>,
    /// The connection is a file path. The suggested name is the file stem.
    pub file_based: bool,
    pub supports_network: bool,
}

pub fn driver_defaults(driver: &str) -> DriverDefaults {
    match driver {
        "postgres" => DriverDefaults {
            host: "localhost",
            port: Some(5432),
            user: "postgres",
            database: Some("postgres"),
            file_based: false,
            supports_network: true,
        },
        "dameng" => DriverDefaults {
            host: "localhost",
            port: Some(5236),
            user: "SYSDBA",
            database: None,
            file_based: false,
            supports_network: true,
        },
        "sqlite" => DriverDefaults {
            host: "",
            port: None,
            user: "",
            database: None,
            file_based: true,
            supports_network: false,
        },
        _ => DriverDefaults {
            host: "localhost",
            port: Some(3306),
            user: "root",
            database: None,
            file_based: false,
            supports_network: true,
        },
    }
}

impl Connection {
    /// The settings in force: the active profile.
    pub fn active(&self) -> &Profile {
        &self.profiles[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Profile {
        &mut self.profiles[self.active]
    }

    /// Name of the active profile ("" is the default one).
    pub fn active_profile_name(&self) -> &str {
        &self.active().name
    }

    /// Runs `f` with profile `ix` active (an index into `profiles`). The previous profile is
    /// restored even when `f` panics, and no settings are copied or swapped.
    pub fn with_active_profile<R>(&mut self, ix: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        let ix = ix.min(self.profiles.len().saturating_sub(1));
        if ix == self.active {
            return f(self);
        }
        let previous = std::mem::replace(&mut self.active, ix);
        struct Restore<'a> {
            connection: &'a mut Connection,
            previous: usize,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.connection.active = self.previous;
            }
        }
        let restore = Restore {
            connection: self,
            previous,
        };
        f(&mut *restore.connection)
    }

    /// Makes the named profile active; false when no profile has that name.
    pub fn switch_profile(&mut self, name: &str) -> bool {
        let Some(ix) = self.profiles.iter().position(|p| p.name == name) else {
            return false;
        };
        self.active = ix;
        true
    }

    /// Adds a profile that starts as a copy of the active settings, and makes it active.
    pub fn add_profile(&mut self, name: String) {
        let mut profile = self.active().clone();
        profile.name = name;
        self.profiles.push(profile);
        self.active = self.profiles.len() - 1;
    }

    /// Drops the active profile and activates the first remaining one. The last profile stays:
    /// a connection always has settings.
    pub fn remove_active_profile(&mut self) -> bool {
        if self.profiles.len() <= 1 {
            return false;
        }
        self.profiles.remove(self.active);
        self.active = 0;
        true
    }

    /// All profile names, the active one first.
    pub fn profile_names(&self) -> impl Iterator<Item = &str> {
        let active = self.active;
        std::iter::once(self.profiles[active].name.as_str()).chain(
            self.profiles
                .iter()
                .enumerate()
                .filter(move |(ix, _)| *ix != active)
                .map(|(_, profile)| profile.name.as_str()),
        )
    }

    /// Navicat's defaults for the given driver (11-connection.md §2).
    pub fn new(driver: &str) -> Self {
        let defaults = driver_defaults(driver);
        let (port, user, database) = (
            defaults.port,
            defaults.user,
            defaults.database.map(str::to_string),
        );
        let host = defaults.host;
        let mut conn = Self {
            id: new_connection_id(),
            name: String::new(),
            driver: driver.to_string(),
            group: None,
            color: None,
            databases: Databases::default(),
            remark: String::new(),
            starred: false,
            hidden: false,
            profiles: vec![Profile {
                name: String::new(),
                general: General {
                    host: host.into(),
                    port,
                    user: user.into(),
                    database,
                    save_password: true,
                    ..General::default()
                },
                ..Profile::default()
            }],
            active: 0,
            extra: Map::new(),
        };
        conn.name = conn.suggested_name();
        conn
    }

    /// Navicat's default connection name: host_port; a file database uses the file name.
    pub fn suggested_name(&self) -> String {
        if driver_defaults(&self.driver).file_based {
            return self
                .general
                .file
                .as_deref()
                .and_then(|f| std::path::Path::new(f).file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.driver.clone());
        }
        match self.general.port {
            Some(port) => format!("{}_{}", self.general.host, port),
            None => self.general.host.clone(),
        }
    }

    pub fn supports_network(&self) -> bool {
        driver_defaults(&self.driver).supports_network
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

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
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
        self.into()
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
    /// Verify the SSH server key against `~/.ssh/known_hosts` (trust on first use).
    pub verify_host_key: bool,
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
            verify_host_key: true,
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

    /// The field's slot in one profile. A `&Connection` works too, through its `Deref` to the
    /// active profile.
    pub fn slot(self, profile: &Profile) -> &Option<Secret> {
        match self {
            Self::Password => &profile.general.password,
            Self::SshPassword => &profile.ssh.password,
            Self::SshPassphrase => &profile.ssh.passphrase,
        }
    }

    pub fn slot_mut(self, profile: &mut Profile) -> &mut Option<Secret> {
        match self {
            Self::Password => &mut profile.general.password,
            Self::SshPassword => &mut profile.ssh.password,
            Self::SshPassphrase => &mut profile.ssh.passphrase,
        }
    }

    /// The "save password" switch of this field.
    pub fn saved_mut(self, profile: &mut Profile) -> &mut bool {
        match self {
            Self::Password => &mut profile.general.save_password,
            Self::SshPassword => &mut profile.ssh.save_password,
            Self::SshPassphrase => &mut profile.ssh.save_passphrase,
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
    fn the_active_profile_is_restored_when_the_operation_panics() {
        let mut connection = Connection::new("mysql");
        connection.add_profile("other".into());
        assert!(connection.switch_profile(""));
        let before = connection.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connection.with_active_profile(1, |_| panic!("fixture"));
        }));
        assert!(result.is_err());
        assert_eq!(connection, before, "the active profile was not restored");
    }

    #[test]
    fn driver_defaults_match_the_connection_constructor() {
        let pg = Connection::new("postgres");
        assert_eq!(driver_defaults("postgres").port, pg.general.port);
        let lite = Connection::new("sqlite");
        assert_eq!(lite.general.host, "");
        assert_eq!(lite.general.port, None);
    }

    #[test]
    fn ssl_mode_keys_match_their_serialized_names() {
        for mode in SslMode::ALL {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
        }
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
        assert_eq!(
            (c.active_profile_name(), c.general.user.as_str()),
            ("", "root")
        );
        assert!(!c.switch_profile("missing"));
        assert!(c.switch_profile("readonly"));
        assert_eq!(c.general.user, "reader");
        assert_eq!(c.profiles.len(), 2);
        assert!(c.remove_active_profile());
        assert_eq!(
            (
                c.active_profile_name(),
                c.general.user.as_str(),
                c.profiles.len()
            ),
            ("", "root", 1)
        );
        assert!(!c.remove_active_profile(), "the last profile stays");
    }

    #[test]
    fn many_profiles_keep_their_own_settings() {
        let mut c = Connection::new("postgres");
        for ix in 0..8 {
            c.add_profile(format!("p{ix}"));
            c.general.user = format!("u{ix}");
        }
        assert_eq!(
            c.profiles.len(),
            9,
            "seven or more profiles plus the default"
        );
        assert_eq!(c.profile_names().count(), 9);
        for ix in 0..8 {
            assert!(c.switch_profile(&format!("p{ix}")));
            assert_eq!(c.general.user, format!("u{ix}"));
        }
        // Duplicate and empty names are the form's business to reject; the model keeps them.
        c.add_profile("p0".into());
        c.add_profile(String::new());
        assert_eq!(c.profile_names().filter(|name| *name == "p0").count(), 2);
        assert!(c.switch_profile(""));
        assert_eq!(c.active_profile_name(), "");
        // Every profile still has its own settings after all those switches.
        assert!(c.switch_profile("p3"));
        assert_eq!(c.general.user, "u3");
    }

    #[test]
    fn the_old_on_disk_layout_loads_and_writes_back() {
        let json = r#"{"id":"c-1","name":"a","driver":"mysql",
            "general":{"host":"h","user":"root","save_password":true},
            "profile":"other",
            "profiles":[{"name":"work","general":{"host":"w","user":"worker"}}]}"#;
        let conn: Connection = serde_json::from_str(json).unwrap();
        assert_eq!(conn.profile_names().collect::<Vec<_>>(), ["other", "work"]);
        assert_eq!(conn.general.host, "h");
        // Written back in the same layout, so an older build can still read the file.
        let back = serde_json::to_value(&conn).unwrap();
        assert!(back.get("general").is_some());
        assert_eq!(back["profile"], "other");
        assert_eq!(back["profiles"][0]["name"], "work");
        let reread: Connection = serde_json::from_value(back).unwrap();
        assert_eq!(reread, conn);
    }

    #[test]
    fn the_profiles_and_active_layout_loads_too() {
        let json = r#"{"id":"c-1","name":"a","driver":"mysql","active":1,
            "profiles":[{"name":"","general":{"host":"one"}},
                        {"name":"work","general":{"host":"two"}}]}"#;
        let mut conn: Connection = serde_json::from_str(json).unwrap();
        assert_eq!(conn.active_profile_name(), "work");
        assert_eq!(conn.general.host, "two");
        assert!(conn.switch_profile(""));
        assert_eq!(conn.general.host, "one");
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
