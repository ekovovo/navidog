//! Reading and writing connections.json, and managing the secret fields.

use std::path::PathBuf;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::connection::{Connection, ConnectionId, Group, GroupId, SecretField, new_group_id};
use crate::crypto::{self, CryptoError, CryptoMode, Key};
use crate::paths;
use rust_i18n::t;

/// A store or key error in the interface language. `StoreError`'s own text stays English, for logs.
pub fn error_message(e: &StoreError) -> String {
    match e {
        StoreError::Locked => t!("core.store_locked").to_string(),
        StoreError::NotFound => t!("core.not_found").to_string(),
        StoreError::Io(detail) => t!("core.save_failed", detail = detail).to_string(),
        StoreError::Crypto(CryptoError::Decrypt | CryptoError::Format) => {
            t!("core.secret_unreadable").to_string()
        }
        StoreError::Crypto(CryptoError::Keychain(detail) | CryptoError::Io(detail)) => {
            t!("core.keychain_failed", detail = detail).to_string()
        }
        StoreError::Crypto(CryptoError::Kdf(detail)) => {
            t!("core.kdf_failed", detail = detail).to_string()
        }
    }
}

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

fn aad(conn: &str, field: SecretField) -> String {
    format!("{conn}/{}", field.key())
}

impl ConnectionsFile {
    pub fn get(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// Writes or clears one secret field.
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

    /// Re-encrypts every secret field with a new key (setting, changing or clearing the master password).
    /// Returns the names of connections that had a field `old` cannot open; those fields are cleared
    /// (see `reencrypt`), so the file never mixes ciphertext of two keys.
    pub fn rekey(&mut self, old: &Key, new: &Key) -> Vec<String> {
        self.connections
            .iter_mut()
            .filter_map(|conn| {
                let id = conn.id.clone();
                Self::reencrypt(conn, &id, old, new).then(|| conn.name.clone())
            })
            .collect()
    }

    /// Re-encrypts the secrets of every profile of `conn`, read as `old_id` with `old`, for its
    /// own id with `new`. A field `old` cannot open was written with a key that is gone (another
    /// machine, a keychain item that was replaced) and can never be read again: it is cleared and
    /// its "save password" switched off, so opening asks for it. Returns whether that happened.
    fn reencrypt(conn: &mut Connection, old_id: &str, old: &Key, new: &Key) -> bool {
        let mut cleared = false;
        for ix in 0..=conn.profiles.len() {
            // Inactive profiles are swapped into the active slots so the field helpers reach them
            if ix > 0 {
                conn.swap_profile(ix - 1);
            }
            for field in SecretField::ALL {
                let opened = field
                    .slot(conn)
                    .as_ref()
                    .map(|s| old.decrypt(&aad(old_id, field), s))
                    .transpose();
                match opened {
                    Ok(plain) => Self::set_secret(conn, field, plain.as_deref(), new),
                    Err(_) => {
                        *field.slot_mut(conn) = None;
                        *field.saved_mut(conn) = false;
                        cleared = true;
                    }
                }
            }
            if ix > 0 {
                conn.swap_profile(ix - 1);
            }
        }
        cleared
    }

    /// Duplicates a connection: new id, the name gets the "copy" suffix, secrets are re-encrypted for the new id.
    pub fn duplicate(&mut self, id: &str, suffix: &str, key: &Key) -> Option<ConnectionId> {
        let src = self.get(id).cloned()?;
        let mut copy = src.clone();
        copy.id = crate::connection::new_connection_id();
        copy.name = format!("{} {suffix}", src.name);
        // A field the key cannot open is left out of the copy; the source keeps it
        Self::reencrypt(&mut copy, &src.id, key, key);
        let new_id = copy.id.clone();
        let ix = self.connections.iter().position(|c| c.id == id).unwrap();
        self.connections.insert(ix + 1, copy);
        Some(new_id)
    }

    /// Deletes a group; its connections and subgroups move up one level.
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

    /// Whether moving `group` under `parent` would form a cycle.
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
    #[error("a master password is set; unlock first")]
    Locked,
    #[error("connection or group was not found")]
    NotFound,
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("cannot save connection settings: {0}")]
    Io(#[from] std::io::Error),
}

impl StoreError {
    /// User-facing diagnostics in the interface language. `Display` remains for logs.
    pub fn localized_message(&self) -> String {
        error_message(self)
    }
}

pub enum StoreEvent {
    Changed,
}

/// The connection store entity. A successful save publishes one `StoreEvent::Changed`.
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
                // Never overwrite a file that fails to parse: back it up and start empty
                tracing::error!("connections.json cannot be parsed, backed up: {err}");
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

    /// The current key: read from the keychain on demand in local mode; master password mode needs `unlock` first.
    pub fn key(&mut self) -> Result<Key, StoreError> {
        if let Some(key) = &self.key {
            return Ok(key.clone());
        }
        let key = self.current_key()?;
        self.key = Some(key.clone());
        Ok(key)
    }

    /// Prepares a write without changing the cached key before the candidate is saved.
    fn current_key(&self) -> Result<Key, StoreError> {
        if let Some(key) = &self.key {
            return Ok(key.clone());
        }
        match self.file.crypto {
            CryptoMode::Machine => Ok(crypto::machine_key()?),
            CryptoMode::Master { .. } => Err(StoreError::Locked),
        }
    }

    /// Injects the key directly (tests, embedding), skipping the keychain.
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

    /// Saves a connection; plaintext in `secrets` is encrypted per field, `None` keeps the old value
    /// and `Some("")` clears it.
    pub fn upsert(
        &mut self,
        mut conn: Connection,
        secrets: &[(SecretField, Option<String>)],
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let existing = self.file.get(&conn.id).cloned();
        let needs_key = secrets.iter().any(|(_, v)| v.is_some());
        let key = if needs_key {
            Some(self.current_key()?)
        } else {
            None
        };
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
        let mut candidate = self.file.clone();
        match candidate.connections.iter_mut().find(|c| c.id == conn.id) {
            Some(slot) => *slot = conn,
            None => candidate.connections.push(conn),
        }
        self.commit(candidate, key, cx)
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), StoreError> {
        let mut candidate = self.file.clone();
        candidate.connections.retain(|c| c.id != id);
        self.commit(candidate, None, cx)
    }

    pub fn duplicate(
        &mut self,
        id: &str,
        suffix: &str,
        cx: &mut Context<Self>,
    ) -> Result<Option<ConnectionId>, StoreError> {
        let key = self.current_key()?;
        let mut candidate = self.file.clone();
        let new_id = candidate.duplicate(id, suffix, &key);
        self.commit(candidate, Some(key), cx)?;
        Ok(new_id)
    }

    pub fn update(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut Connection),
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let mut candidate = self.file.clone();
        let Some(conn) = candidate.connections.iter_mut().find(|c| c.id == id) else {
            return Err(StoreError::NotFound);
        };
        f(conn);
        self.commit(candidate, None, cx)
    }

    /// Saves a multi-selection operation as a single candidate, with no partial changes on failure.
    pub fn update_many(
        &mut self,
        ids: &[ConnectionId],
        mut f: impl FnMut(&mut Connection),
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let mut candidate = self.file.clone();
        if ids
            .iter()
            .any(|id| !candidate.connections.iter().any(|c| &c.id == id))
        {
            return Err(StoreError::NotFound);
        }
        if ids.is_empty() {
            return Ok(());
        }
        for conn in &mut candidate.connections {
            if ids.contains(&conn.id) {
                f(conn);
            }
        }
        self.commit(candidate, None, cx)
    }

    pub fn add_group(
        &mut self,
        name: String,
        parent: Option<GroupId>,
        cx: &mut Context<Self>,
    ) -> Result<GroupId, StoreError> {
        let id = new_group_id();
        let mut candidate = self.file.clone();
        candidate.groups.push(Group {
            id: id.clone(),
            name,
            parent,
            extra: Map::new(),
        });
        self.commit(candidate, None, cx)?;
        Ok(id)
    }

    pub fn rename_group(
        &mut self,
        id: &str,
        name: String,
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let mut candidate = self.file.clone();
        let Some(g) = candidate.groups.iter_mut().find(|g| g.id == id) else {
            return Err(StoreError::NotFound);
        };
        g.name = name;
        self.commit(candidate, None, cx)
    }

    pub fn remove_group(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), StoreError> {
        let mut candidate = self.file.clone();
        candidate.remove_group(id);
        self.commit(candidate, None, cx)
    }

    pub fn move_to_group(
        &mut self,
        conn_id: &str,
        group: Option<GroupId>,
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        self.update(conn_id, |c| c.group = group, cx)
    }

    /// Sets, changes or clears (`None`) the master password and re-encrypts every secret with the new key.
    /// Returns the connections whose unreadable saved passwords were dropped (see `ConnectionsFile::rekey`).
    pub fn set_master_password(
        &mut self,
        password: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<String>, StoreError> {
        let old = self.current_key()?;
        let (mode, new) = match password {
            Some(p) => crypto::new_master(p)?,
            None => (CryptoMode::Machine, crypto::machine_key()?),
        };
        let mut candidate = self.file.clone();
        let cleared = candidate.rekey(&old, &new);
        candidate.crypto = mode;
        self.commit(candidate, Some(new), cx)?;
        Ok(cleared)
    }

    fn commit(
        &mut self,
        candidate: ConnectionsFile,
        key: Option<Key>,
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec_pretty(&candidate).expect("connections.json serializes");
        paths::write_atomic(&self.path, &bytes)?;
        self.file = candidate;
        if let Some(key) = key {
            self.key = Some(key);
        }
        cx.emit(StoreEvent::Changed);
        cx.notify();
        Ok(())
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

    struct StoreFixture {
        path: PathBuf,
    }

    impl StoreFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("navidog-store-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self {
                path: root.join("connections.json"),
            }
        }

        fn block_write(&self) {
            std::fs::create_dir(self.path.with_extension("tmp")).unwrap();
        }

        fn unblock_write(&self) {
            std::fs::remove_dir(self.path.with_extension("tmp")).unwrap();
        }
    }

    impl Drop for StoreFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
        }
    }

    fn profiled_connection(key: &Key) -> Connection {
        let mut conn = Connection::new("mysql");
        conn.general.save_password = true;
        conn.ssh.save_password = true;
        conn.ssh.save_passphrase = true;
        for profile in ["", "other"] {
            if !profile.is_empty() {
                conn.add_profile(profile.into());
            }
            for field in SecretField::ALL {
                let value = format!("fixture-{profile}-{}", field.key());
                ConnectionsFile::set_secret(&mut conn, field, Some(&value), key);
            }
        }
        conn.switch_profile("");
        conn
    }

    fn assert_profile_secrets(conn: &Connection, key: &Key) {
        let mut conn = conn.clone();
        for profile in ["", "other"] {
            conn.switch_profile(profile);
            for field in SecretField::ALL {
                let expected = format!("fixture-{profile}-{}", field.key());
                let revealed = ConnectionsFile::reveal(&conn, field, key).unwrap();
                assert!(
                    revealed.as_deref() == Some(expected.as_str()),
                    "profile secret preserved"
                );
            }
        }
    }

    #[gpui_kit::test]
    fn failed_writes_keep_configuration_and_publish_no_changes(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let fixture = StoreFixture::new();
        let key = Key::random();
        let first = profiled_connection(&key);
        let second = profiled_connection(&key);
        let ids = [first.id.clone(), second.id.clone()];
        let store = cx.new(|_| ConnectionStore::load(fixture.path.clone()));
        let group = store.update(cx, |store, cx| {
            store.set_key(key.clone());
            let secrets =
                SecretField::ALL.map(|field| (field, Some(format!("fixture--{}", field.key()))));
            store.upsert(first, &secrets, cx).unwrap();
            store.upsert(second, &secrets, cx).unwrap();
            store.add_group("original".into(), None, cx).unwrap()
        });
        let original = cx.update(|cx| store.read(cx).file.clone());
        let disk = std::fs::read(&fixture.path).unwrap();
        let changes = Rc::new(Cell::new(0));
        let seen = changes.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&store, move |_, _: &StoreEvent, _| {
                seen.set(seen.get() + 1);
            })
        });
        fixture.block_write();
        store.update(cx, |store, cx| {
            let mut edited = store.get(&ids[0]).unwrap().clone();
            edited.name = "changed".into();
            assert!(matches!(
                store.upsert(edited, &[], cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(
                store.duplicate(&ids[0], "copy", cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(
                store.update(&ids[0], |c| c.hidden = true, cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(
                store.update_many(&ids, |c| c.starred = true, cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(store.remove(&ids[0], cx), Err(StoreError::Io(_))));
            assert!(matches!(
                store.add_group("new".into(), None, cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(
                store.rename_group(&group, "changed".into(), cx),
                Err(StoreError::Io(_))
            ));
            assert!(matches!(
                store.remove_group(&group, cx),
                Err(StoreError::Io(_))
            ));
            assert!(
                store.file == original,
                "failed candidates do not replace configuration"
            );
        });
        cx.run_until_parked();
        assert_eq!(changes.get(), 0);
        assert!(
            std::fs::read(&fixture.path).unwrap() == disk,
            "original file preserved"
        );
        fixture.unblock_write();
        store
            .update(cx, |store, cx| {
                store.update_many(&ids, |c| c.starred = true, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            changes.get(),
            1,
            "one event for the whole multi-selection save"
        );
        let saved: ConnectionsFile =
            serde_json::from_slice(&std::fs::read(&fixture.path).unwrap()).unwrap();
        assert!(
            cx.update(|cx| saved == store.read(cx).file),
            "published state is already on disk"
        );
        assert!(saved.connections.iter().all(|c| c.starred && !c.hidden));
        for conn in &saved.connections {
            assert_profile_secrets(conn, &key);
        }
    }

    #[gpui_kit::test]
    fn failed_rekey_preserves_current_key_and_every_profile(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let fixture = StoreFixture::new();
        let key = Key::random();
        let conn = profiled_connection(&key);
        let id = conn.id.clone();
        let store = cx.new(|_| ConnectionStore::load(fixture.path.clone()));
        store.update(cx, |store, cx| {
            store.set_key(key.clone());
            let secrets =
                SecretField::ALL.map(|field| (field, Some(format!("fixture--{}", field.key()))));
            store.upsert(conn, &secrets, cx).unwrap();
        });
        let original = cx.update(|cx| store.read(cx).file.clone());
        let disk = std::fs::read(&fixture.path).unwrap();
        let changes = Rc::new(Cell::new(0));
        let seen = changes.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&store, move |_, _: &StoreEvent, _| {
                seen.set(seen.get() + 1);
            })
        });
        fixture.block_write();
        store.update(cx, |store, cx| {
            assert!(matches!(
                store.set_master_password(Some("fixture-master"), cx),
                Err(StoreError::Io(_))
            ));
            assert!(
                store.file == original,
                "failed rekey preserves the crypto mode and ciphertext"
            );
            let retained = store.key().unwrap();
            assert_profile_secrets(store.get(&id).unwrap(), &retained);
        });
        cx.run_until_parked();
        assert_eq!(changes.get(), 0);
        assert!(
            std::fs::read(&fixture.path).unwrap() == disk,
            "original encrypted file preserved"
        );
        let old_disk: ConnectionsFile = serde_json::from_slice(&disk).unwrap();
        assert_profile_secrets(old_disk.get(&id).unwrap(), &key);
        fixture.unblock_write();
        store.update(cx, |store, cx| {
            assert!(
                store
                    .set_master_password(Some("fixture-master"), cx)
                    .unwrap()
                    .is_empty()
            );
        });
        cx.run_until_parked();
        assert_eq!(changes.get(), 1);
        let saved: ConnectionsFile =
            serde_json::from_slice(&std::fs::read(&fixture.path).unwrap()).unwrap();
        let new_key = crypto::unlock_master(&saved.crypto, "fixture-master").unwrap();
        assert_profile_secrets(saved.get(&id).unwrap(), &new_key);
        assert!(
            ConnectionsFile::reveal(saved.get(&id).unwrap(), SecretField::Password, &key).is_err()
        );
        store.update(cx, |store, _| {
            assert!(store.file == saved, "memory matches the saved candidate");
            let retained = store.key().unwrap();
            assert_profile_secrets(store.get(&id).unwrap(), &retained);
        });
        // A failed password change must preserve the already active master key as well.
        fixture.block_write();
        let committed = std::fs::read(&fixture.path).unwrap();
        store.update(cx, |store, cx| {
            assert!(matches!(
                store.set_master_password(Some("fixture-replacement"), cx),
                Err(StoreError::Io(_))
            ));
            assert!(
                store.file == saved,
                "failed replacement retains the active master configuration"
            );
            let retained = store.key().unwrap();
            assert_profile_secrets(store.get(&id).unwrap(), &retained);
        });
        cx.run_until_parked();
        assert_eq!(
            changes.get(),
            1,
            "failed replacement publishes no new event"
        );
        assert!(std::fs::read(&fixture.path).unwrap() == committed);
    }

    #[test]
    fn rekey_and_duplicate_keep_plaintext() {
        let k1 = Key::random();
        let mut conn = Connection::new("mysql");
        ConnectionsFile::set_secret(&mut conn, SecretField::Password, Some("pw"), &k1);
        // An inactive profile with its own password must survive both operations too
        conn.add_profile("other".into());
        ConnectionsFile::set_secret(&mut conn, SecretField::Password, Some("pw2"), &k1);
        conn.switch_profile("");
        let id = conn.id.clone();
        let mut file = file_with(conn);
        let reveal = |c: &Connection, k: &Key| {
            let mut c = c.clone();
            let active = ConnectionsFile::reveal(&c, SecretField::Password, k)
                .map(|p| p.unwrap_or_default());
            c.switch_profile("other");
            let other = ConnectionsFile::reveal(&c, SecretField::Password, k)
                .map(|p| p.unwrap_or_default());
            active.and_then(|a| other.map(|o| (a, o)))
        };

        let new_id = file.duplicate(&id, "副本", &k1).unwrap();
        let copy = file.get(&new_id).unwrap();
        assert!(copy.name.ends_with(" 副本"));
        assert_eq!(reveal(copy, &k1).unwrap(), ("pw".into(), "pw2".into()));

        let k2 = Key::random();
        assert!(file.rekey(&k1, &k2).is_empty());
        for c in &file.connections {
            assert_eq!(reveal(c, &k2).unwrap(), ("pw".into(), "pw2".into()));
            assert!(reveal(c, &k1).is_err());
        }
        // No plaintext after serialization
        let json = serde_json::to_string(&file).unwrap();
        assert!(!json.contains("\"pw\"") && !json.contains("\"pw2\""));
    }

    #[test]
    fn rekey_drops_fields_the_old_key_cannot_open() {
        let (k1, lost, k3) = (Key::random(), Key::random(), Key::random());
        let mut good = Connection::new("mysql");
        good.general.save_password = true;
        ConnectionsFile::set_secret(&mut good, SecretField::Password, Some("pw"), &k1);
        // A field from a key that is gone (another machine, a keychain item that was replaced)
        let mut bad = Connection::new("mysql");
        bad.name = "bad".into();
        bad.general.save_password = true;
        ConnectionsFile::set_secret(&mut bad, SecretField::Password, Some("x"), &lost);
        let mut file = ConnectionsFile {
            connections: vec![good, bad],
            ..Default::default()
        };
        assert_eq!(file.rekey(&k1, &k3), ["bad"]);
        // The readable one moved to the new key; the other is gone and will be asked for
        let revealed = ConnectionsFile::reveal(&file.connections[0], SecretField::Password, &k3);
        assert_eq!(revealed.unwrap().as_deref(), Some("pw"));
        let bad = &file.connections[1];
        assert!(bad.general.password.is_none() && !bad.general.save_password);
        // Nothing left that the new key cannot read
        assert!(file.rekey(&k3, &k1).is_empty());
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

    #[gpui_kit::test]
    fn missing_targets_change_nothing(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let fixture = StoreFixture::new();
        let key = Key::random();
        let conn = profiled_connection(&key);
        let id = conn.id.clone();
        let store = cx.new(|_| ConnectionStore::load(fixture.path.clone()));
        let group = store.update(cx, |store, cx| {
            store.set_key(key);
            store.upsert(conn, &[], cx).unwrap();
            store.add_group("original".into(), None, cx).unwrap()
        });
        let memory = cx.update(|cx| store.read(cx).file.clone());
        let disk = std::fs::read(&fixture.path).unwrap();
        let changes = Rc::new(Cell::new(0));
        let seen = changes.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&store, move |_, _: &StoreEvent, _| {
                seen.set(seen.get() + 1);
            })
        });
        let called = Cell::new(false);
        store.update(cx, |store, cx| {
            assert!(matches!(
                store.update("missing", |_| {}, cx),
                Err(StoreError::NotFound)
            ));
            assert!(matches!(
                store.update_many(
                    &["missing".into(), id.clone()],
                    |c| {
                        called.set(true);
                        c.name = "changed".into();
                    },
                    cx
                ),
                Err(StoreError::NotFound)
            ));
            assert!(matches!(
                store.rename_group("missing", "renamed".into(), cx),
                Err(StoreError::NotFound)
            ));
            assert!(store.update_many(&[], |_| {}, cx).is_ok());
            assert!(store.rename_group(&group, "renamed".into(), cx).is_ok());
        });
        assert!(!called.get());
        // The rejected calls publish nothing. The successful rename publishes once.
        assert_eq!(changes.get(), 1);
        let file = cx.update(|cx| store.read(cx).file.clone());
        assert_eq!(file.connections, memory.connections);
        assert_eq!(file.groups[0].name, "renamed");
        let bytes = std::fs::read(&fixture.path).unwrap();
        assert_ne!(bytes, disk);
        assert!(std::str::from_utf8(&bytes).unwrap().contains("renamed"));
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("\"missing\""));
    }
    #[gpui_kit::test]
    fn missing_update_targets_are_rejected_before_any_callback(cx: &mut gpui_kit::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let fixture = StoreFixture::new();
        let first = Connection::new("sqlite");
        let second = Connection::new("sqlite");
        let ids = [first.id.clone(), second.id.clone()];
        let store = cx.new(|_| ConnectionStore::load(fixture.path.clone()));
        store.update(cx, |store, cx| {
            store.upsert(first, &[], cx).unwrap();
            store.upsert(second, &[], cx).unwrap();
        });
        let original = cx.update(|cx| store.read(cx).file.clone());
        let disk = std::fs::read(&fixture.path).unwrap();
        let changes = Rc::new(Cell::new(0));
        let seen = changes.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&store, move |_, _: &StoreEvent, _| seen.set(seen.get() + 1))
        });
        let callbacks = Cell::new(0);
        fixture.block_write();
        store.update(cx, |store, cx| {
            let missing = "missing-connection".to_string();
            let result = store.update(&missing, |_| callbacks.set(callbacks.get() + 1), cx);
            assert!(matches!(result, Err(StoreError::NotFound)));
            for targets in [
                vec![ids[0].clone(), missing.clone()],
                vec![missing.clone(), ids[1].clone()],
                vec![ids[0].clone(), missing.clone(), ids[1].clone()],
            ] {
                let result = store.update_many(
                    &targets,
                    |connection| {
                        callbacks.set(callbacks.get() + 1);
                        connection.starred = true;
                    },
                    cx,
                );
                assert!(matches!(result, Err(StoreError::NotFound)));
            }
            store
                .update_many(&[], |_| callbacks.set(callbacks.get() + 1), cx)
                .unwrap();
            assert!(store.file == original);
        });
        cx.run_until_parked();
        assert_eq!(callbacks.get(), 0);
        assert_eq!(changes.get(), 0);
        assert!(std::fs::read(&fixture.path).unwrap() == disk);
        fixture.unblock_write();
        store
            .update(cx, |store, cx| {
                store.update_many(&ids, |connection| connection.starred = true, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(changes.get(), 1);
        assert!(cx.update(|cx| store.read(cx).connections().iter().all(|c| c.starred)));
    }
}
