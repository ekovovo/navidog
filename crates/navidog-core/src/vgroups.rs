//! Virtual groups (F-NAV-007): user-defined folders that group the objects of one category,
//! saved in `vgroup.json`. Connection groups live in the connection store instead.

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

#[derive(Default, Serialize, Deserialize)]
pub struct VirtualGroups {
    groups: Vec<VirtualGroup>,
    /// Where to save; `None` keeps the groups in memory (tests)
    #[serde(skip)]
    path: Option<std::path::PathBuf>,
}

struct GlobalGroups(Entity<VirtualGroups>);
impl Global for GlobalGroups {}

impl VirtualGroups {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalGroups>().0.clone()
    }

    fn load() -> Self {
        let path = paths::config_dir().join("vgroup.json");
        let mut groups: Self = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        groups.path = Some(path);
        groups
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = &self.path {
            let bytes = serde_json::to_vec_pretty(self).expect("virtual groups serialize");
            if let Err(err) = paths::write_atomic(path, &bytes) {
                tracing::error!("failed to save vgroup.json: {err}");
            }
        }
        cx.notify();
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
        self.groups.push(VirtualGroup {
            id: id.clone(),
            name,
            scope,
            kind,
            members: Vec::new(),
        });
        self.commit(cx);
        id
    }

    pub fn rename_group(&mut self, id: &str, name: String, cx: &mut Context<Self>) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
            g.name = name;
            self.commit(cx);
        }
    }

    pub fn delete_group(&mut self, id: &str, cx: &mut Context<Self>) {
        self.groups.retain(|g| g.id != id);
        self.commit(cx);
    }

    /// Moves objects into `group` (`None`: back to the top level). An object is in at most one
    /// group of its category.
    pub fn move_objects(
        &mut self,
        objects: &[ObjectRef],
        group: Option<&str>,
        cx: &mut Context<Self>,
    ) {
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
        self.commit(cx);
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
}
