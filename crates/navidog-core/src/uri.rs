//! navidog URIs (F-APP-014, F-APP-015) name an object across machines:
//! `navidog://open?conn=<name>&host=<host>&port=<port>&db=<database>&schema=<schema>&kind=<kind>&name=<object>`.
//! The format is our own and not compatible with Navicat URIs. Standard database URIs
//! (`mysql://user:password@host:port/db`, `postgres://…`) prefill new connections (F-CONN-016).

use crate::object::ObjectKind;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectUri {
    pub conn: String,
    /// Left out when sharing with "hide host information"
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub kind: Option<ObjectKind>,
    pub name: Option<String>,
}

impl ObjectUri {
    pub fn build(&self) -> String {
        let mut q = vec![("conn", self.conn.clone())];
        let opt = |k, v: &Option<String>| v.clone().map(|v| (k, v));
        q.extend(opt("host", &self.host));
        q.extend(self.port.map(|p| ("port", p.to_string())));
        q.extend(opt("db", &self.database));
        q.extend(opt("schema", &self.schema));
        q.extend(self.kind.map(|k| ("kind", k.key().to_string())));
        q.extend(opt("name", &self.name));
        let query: Vec<String> = q
            .into_iter()
            .map(|(k, v)| format!("{k}={}", encode(&v)))
            .collect();
        format!("navidog://open?{}", query.join("&"))
    }

    pub fn parse(s: &str) -> Option<Self> {
        let query = s.trim().strip_prefix("navidog://open?")?;
        let mut uri = Self::default();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = decode(v)?;
            match k {
                "conn" => uri.conn = v,
                "host" => uri.host = Some(v),
                "port" => uri.port = v.parse().ok(),
                "db" => uri.database = Some(v),
                "schema" => uri.schema = Some(v),
                "kind" => uri.kind = ObjectKind::from_key(&v),
                "name" => uri.name = Some(v),
                _ => {}
            }
        }
        (!uri.conn.is_empty()).then_some(uri)
    }
}

/// A standard database URI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatabaseUri {
    /// Driver id: mysql, mariadb or postgres
    pub driver: String,
    pub user: String,
    pub password: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub database: Option<String>,
}

impl DatabaseUri {
    pub fn parse(s: &str) -> Option<Self> {
        let (scheme, rest) = s.trim().split_once("://")?;
        let driver = match scheme {
            "mysql" => "mysql",
            "mariadb" => "mariadb",
            "postgres" | "postgresql" => "postgres",
            _ => return None,
        };
        let rest = rest.split(['?', '#']).next().unwrap_or_default();
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        let (user, password) = match userinfo {
            Some(u) => match u.split_once(':') {
                Some((u, p)) => (decode(u)?, Some(decode(p)?)),
                None => (decode(u)?, None),
            },
            None => (String::new(), None),
        };
        // IPv6 hosts are written as [::1]:5432
        let (host, port) = match hostport.strip_prefix('[') {
            Some(v6) => {
                let (h, p) = v6.split_once(']')?;
                (
                    h.to_string(),
                    p.strip_prefix(':').and_then(|p| p.parse().ok()),
                )
            }
            None => match hostport.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p.parse().ok()?)),
                None => (hostport.to_string(), None),
            },
        };
        let database = Some(decode(path)?).filter(|d| !d.is_empty());
        Some(Self {
            driver: driver.into(),
            user,
            password,
            host,
            port,
            database,
        })
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_uris_round_trip() {
        let uri = ObjectUri {
            conn: "本地 MySQL".into(),
            host: Some("db.example.com".into()),
            port: Some(3306),
            database: Some("sakila".into()),
            schema: None,
            kind: Some(ObjectKind::Table),
            name: Some("film actor&x".into()),
        };
        let s = uri.build();
        assert!(s.starts_with("navidog://open?conn=%E6%9C%AC"), "{s}");
        assert_eq!(ObjectUri::parse(&s), Some(uri));
        assert_eq!(
            ObjectUri::parse("navidog://open?db=x"),
            None,
            "a connection name is required"
        );
        assert_eq!(ObjectUri::parse("https://x"), None);
    }

    #[test]
    fn database_uris() {
        let u = DatabaseUri::parse("postgresql://app:p%40ss@db.local:6543/shop?sslmode=require")
            .unwrap();
        assert_eq!(
            u,
            DatabaseUri {
                driver: "postgres".into(),
                user: "app".into(),
                password: Some("p@ss".into()),
                host: "db.local".into(),
                port: Some(6543),
                database: Some("shop".into()),
            }
        );
        let m = DatabaseUri::parse("mysql://root@[::1]/").unwrap();
        assert_eq!((m.host.as_str(), m.port, m.database), ("::1", None, None));
        assert!(DatabaseUri::parse("redis://x").is_none());
    }
}
