//! Virtual groups (F-NAV-007): user-defined folders that group the objects of one category,
//! one `.vgroups.json` per connection. Connection groups live in the connection store instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use serde::{Deserialize, Serialize};

use crate::object::{ObjectKind, ObjectRef, Scope};
use crate::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualGroup {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub kind: ObjectKind,
    /// Object names in this group
    pub members: Vec<String>,
}

#[derive(Default)]
pub struct VirtualGroups {
    groups: Vec<VirtualGroup>,
    /// Config directory; `None` keeps the groups in memory (tests)
    root: Option<PathBuf>,
    /// Connections that have a `.vgroups.json`, so an emptied one can be deleted
    written: HashSet<String>,
}

struct GlobalGroups(Entity<VirtualGroups>);
impl Global for GlobalGroups {}

impl VirtualGroups {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalGroups>().0.clone()
    }

    fn load() -> Self {
        let root = paths::config_dir();
        let (groups, written) = load_all(&root);
        Self {
            groups,
            root: Some(root),
            written,
        }
    }

    fn commit(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if let Some(root) = self.root.clone() {
            save_connections(&root, &self.groups, &mut self.written, ids);
        }
        cx.notify();
    }

    /// Drops this connection's groups without writing. The profile directory is about to go.
    pub fn forget_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        self.groups.retain(|g| g.scope.connection != id);
        self.written.remove(id);
        cx.notify();
    }

    pub fn drop_connection(id: &str, cx: &mut App) {
        if !cx.has_global::<GlobalGroups>() {
            return;
        }
        Self::global(cx).update(cx, |groups, cx| groups.forget_connection(id, cx));
    }

    /// Groups of one category, in creation order.
    pub fn groups_for<'a>(
        &'a self,
        scope: &'a Scope,
        kind: ObjectKind,
    ) -> impl Iterator<Item = &'a VirtualGroup> {
        self.groups
            .iter()
            .filter(move |g| g.scope == *scope && g.kind == kind)
    }

    pub fn get(&self, id: &str) -> Option<&VirtualGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_of(&self, obj: &ObjectRef) -> Option<&VirtualGroup> {
        self.groups
            .iter()
            .find(|g| g.scope == obj.scope && g.kind == obj.kind && g.members.contains(&obj.name))
    }

    pub fn add_group(
        &mut self,
        scope: Scope,
        kind: ObjectKind,
        name: String,
        cx: &mut Context<Self>,
    ) -> String {
        let id = format!("vg-{}", uuid::Uuid::new_v4().simple());
        let connection = scope.connection.clone();
        self.groups.push(VirtualGroup {
            id: id.clone(),
            name,
            scope,
            kind,
            members: Vec::new(),
        });
        self.commit(std::slice::from_ref(&connection), cx);
        id
    }

    pub fn rename_group(&mut self, id: &str, name: String, cx: &mut Context<Self>) {
        let connection = self
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.scope.connection.clone());
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
            g.name = name;
        }
        if let Some(connection) = connection {
            self.commit(std::slice::from_ref(&connection), cx);
        }
    }

    pub fn delete_group(&mut self, id: &str, cx: &mut Context<Self>) {
        let connection = self
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.scope.connection.clone());
        self.groups.retain(|g| g.id != id);
        if let Some(connection) = connection {
            self.commit(std::slice::from_ref(&connection), cx);
        }
    }

    /// Moves objects into `group` (`None`: back to the top level). An object is in at most one
    /// group of its category.
    pub fn move_objects(
        &mut self,
        objects: &[ObjectRef],
        group: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let mut ids: Vec<String> = objects.iter().map(|o| o.scope.connection.clone()).collect();
        if let Some(connection) = group
            .and_then(|id| self.groups.iter().find(|g| g.id == id))
            .map(|g| g.scope.connection.clone())
            && !ids.contains(&connection)
        {
            ids.push(connection);
        }
        for g in &mut self.groups {
            g.members.retain(|m| {
                !objects
                    .iter()
                    .any(|o| o.scope == g.scope && o.kind == g.kind && o.name == *m)
            });
        }
        if let Some(g) = group.and_then(|id| self.groups.iter_mut().find(|g| g.id == id)) {
            for o in objects
                .iter()
                .filter(|o| o.scope == g.scope && o.kind == g.kind)
            {
                g.members.push(o.name.clone());
            }
        }
        self.commit(&ids, cx);
    }
}

#[derive(Serialize, Deserialize)]
struct StoredGroup {
    id: String,
    name: String,
    database: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    kind: ObjectKind,
    members: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredFile {
    version: u32,
    groups: Vec<StoredGroup>,
}

fn file_path(root: &Path, connection: &str) -> PathBuf {
    root.join("profiles").join(connection).join(".vgroups.json")
}

fn load_all(root: &Path) -> (Vec<VirtualGroup>, HashSet<String>) {
    let mut groups = Vec::new();
    let mut written = HashSet::new();
    let Ok(entries) = std::fs::read_dir(root.join("profiles")) else {
        return (groups, written);
    };
    for entry in entries.flatten() {
        let Ok(id) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(bytes) = std::fs::read(entry.path().join(".vgroups.json")) else {
            continue;
        };
        let file = match serde_json::from_slice::<StoredFile>(&bytes) {
            Ok(file) => file,
            Err(err) => {
                tracing::error!("failed to read virtual groups for {id}: {err}");
                continue;
            }
        };
        written.insert(id.clone());
        for group in file.groups {
            groups.push(VirtualGroup {
                id: group.id,
                name: group.name,
                scope: Scope::new(&id, group.database, group.schema),
                kind: group.kind,
                members: group.members,
            });
        }
    }
    (groups, written)
}

/// Writes only `ids`. Another connection's file stays as it is, so two instances
/// editing different connections do not overwrite each other.
fn save_connections(
    root: &Path,
    groups: &[VirtualGroup],
    written: &mut HashSet<String>,
    ids: &[String],
) {
    for id in ids {
        let mine: Vec<&VirtualGroup> = groups
            .iter()
            .filter(|group| group.scope.connection == *id)
            .collect();
        if mine.is_empty() {
            if written.remove(id) {
                let _ = std::fs::remove_file(file_path(root, id));
            }
            continue;
        }
        let file = StoredFile {
            version: 1,
            groups: mine
                .iter()
                .map(|group| StoredGroup {
                    id: group.id.clone(),
                    name: group.name.clone(),
                    database: group.scope.database.clone(),
                    schema: group.scope.schema.clone(),
                    kind: group.kind,
                    members: group.members.clone(),
                })
                .collect(),
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&file) else {
            continue;
        };
        if let Err(err) = paths::write_atomic(&file_path(root, id), &bytes) {
            tracing::error!("failed to save virtual groups: {err}");
        } else {
            written.insert(id.clone());
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    let groups = cx.new(|_| VirtualGroups::load());
    cx.set_global(GlobalGroups(groups));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn objects_live_in_one_group(cx: &mut gpui_kit::TestAppContext) {
        let groups = cx.new(|_| VirtualGroups::default());
        let scope = Scope::new("c", "db", None);
        let obj = |n: &str| ObjectRef {
            scope: scope.clone(),
            kind: ObjectKind::Table,
            name: n.into(),
        };
        groups.update(cx, |g, cx| {
            let a = g.add_group(scope.clone(), ObjectKind::Table, "A".into(), cx);
            let b = g.add_group(scope.clone(), ObjectKind::Table, "B".into(), cx);
            g.move_objects(&[obj("t1"), obj("t2")], Some(&a), cx);
            g.move_objects(&[obj("t2")], Some(&b), cx);
            assert_eq!(
                g.group_of(&obj("t1")).map(|g| g.id.clone()),
                Some(a.clone())
            );
            assert_eq!(g.group_of(&obj("t2")).map(|g| g.id.clone()), Some(b));
            g.move_objects(&[obj("t1")], None, cx);
            assert!(g.group_of(&obj("t1")).is_none());
            // Views of the same scope have their own groups
            assert_eq!(g.groups_for(&scope, ObjectKind::View).count(), 0);
        });
    }

    #[test]
    fn groups_split_by_connection() {
        let root = std::env::temp_dir().join(format!("navidog-vgroups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let saved = vec![
            VirtualGroup {
                id: "vg-1".into(),
                name: "A".into(),
                scope: Scope::new("c-a", "db", None),
                kind: ObjectKind::Table,
                members: vec!["t".into()],
            },
            VirtualGroup {
                id: "vg-2".into(),
                name: "B".into(),
                scope: Scope::new("c-b", ".vgroups.json", None),
                kind: ObjectKind::View,
                members: Vec::new(),
            },
        ];
        let mut written = HashSet::new();
        save_connections(&root, &saved, &mut written, &["c-a".into(), "c-b".into()]);
        let (groups, written) = load_all(&root);
        assert!(written.contains("c-a") && written.contains("c-b"));
        let a = groups.iter().find(|g| g.scope.connection == "c-a").unwrap();
        assert!(a.scope.schema.is_none());
        assert_eq!(a.members, ["t"]);
        let meta = file_path(&root, "c-b");
        assert!(meta.is_file());
        assert_ne!(
            meta,
            root.join("profiles")
                .join("c-b")
                .join(crate::queries::encode_segment(".vgroups.json"))
        );

        let mut written = written;
        let kept: Vec<_> = groups
            .iter()
            .filter(|g| g.scope.connection == "c-a")
            .cloned()
            .collect();
        // Saving c-a must not rewrite or delete c-b.
        save_connections(&root, &kept, &mut written, &["c-a".into()]);
        assert!(file_path(&root, "c-a").is_file());
        assert!(file_path(&root, "c-b").is_file());
        save_connections(&root, &kept, &mut written, &["c-b".into()]);
        assert!(file_path(&root, "c-a").is_file());
        assert!(!file_path(&root, "c-b").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
