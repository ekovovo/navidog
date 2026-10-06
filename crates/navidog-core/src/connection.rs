//! 连接配置模型，对应 docs/design/11-connection.md 第 6 节的 connections.json。
//! 每个结构都保留未知字段，兼容较新版本写入的文件。

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
    /// `DriverRegistry` 的键：mysql、mariadb、postgres、sqlite，插件驱动带插件前缀
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Connection {
    /// 按驱动给出 Navicat 的默认值（11-connection.md 第 2 节）。
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
            extra: Map::new(),
        };
        conn.name = conn.suggested_name();
        conn
    }

    /// Navicat 的默认连接名：主机_端口；SQLite 取文件名。
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
    /// PostgreSQL 初始数据库
    pub database: Option<String>,
    /// SQLite 数据库文件
    pub file: Option<String>,
    /// SQLite：打开时新建文件
    pub create_new: bool,
    pub save_password: bool,
    pub password: Option<Secret>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Advanced {
    pub auto_connect: bool,
    /// None 表示不启用保持连接
    pub keepalive_secs: Option<u32>,
    pub connect_timeout_secs: Option<u32>,
    pub read_timeout_secs: Option<u32>,
    pub write_timeout_secs: Option<u32>,
    pub compression: bool,
    pub socket: Option<String>,
    pub session_limit: Option<u32>,
    pub charset: String,
    pub encoding: String,
    /// PostgreSQL 连接参数，`key=value` 以空格分隔
    pub params: String,
    /// 每个新会话执行，多条以分号分隔
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
    /// SQLite 附加的数据库
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

/// 秘密字段名，同时用作加密的 associated data。
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
