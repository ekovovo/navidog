//! 把保存的连接配置与解密后的秘密换算成驱动连接参数。

use std::path::PathBuf;
use std::time::Duration;

use navidog_core::connection::{Connection, SslMode};

#[derive(Clone, Debug, Default)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub file: Option<PathBuf>,
    pub create_new: bool,
    pub attached: Vec<(String, PathBuf)>,
    pub connect_timeout: Option<Duration>,
    pub compression: bool,
    pub socket: Option<String>,
    /// PostgreSQL 连接参数（libpq `key=value` 形式）
    pub params: String,
    pub initial_queries: String,
    pub ssl: Option<SslParams>,
    pub ssh: Option<SshParams>,
    /// 经 SSH 隧道连接时，TLS 证书校验使用的原始主机名
    pub tls_host: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SslParams {
    pub mode: SslMode,
    pub client_key: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub ca_cert: Option<PathBuf>,
    pub verify_server_cert: bool,
    pub cipher: Option<String>,
    pub crl: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub private_key: Option<PathBuf>,
    pub passphrase: Option<String>,
}

/// 解密后的秘密字段。
#[derive(Clone, Debug, Default)]
pub struct Secrets {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
}

fn non_empty(s: &Option<String>) -> Option<PathBuf> {
    s.as_deref().filter(|v| !v.is_empty()).map(expand_home)
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(rest),
        None => PathBuf::from(path),
    }
}

impl ConnectParams {
    pub fn from_connection(conn: &Connection, secrets: Secrets) -> Self {
        let g = &conn.general;
        let a = &conn.advanced;
        let default_port = if conn.driver == "postgres" {
            5432
        } else {
            3306
        };
        let ssl = (conn.ssl.enabled && conn.supports_network()).then(|| SslParams {
            mode: conn.ssl.mode,
            client_key: conn
                .ssl
                .use_auth
                .then(|| non_empty(&conn.ssl.client_key))
                .flatten(),
            client_cert: conn
                .ssl
                .use_auth
                .then(|| non_empty(&conn.ssl.client_cert))
                .flatten(),
            ca_cert: conn
                .ssl
                .use_auth
                .then(|| non_empty(&conn.ssl.ca_cert))
                .flatten(),
            verify_server_cert: conn.ssl.verify_server_cert,
            cipher: conn.ssl.cipher.clone().filter(|c| !c.is_empty()),
            crl: conn
                .ssl
                .use_auth
                .then(|| non_empty(&conn.ssl.crl))
                .flatten(),
        });
        let s = &conn.ssh;
        let ssh = (s.enabled && conn.supports_network()).then(|| SshParams {
            host: s.host.clone(),
            port: s.port,
            user: s.user.clone(),
            password: s
                .auth
                .uses_password()
                .then(|| secrets.ssh_password.clone())
                .flatten(),
            private_key: s
                .auth
                .uses_key()
                .then(|| non_empty(&s.private_key))
                .flatten(),
            passphrase: s
                .auth
                .uses_key()
                .then(|| secrets.ssh_passphrase.clone())
                .flatten(),
        });
        Self {
            host: g.host.clone(),
            port: g.port.unwrap_or(default_port),
            user: g.user.clone(),
            password: secrets.password,
            database: g.database.clone().filter(|d| !d.is_empty()),
            file: non_empty(&g.file),
            create_new: g.create_new,
            attached: conn
                .databases
                .attached
                .iter()
                .map(|d| (d.name.clone(), expand_home(&d.file)))
                .collect(),
            connect_timeout: a
                .connect_timeout_secs
                .map(|s| Duration::from_secs(s as u64)),
            compression: a.compression,
            socket: a.socket.clone().filter(|s| !s.is_empty()),
            params: a.params.clone(),
            initial_queries: a.initial_queries.clone(),
            ssl,
            ssh,
            tls_host: None,
        }
    }

    pub fn timeout(&self) -> Duration {
        self.connect_timeout.unwrap_or(Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_core::connection::SshAuth;

    #[test]
    fn maps_connection_and_drops_unused_secrets() {
        let mut conn = Connection::new("postgres");
        conn.ssh.enabled = true;
        conn.ssh.host = "bastion".into();
        conn.ssh.auth = SshAuth::PublicKey;
        conn.ssh.private_key = Some("~/.ssh/id_ed25519".into());
        let p = ConnectParams::from_connection(
            &conn,
            Secrets {
                password: Some("pw".into()),
                ssh_password: Some("ignored".into()),
                ssh_passphrase: Some("pp".into()),
            },
        );
        assert_eq!((p.port, p.database.as_deref()), (5432, Some("postgres")));
        let ssh = p.ssh.unwrap();
        assert_eq!(ssh.password, None);
        assert_eq!(ssh.passphrase.as_deref(), Some("pp"));
        assert!(ssh.private_key.unwrap().ends_with(".ssh/id_ed25519"));
        assert!(p.ssl.is_none());

        let lite = Connection::new("sqlite");
        let p = ConnectParams::from_connection(&lite, Secrets::default());
        assert!(p.ssh.is_none() && p.file.is_none());
    }
}
