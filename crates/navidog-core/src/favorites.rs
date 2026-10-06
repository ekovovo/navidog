//! Favorites (F-APP-011): objects opened from the Favorites menu or with ⌃⇧1–9, saved in
//! `favorites.json`. Recently opened objects (F-APP-013) are kept here too, in `recent.json`.

use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use serde::{Deserialize, Serialize};

use crate::object::ObjectRef;
use crate::paths;

/// How many recently opened objects the File menu lists.
const RECENT_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub name: String,
    pub object: ObjectRef,
}

#[derive(Default, Serialize, Deserialize)]
struct Lists {
    favorites: Vec<Favorite>,
    recent: Vec<ObjectRef>,
}

#[derive(Default)]
pub struct Favorites {
    lists: Lists,
    /// Where to save; `None` keeps the lists in memory (tests)
    path: Option<std::path::PathBuf>,
}

struct GlobalFavorites(Entity<Favorites>);
impl Global for GlobalFavorites {}

impl Favorites {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalFavorites>().0.clone()
    }

    fn load() -> Self {
        let path = paths::config_dir().join("favorites.json");
        let lists = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            lists,
            path: Some(path),
        }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = &self.path {
            let bytes = serde_json::to_vec_pretty(&self.lists).expect("favorites serialize");
            if let Err(err) = paths::write_atomic(path, &bytes) {
                tracing::error!("failed to save favorites.json: {err}");
            }
        }
        cx.notify();
    }

    pub fn favorites(&self) -> &[Favorite] {
        &self.lists.favorites
    }

    pub fn recent(&self) -> &[ObjectRef] {
        &self.lists.recent
    }

    /// Adds an object (once); its menu label defaults to the object name.
    pub fn add(&mut self, object: ObjectRef, cx: &mut Context<Self>) {
        if !self.lists.favorites.iter().any(|f| f.object == object) {
            let name = object.name.clone();
            self.lists.favorites.push(Favorite { name, object });
            self.commit(cx);
        }
    }

    pub fn remove(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.lists.favorites.len() {
            self.lists.favorites.remove(ix);
            self.commit(cx);
        }
    }

    /// Records an opened object, most recent first.
    pub fn opened(&mut self, object: ObjectRef, cx: &mut Context<Self>) {
        self.lists.recent.retain(|o| *o != object);
        self.lists.recent.insert(0, object);
        self.lists.recent.truncate(RECENT_LIMIT);
        self.commit(cx);
    }

    pub fn clear_recent(&mut self, cx: &mut Context<Self>) {
        self.lists.recent.clear();
        self.commit(cx);
    }
}

pub(crate) fn init(cx: &mut App) {
    let favorites = cx.new(|_| Favorites::load());
    cx.set_global(GlobalFavorites(favorites));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{ObjectKind, Scope};

    #[gpui_kit::test]
    fn recent_is_most_recent_first_and_bounded(cx: &mut gpui_kit::TestAppContext) {
        let favorites = cx.new(|_| Favorites::default());
        let obj = |i: usize| ObjectRef {
            scope: Scope::new("c", "db", None),
            kind: ObjectKind::Table,
            name: format!("t{i}"),
        };
        favorites.update(cx, |f, cx| {
            for i in 0..12 {
                f.opened(obj(i), cx);
            }
            f.opened(obj(5), cx);
            assert_eq!(f.recent().len(), RECENT_LIMIT);
            assert_eq!(f.recent()[0], obj(5));
            assert_eq!(f.recent().iter().filter(|o| **o == obj(5)).count(), 1);
            f.add(obj(1), cx);
            f.add(obj(1), cx);
            assert_eq!(f.favorites().len(), 1);
        });
    }
}
