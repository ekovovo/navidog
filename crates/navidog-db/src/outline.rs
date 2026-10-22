//! Code completion info (30-query-editor.md §4): tables, views, functions and every column of a
//! database (PostgreSQL: a schema), fetched in one go, kept in memory and written to
//! `profiles/<connection>/.outline/<database>[/<schema>].json` in the config folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use navidog_core::object::Scope;
use navidog_core::queries::encode_segment;
use navidog_core::settings::Settings;
use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::driver::{DbResult, Relational, ResultSet};
use crate::sessions::Sessions;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutlineTable {
    pub name: String,
    pub view: bool,
    /// (name, type) in column order
    pub columns: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    /// Databases (MySQL, SQLite) or schemas (PostgreSQL) to complete after nothing or before `.`
    pub schemas: Vec<String>,
    pub tables: Vec<OutlineTable>,
    pub functions: Vec<String>,
}

impl Outline {
    pub fn table(&self, name: &str) -> Option<&OutlineTable> {
        self.tables.iter().find(|t| t.name == name).or_else(|| {
            self.tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(name))
        })
    }
}

fn rows(sets: Vec<ResultSet>) -> Vec<Vec<String>> {
    sets.into_iter()
        .find(|s| !s.columns.is_empty())
        .map(|s| {
            s.rows
                .into_iter()
                .map(|r| r.into_iter().map(Option::unwrap_or_default).collect())
                .collect()
        })
        .unwrap_or_default()
}

fn group_columns(tables: &mut [OutlineTable], columns: Vec<Vec<String>>) {
    let mut by_table: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for c in columns {
        if let [table, name, ty, ..] = c.as_slice() {
            by_table
                .entry(table.clone())
                .or_default()
                .push((name.clone(), ty.clone()));
        }
    }
    for t in tables {
        t.columns = by_table.remove(&t.name).unwrap_or_default();
    }
}

/// Fetches the outline of a scope. `system` adds the system databases or schemas (F-PREF-006).
pub async fn fetch(rel: &mut dyn Relational, scope: &Scope, system: bool) -> DbResult<Outline> {
    let d = rel.dialect();
    let lit = |s: &str| d.string_literal(s);
    let mut out = Outline::default();
    match d {
        Dialect::MySql => {
            let db = lit(&scope.database);
            out.tables = rows(
                rel.execute_read(&format!(
                    "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = {db} ORDER BY TABLE_NAME"
                ))
                .await?,
            )
            .into_iter()
            .map(|r| OutlineTable {
                name: r[0].clone(),
                view: r.get(1).is_some_and(|t| t.contains("VIEW")),
                columns: Vec::new(),
            })
            .collect();
            let columns = rows(
                rel.execute_read(&format!(
                    "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = {db} ORDER BY TABLE_NAME, ORDINAL_POSITION"
                ))
                .await?,
            );
            group_columns(&mut out.tables, columns);
            out.functions = rows(
                rel.execute_read(&format!(
                    "SELECT ROUTINE_NAME FROM information_schema.ROUTINES \
                     WHERE ROUTINE_SCHEMA = {db} ORDER BY ROUTINE_NAME"
                ))
                .await?,
            )
            .into_iter()
            .map(|r| r[0].clone())
            .collect();
            out.schemas = rel.list_databases().await?;
            if !system {
                out.schemas.retain(|s| !d.is_system_database(s));
            }
        }
        Dialect::Postgres => {
            let schema = scope.schema.clone().unwrap_or_else(|| "public".into());
            let mut wanted = vec![lit(&schema)];
            if system {
                wanted.extend([lit("pg_catalog"), lit("information_schema")]);
            }
            let list = wanted.join(", ");
            out.tables = rows(
                rel.execute_read(&format!(
                    "SELECT c.relname, c.relkind FROM pg_class c \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname IN ({list}) AND c.relkind IN ('r', 'p', 'v', 'm', 'f') \
                     ORDER BY c.relname"
                ))
                .await?,
            )
            .into_iter()
            .map(|r| OutlineTable {
                name: r[0].clone(),
                view: matches!(r.get(1).map(String::as_str), Some("v" | "m")),
                columns: Vec::new(),
            })
            .collect();
            let columns = rows(
                rel.execute_read(&format!(
                    "SELECT c.relname, a.attname, format_type(a.atttypid, a.atttypmod) \
                     FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname IN ({list}) AND a.attnum > 0 AND NOT a.attisdropped \
                     AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY c.relname, a.attnum"
                ))
                .await?,
            );
            group_columns(&mut out.tables, columns);
            out.functions = rows(
                rel.execute_read(&format!(
                    "SELECT DISTINCT p.proname FROM pg_proc p \
                     JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname IN ({list}) ORDER BY p.proname"
                ))
                .await?,
            )
            .into_iter()
            .map(|r| r[0].clone())
            .collect();
            out.schemas = rel.list_schemas().await?;
            if !system {
                out.schemas.retain(|s| !d.is_system_schema(s));
            }
        }
        Dialect::Sqlite => {
            let db = d.quote_ident(&scope.database);
            let dbl = lit(&scope.database);
            out.tables = rows(
                rel.execute_read(&format!(
                    "SELECT name, type FROM {db}.sqlite_schema \
                     WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                     ORDER BY name"
                ))
                .await?,
            )
            .into_iter()
            .map(|r| OutlineTable {
                name: r[0].clone(),
                view: r.get(1).is_some_and(|t| t == "view"),
                columns: Vec::new(),
            })
            .collect();
            let columns = rows(
                rel.execute_read(&format!(
                    "SELECT m.name, p.name, p.type FROM {db}.sqlite_schema m \
                     JOIN pragma_table_xinfo(m.name, {dbl}) p \
                     WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
                     ORDER BY m.name, p.cid"
                ))
                .await?,
            );
            group_columns(&mut out.tables, columns);
            out.schemas = rel.list_databases().await?;
        }
    }
    Ok(out)
}

fn path_in(root: &Path, scope: &Scope) -> PathBuf {
    // A leading dot cannot clash with a database folder: those names encode `.` as `%2E`.
    let base = root
        .join("profiles")
        .join(&scope.connection)
        .join(".outline")
        .join(encode_segment(&scope.database));
    match &scope.schema {
        Some(s) => base.join(format!("{}.json", encode_segment(s))),
        None => base.with_extension("json"),
    }
}

pub fn path(scope: &Scope) -> PathBuf {
    path_in(&navidog_core::paths::config_dir(), scope)
}

pub fn load(scope: &Scope) -> Option<Outline> {
    serde_json::from_slice(&std::fs::read(path(scope)).ok()?).ok()
}

pub fn save(scope: &Scope, outline: &Outline) {
    if let Ok(bytes) = serde_json::to_vec(outline)
        && let Err(e) = navidog_core::paths::write_atomic(&path(scope), &bytes)
    {
        tracing::warn!("saving code completion info failed: {e}");
    }
}

fn clear_files() {
    let root = navidog_core::paths::config_dir();
    if let Ok(entries) = std::fs::read_dir(root.join("profiles")) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_dir_all(entry.path().join(".outline"));
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutlineState {
    Loading,
    Ready(Arc<Outline>),
    Failed(String),
}

/// Outlines by scope, for the query editor's completion and the info pane's identifiers page.
#[derive(Default)]
pub struct OutlineCache {
    map: HashMap<Scope, OutlineState>,
    tasks: HashMap<Scope, Task<()>>,
}

struct GlobalOutlines(Entity<OutlineCache>);
impl Global for GlobalOutlines {}

impl OutlineCache {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalOutlines>().0.clone()
    }

    pub fn get(&self, scope: &Scope) -> Option<&OutlineState> {
        self.map.get(scope)
    }

    pub fn ready(&self, scope: &Scope) -> Option<Arc<Outline>> {
        match self.map.get(scope)? {
            OutlineState::Ready(o) => Some(o.clone()),
            _ => None,
        }
    }

    /// Makes the outline available: with "auto update" (or `refresh`) it is fetched again;
    /// otherwise the copy on disk is used, and fetched only when there is none.
    pub fn ensure(&mut self, scope: &Scope, refresh: bool, cx: &mut Context<Self>) {
        let s = Settings::global(cx);
        let (auto, system) = (s.auto_update_outline, s.outline_system_objects);
        if !refresh && self.tasks.contains_key(scope) {
            return;
        }
        let fetch_now =
            refresh || (auto && !matches!(self.map.get(scope), Some(OutlineState::Ready(_))));
        if !fetch_now {
            if self.map.contains_key(scope) {
                return;
            }
            if let Some(o) = load(scope) {
                self.map
                    .insert(scope.clone(), OutlineState::Ready(Arc::new(o)));
                cx.notify();
                return;
            }
        }
        let target = scope.clone();
        let job = Sessions::global(cx).read(cx).run_rel(
            &scope.connection,
            Some(&scope.database),
            move |rel| Box::pin(async move { fetch(rel, &target, system).await }),
        );
        let Some(job) = job else { return };
        if !self.map.contains_key(scope) {
            self.map.insert(scope.clone(), OutlineState::Loading);
        }
        let key = scope.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                // Dropped with the connection, or cleared: do not write the file back.
                if !this.tasks.contains_key(&key) {
                    return;
                }
                this.tasks.remove(&key);
                let state = match result {
                    Ok(Ok(o)) => {
                        save(&key, &o);
                        OutlineState::Ready(Arc::new(o))
                    }
                    Ok(Err(e)) => OutlineState::Failed(e.to_string()),
                    Err(e) => OutlineState::Failed(e.to_string()),
                };
                // A failed refresh keeps the outline it had
                if !(matches!(state, OutlineState::Failed(_))
                    && matches!(this.map.get(&key), Some(OutlineState::Ready(_))))
                {
                    this.map.insert(key, state);
                }
                cx.notify();
            });
        });
        self.tasks.insert(scope.clone(), task);
        cx.notify();
    }

    /// "Clear Code Completion Info": every file and everything in memory.
    pub fn clear_all(&mut self, cx: &mut Context<Self>) {
        self.map.clear();
        self.tasks.clear();
        clear_files();
        cx.notify();
    }

    /// Drops cached outlines for a connection that is about to be deleted, so a finished fetch
    /// does not recreate `.outline` after the profile directory is removed.
    pub fn forget_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        self.map.retain(|scope, _| scope.connection != id);
        self.tasks.retain(|scope, _| scope.connection != id);
        cx.notify();
    }

    pub fn drop_connection(id: &str, cx: &mut App) {
        if !cx.has_global::<GlobalOutlines>() {
            return;
        }
        Self::global(cx).update(cx, |cache, cx| cache.forget_connection(id, cx));
    }
}

pub(crate) fn init(cx: &mut App) {
    let cache = cx.new(|_| OutlineCache::default());
    cx.set_global(GlobalOutlines(cache));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_encode_names_inside_the_connection_folder() {
        let root = Path::new("/cfg");
        let s = Scope::new("c1", "a/b", Some("public".into()));
        let p = path_in(root, &s);
        assert!(
            p.ends_with("profiles/c1/.outline/a%2Fb/public.json"),
            "{p:?}"
        );
        let m = Scope::new("c1", "sakila", None);
        assert!(path_in(root, &m).ends_with("profiles/c1/.outline/sakila.json"));
        let dotted = Scope::new("c1", ".outline", None);
        assert!(path_in(root, &dotted).ends_with("profiles/c1/.outline/%2Eoutline.json"));
    }

    #[gpui_kit::test]
    fn outline_on_disk_is_used_when_auto_update_is_off_and_clear_removes_it(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _guard = crate::lock_config_home();
        let dir = std::env::temp_dir().join(format!("navidog-outline-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: `LOCK` is held until this test returns, and this binary has no other writer.
        unsafe { std::env::set_var("NAVIDOG_HOME", &dir) };

        let scope = Scope::new("c1", "sakila", None);
        let outline = Outline {
            tables: vec![OutlineTable {
                name: "actor".into(),
                view: false,
                columns: vec![("actor_id".into(), "smallint".into())],
            }],
            ..Outline::default()
        };
        save(&scope, &outline);
        assert_eq!(
            load(&scope).unwrap().table("actor").unwrap().columns,
            vec![("actor_id".into(), "smallint".into())]
        );

        let cache = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(navidog_core::settings::Settings {
                auto_update_outline: false,
                ..navidog_core::settings::Settings::default()
            });
            cx.new(|_| OutlineCache::default())
        });
        cache.update(cx, |cache, cx| {
            cache.ensure(&scope, false, cx);
            assert_eq!(
                cache.ready(&scope).unwrap().table("actor").unwrap().name,
                "actor"
            );
            assert!(cache.tasks.is_empty(), "a disk hit must not fetch");
            cache.clear_all(cx);
            assert!(cache.ready(&scope).is_none());
        });
        assert!(load(&scope).is_none());
    }
}
