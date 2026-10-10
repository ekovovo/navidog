//! Code snippets (30-query-editor.md §5). The preset snippets are our own and read-only; custom
//! ones live in `snippets.json`. `${text}` in the code marks a placeholder.

use std::sync::LazyLock;

use gpui_kit::{App, AppContext as _, Context, Entity, Global};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    /// Driver ID the snippet is for (`mysql`, `mariadb`, `postgres`, `sqlite`); empty is all
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: String,
    pub code: String,
    #[serde(skip)]
    pub preset: bool,
}

impl Snippet {
    /// Shown for a connection of `driver`: the snippet is for all databases or for that one.
    pub fn applies_to(&self, driver: &str) -> bool {
        self.driver.is_empty() || self.driver == driver
    }
}

fn preset(id: &str, name: &str, driver: &str, tag: &str, code: &str) -> Snippet {
    Snippet {
        id: format!("preset-{id}"),
        name: name.into(),
        driver: driver.into(),
        tags: vec![tag.into()],
        note: String::new(),
        code: code.into(),
        preset: true,
    }
}

static PRESETS: LazyLock<Vec<Snippet>> = LazyLock::new(|| {
    vec![
        preset(
            "select",
            "SELECT",
            "",
            "DML",
            "SELECT ${columns}\nFROM ${table}\nWHERE ${condition};",
        ),
        preset(
            "insert",
            "INSERT",
            "",
            "DML",
            "INSERT INTO ${table} (${columns})\nVALUES (${values});",
        ),
        preset(
            "update",
            "UPDATE",
            "",
            "DML",
            "UPDATE ${table}\nSET ${column} = ${value}\nWHERE ${condition};",
        ),
        preset(
            "delete",
            "DELETE",
            "",
            "DML",
            "DELETE FROM ${table}\nWHERE ${condition};",
        ),
        preset(
            "join",
            "LEFT JOIN",
            "",
            "DML",
            "SELECT ${columns}\nFROM ${left} a\nLEFT JOIN ${right} b ON b.${key} = a.${key};",
        ),
        preset(
            "group",
            "GROUP BY COUNT",
            "",
            "DML",
            "SELECT ${column}, COUNT(*) AS n\nFROM ${table}\nGROUP BY ${column}\nORDER BY n DESC;",
        ),
        preset(
            "create-table",
            "CREATE TABLE",
            "",
            "DDL",
            "CREATE TABLE ${table} (\n  ${id} INTEGER PRIMARY KEY,\n  ${column} VARCHAR(255) NOT NULL\n);",
        ),
        preset(
            "create-index",
            "CREATE INDEX",
            "",
            "DDL",
            "CREATE INDEX ${index} ON ${table} (${column});",
        ),
        preset(
            "procedure",
            "CREATE PROCEDURE",
            "mysql",
            "DDL",
            "DELIMITER $$\nCREATE PROCEDURE ${name}()\nBEGIN\n  ${body};\nEND$$\nDELIMITER ;",
        ),
        preset(
            "function",
            "CREATE FUNCTION",
            "postgres",
            "DDL",
            "CREATE OR REPLACE FUNCTION ${name}() RETURNS ${type} AS $$\nBEGIN\n  ${body};\nEND;\n$$ LANGUAGE plpgsql;",
        ),
        preset(
            "pragma",
            "PRAGMA table_info",
            "sqlite",
            "Query",
            "PRAGMA table_info(${table});",
        ),
    ]
});

/// Preset and custom snippets.
pub struct Snippets {
    custom: Vec<Snippet>,
    path: std::path::PathBuf,
}

struct GlobalSnippets(Entity<Snippets>);
impl Global for GlobalSnippets {}

impl Snippets {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalSnippets>().0.clone()
    }

    fn load(path: std::path::PathBuf) -> Self {
        let custom = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { custom, path }
    }

    fn save(&self) {
        let bytes = serde_json::to_vec_pretty(&self.custom).expect("snippets serialize");
        if let Err(e) = paths::write_atomic(&self.path, &bytes) {
            tracing::error!("failed to save snippets.json: {e}");
        }
    }

    pub fn presets() -> &'static [Snippet] {
        &PRESETS
    }

    pub fn all(&self) -> impl Iterator<Item = &Snippet> {
        PRESETS.iter().chain(&self.custom)
    }

    pub fn get(&self, id: &str) -> Option<&Snippet> {
        self.all().find(|s| s.id == id)
    }

    /// Every tag in use, sorted.
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.all().flat_map(|s| s.tags.clone()).collect();
        tags.sort_by_key(|t| t.to_lowercase());
        tags.dedup();
        tags
    }

    /// A snippet name must be non-empty and unique among snippets of the same database type.
    pub fn name_ok(&self, name: &str, driver: &str, except: Option<&str>) -> bool {
        let name = name.trim();
        !name.is_empty()
            && !self.all().any(|s| {
                Some(s.id.as_str()) != except
                    && s.driver == driver
                    && s.name.eq_ignore_ascii_case(name)
            })
    }

    /// Adds or replaces a custom snippet; presets cannot change.
    pub fn upsert(&mut self, mut snippet: Snippet, cx: &mut Context<Self>) {
        if snippet.preset || snippet.id.starts_with("preset-") {
            return;
        }
        if snippet.id.is_empty() {
            snippet.id = uuid::Uuid::new_v4().to_string();
        }
        match self.custom.iter_mut().find(|s| s.id == snippet.id) {
            Some(s) => *s = snippet,
            None => self.custom.push(snippet),
        }
        self.save();
        cx.notify();
    }

    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        self.custom.retain(|s| s.id != id);
        self.save();
        cx.notify();
    }

    /// A custom copy named `name_copyN`.
    pub fn duplicate(&mut self, id: &str, cx: &mut Context<Self>) -> Option<String> {
        let src = self.get(id)?.clone();
        let name = (1..)
            .map(|n| format!("{}_copy{n}", src.name))
            .find(|n| self.name_ok(n, &src.driver, None))?;
        let copy = Snippet {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            preset: false,
            ..src
        };
        let id = copy.id.clone();
        self.upsert(copy, cx);
        Some(id)
    }
}

pub(crate) fn init(cx: &mut App) {
    let snippets = cx.new(|_| Snippets::load(paths::config_dir().join("snippets.json")));
    cx.set_global(GlobalSnippets(snippets));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn custom_snippets_save_and_presets_stay(cx: &mut gpui_kit::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("navidog-snippets-{}", std::process::id()));
        let path = dir.join("snippets.json");
        let _ = std::fs::remove_file(&path);
        let store = cx.new(|_| Snippets::load(path.clone()));
        store.update(cx, |s, cx| {
            assert!(
                !s.name_ok("SELECT", "", None),
                "presets count for uniqueness"
            );
            assert!(s.name_ok("SELECT", "mysql", None), "per database type");
            s.upsert(
                Snippet {
                    id: String::new(),
                    name: "mine".into(),
                    driver: "sqlite".into(),
                    tags: vec!["X".into()],
                    note: String::new(),
                    code: "SELECT ${x}".into(),
                    preset: false,
                },
                cx,
            );
            // A preset cannot be overwritten
            let mut p = Snippets::presets()[0].clone();
            p.code = "changed".into();
            s.upsert(p, cx);
        });
        let reloaded = Snippets::load(path.clone());
        let mine = reloaded.all().find(|s| s.name == "mine").unwrap().clone();
        assert_eq!(mine.code, "SELECT ${x}");
        assert!(reloaded.tags().contains(&"X".to_string()));
        assert_ne!(reloaded.all().next().unwrap().code, "changed");
        store.update(cx, |s, cx| {
            let copy = s.duplicate(&mine.id, cx).unwrap();
            assert_eq!(s.get(&copy).unwrap().name, "mine_copy1");
            s.delete(&mine.id, cx);
            assert!(s.get(&mine.id).is_none());
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
