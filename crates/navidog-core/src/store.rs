//! connections.json 的读写与秘密字段管理。

use std::path::PathBuf;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::connection::{Connection, ConnectionId, Group, GroupId, SecretField, new_group_id};
use crate::crypto::{self, CryptoError, CryptoMode, Key};
use crate::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionsFile {
    pub version: u32,
    #[serde(default)]
    pub crypto: CryptoMode,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ConnectionsFile {
    fn default() -> Self {
        Self {
            version: 1,
            crypto: CryptoMode::Machine,
            groups: Vec::new(),
            connections: Vec::new(),
            extra: Map::new(),
        }
    }
}

fn aad(conn: &ConnectionId, field: SecretField) -> String {
    format!("{conn}/{}", field.key())
}

impl ConnectionsFile {
    pub fn get(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// 写入或清除一个秘密字段。
    pub fn set_secret(conn: &mut Connection, field: SecretField, value: Option<&str>, key: &Key) {
        let aad = aad(&conn.id, field);
        *field.slot_mut(conn) = value
            .filter(|v| !v.is_empty())
            .map(|v| key.encrypt(&aad, v));
    }

    pub fn reveal(
        conn: &Connection,
        field: SecretField,
        key: &Key,
    ) -> Result<Option<String>, CryptoError> {
        field
            .slot(conn)
            .as_ref()
            .map(|s| key.decrypt(&aad(&conn.id, field), s))
            .transpose()
    }

    /// 用新密钥重新加密全部秘密字段（设置、修改、清除主密码时）。
    pub fn rekey(&mut self, old: &Key, new: &Key) -> Result<(), CryptoError> {
        for conn in &mut self.connections {
            for field in SecretField::ALL {
                let plain = Self::reveal(conn, field, old)?;
                Self::set_secret(conn, field, plain.as_deref(), new);
            }
        }
        Ok(())
    }

    /// 复制连接：新 ID、名称追加“副本”，秘密字段按新 ID 重新加密。
    pub fn duplicate(
        &mut self,
        id: &str,
        suffix: &str,
        key: &Key,
    ) -> Result<Option<ConnectionId>, CryptoError> {
        let Some(src) = self.get(id).cloned() else {
            return Ok(None);
        };
        let mut copy = src.clone();
        copy.id = crate::connection::new_connection_id();
        copy.name = format!("{} {suffix}", src.name);
        for field in SecretField::ALL {
            let plain = Self::reveal(&src, field, key)?;
            Self::set_secret(&mut copy, field, plain.as_deref(), key);
        }
        let new_id = copy.id.clone();
        let ix = self.connections.iter().position(|c| c.id == id).unwrap();
        self.connections.insert(ix + 1, copy);
        Ok(Some(new_id))
    }

    /// 删除组：组内的连接和子组移到上一级。
    pub fn remove_group(&mut self, id: &str) {
        let Some(group) = self.groups.iter().find(|g| g.id == id).cloned() else {
            return;
        };
        for conn in &mut self.connections {
            if conn.group.as_deref() == Some(id) {
                conn.group = group.parent.clone();
            }
        }
        for g in &mut self.groups {
            if g.parent.as_deref() == Some(id) {
                g.parent = group.parent.clone();
            }
        }
        self.groups.retain(|g| g.id != id);
    }

    /// 是否会形成环：把 `group` 移到 `parent` 下。
    pub fn is_descendant(&self, group: &str, parent: &str) -> bool {
        let mut cur = Some(parent.to_string());
        while let Some(id) = cur {
            if id == group {
                return true;
            }
            cur = self
                .groups
                .iter()
                .find(|g| g.id == id)
                .and_then(|g| g.parent.clone());
        }
        false
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("已设置主密码，需要先解锁")]
    Locked,
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

pub enum StoreEvent {
    Changed,
}

/// 连接存储实体。所有修改立即保存并发出 `StoreEvent::Changed`。
pub struct ConnectionStore {
    path: PathBuf,
    file: ConnectionsFile,
    key: Option<Key>,
    pub unlock_failures: u8,
}

impl EventEmitter<StoreEvent> for ConnectionStore {}

struct GlobalStore(Entity<ConnectionStore>);
impl Global for GlobalStore {}

impl ConnectionStore {
    pub fn load(path: PathBuf) -> Self {
        let file = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                // 不覆盖无法解析的文件：备份后用空配置启动
                tracing::error!("connections.json 无法解析，已备份: {err}");
                let _ = std::fs::copy(&path, path.with_extension("json.bak"));
                ConnectionsFile::default()
            }),
            Err(_) => ConnectionsFile::default(),
        };
        Self {
            path,
            file,
            key: None,
            unlock_failures: 0,
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalStore>().0.clone()
    }

    pub fn file(&self) -> &ConnectionsFile {
        &self.file
    }

    pub fn connections(&self) -> &[Connection] {
        &self.file.connections
    }

    pub fn groups(&self) -> &[Group] {
        &self.file.groups
    }

    pub fn get(&self, id: &str) -> Option<&Connection> {
        self.file.get(id)
    }

    pub fn is_master(&self) -> bool {
        matches!(self.file.crypto, CryptoMode::Master { .. })
    }

    pub fn is_locked(&self) -> bool {
        self.key.is_none() && self.is_master()
    }

    /// 取得当前密钥：本机模式按需从钥匙串读取；主密码模式需要先 `unlock`。
    pub fn key(&mut self) -> Result<Key, StoreError> {
        if let Some(key) = &self.key {
            return Ok(key.clone());
        }
        match self.file.crypto {
            CryptoMode::Machine => {
                let key = crypto::machine_key()?;
                self.key = Some(key.clone());
                Ok(key)
            }
            CryptoMode::Master { .. } => Err(StoreError::Locked),
        }
    }

    /// 测试或嵌入时直接注入密钥，跳过钥匙串。
    pub fn set_key(&mut self, key: Key) {
        self.key = Some(key);
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), CryptoError> {
        match crypto::unlock_master(&self.file.crypto, password) {
            Ok(key) => {
                self.key = Some(key);
                self.unlock_failures = 0;
                Ok(())
            }
            Err(err) => {
                self.unlock_failures = self.unlock_failures.saturating_add(1);
                Err(err)
            }
        }
    }

    pub fn reveal(&mut self, id: &str, field: SecretField) -> Result<Option<String>, StoreError> {
        let key = self.key()?;
        let Some(conn) = self.file.get(id) else {
            return Ok(None);
        };
        Ok(ConnectionsFile::reveal(conn, field, &key)?)
    }

    /// 保存连接配置；`secrets` 中的明文按字段加密，`None` 表示保留原值，`Some("")` 表示清除。
    pub fn upsert(
        &mut self,
        mut conn: Connection,
        secrets: &[(SecretField, Option<String>)],
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let existing = self.file.get(&conn.id).cloned();
        let needs_key = secrets.iter().any(|(_, v)| v.is_some());
        let key = if needs_key { Some(self.key()?) } else { None };
        for field in SecretField::ALL {
            match secrets
                .iter()
                .find(|(f, _)| *f == field)
                .and_then(|(_, v)| v.as_deref())
            {
                Some(plain) => ConnectionsFile::set_secret(
                    &mut conn,
                    field,
                    Some(plain),
                    key.as_ref().unwrap(),
                ),
                None => {
                    *field.slot_mut(&mut conn) =
                        existing.as_ref().and_then(|e| field.slot(e).clone())
                }
            }
        }
        if !conn.general.save_password {
            conn.general.password = None;
        }
        if !conn.ssh.save_password {
            conn.ssh.password = None;
        }
        if !conn.ssh.save_passphrase {
            conn.ssh.passphrase = None;
        }
        match self.file.connections.iter_mut().find(|c| c.id == conn.id) {
            Some(slot) => *slot = conn,
            None => self.file.connections.push(conn),
        }
        self.commit(cx);
        Ok(())
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.file.connections.retain(|c| c.id != id);
        self.commit(cx);
    }

    pub fn duplicate(
        &mut self,
        id: &str,
        suffix: &str,
        cx: &mut Context<Self>,
    ) -> Result<Option<ConnectionId>, StoreError> {
        let key = self.key()?;
        let new_id = self.file.duplicate(id, suffix, &key)?;
        self.commit(cx);
        Ok(new_id)
    }

    pub fn update(&mut self, id: &str, f: impl FnOnce(&mut Connection), cx: &mut Context<Self>) {
        if let Some(conn) = self.file.connections.iter_mut().find(|c| c.id == id) {
            f(conn);
            self.commit(cx);
        }
    }

    pub fn add_group(
        &mut self,
        name: String,
        parent: Option<GroupId>,
        cx: &mut Context<Self>,
    ) -> GroupId {
        let id = new_group_id();
        self.file.groups.push(Group {
            id: id.clone(),
            name,
            parent,
            extra: Map::new(),
        });
        self.commit(cx);
        id
    }

    pub fn rename_group(&mut self, id: &str, name: String, cx: &mut Context<Self>) {
        if let Some(g) = self.file.groups.iter_mut().find(|g| g.id == id) {
            g.name = name;
            self.commit(cx);
        }
    }

    pub fn remove_group(&mut self, id: &str, cx: &mut Context<Self>) {
        self.file.remove_group(id);
        self.commit(cx);
    }

    pub fn move_to_group(&mut self, conn_id: &str, group: Option<GroupId>, cx: &mut Context<Self>) {
        self.update(conn_id, |c| c.group = group, cx);
    }

    /// 设置、修改或清除（`None`）主密码，并用新密钥重新加密全部秘密字段。
    pub fn set_master_password(
        &mut self,
        password: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let old = self.key()?;
        let (mode, new) = match password {
            Some(p) => crypto::new_master(p)?,
            None => (CryptoMode::Machine, crypto::machine_key()?),
        };
        self.file.rekey(&old, &new)?;
        self.file.crypto = mode;
        self.key = Some(new);
        self.commit(cx);
        Ok(())
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let bytes = serde_json::to_vec_pretty(&self.file).expect("连接配置可序列化");
        if let Err(err) = paths::write_atomic(&self.path, &bytes) {
            tracing::error!("保存 connections.json 失败: {err}");
        }
        cx.emit(StoreEvent::Changed);
        cx.notify();
    }
}

pub(crate) fn init(cx: &mut App) {
    let store = cx.new(|_| ConnectionStore::load(paths::config_dir().join("connections.json")));
    cx.set_global(GlobalStore(store));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;

    fn file_with(conn: Connection) -> ConnectionsFile {
        ConnectionsFile {
            connections: vec![conn],
            ..Default::default()
        }
    }

    #[test]
    fn rekey_and_duplicate_keep_plaintext() {
        let k1 = Key::random();
        let mut conn = Connection::new("mysql");
        ConnectionsFile::set_secret(&mut conn, SecretField::Password, Some("pw"), &k1);
        let id = conn.id.clone();
        let mut file = file_with(conn);

        let new_id = file.duplicate(&id, "副本", &k1).unwrap().unwrap();
        let copy = file.get(&new_id).unwrap();
        assert!(copy.name.ends_with(" 副本"));
        assert_eq!(
            ConnectionsFile::reveal(copy, SecretField::Password, &k1)
                .unwrap()
                .as_deref(),
            Some("pw")
        );

        let k2 = Key::random();
        file.rekey(&k1, &k2).unwrap();
        for c in &file.connections {
            assert_eq!(
                ConnectionsFile::reveal(c, SecretField::Password, &k2)
                    .unwrap()
                    .as_deref(),
                Some("pw")
            );
            assert!(ConnectionsFile::reveal(c, SecretField::Password, &k1).is_err());
        }
        // 序列化后没有明文
        assert!(!serde_json::to_string(&file).unwrap().contains("\"pw\""));
    }

    #[test]
    fn removing_group_lifts_children() {
        let mut file = ConnectionsFile::default();
        file.groups.push(Group {
            id: "a".into(),
            name: "A".into(),
            parent: None,
            extra: Map::new(),
        });
        file.groups.push(Group {
            id: "b".into(),
            name: "B".into(),
            parent: Some("a".into()),
            extra: Map::new(),
        });
        let mut conn = Connection::new("sqlite");
        conn.group = Some("b".into());
        file.connections.push(conn);
        assert!(file.is_descendant("a", "b"));
        file.remove_group("b");
        assert_eq!(file.connections[0].group.as_deref(), Some("a"));
        file.remove_group("a");
        assert_eq!(file.connections[0].group, None);
        assert!(file.groups.is_empty());
    }
}
