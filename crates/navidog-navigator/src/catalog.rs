//! Metadata cache: schemas, object lists, DDL, fields and table children. Loads only start when a
//! node expands or the selection changes (never in render), and are invalidated when the
//! connection closes or refreshes.

use std::collections::HashMap;
use std::hash::Hash;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Task};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
use navidog_core::workspace::{Workspace, WorkspaceEvent};
use navidog_db::sessions::{Sessions, SessionsEvent};
use navidog_db::{ColumnInfo, DbResult, ObjectInfo, TableChildren};
use tokio::task::JoinHandle;

#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(SharedString),
}

#[derive(Default)]
pub struct Catalog {
    schemas: HashMap<(String, String), Load<Vec<String>>>,
    objects: HashMap<(Scope, ObjectKind), Load<Vec<ObjectInfo>>>,
    ddl: HashMap<ObjectRef, Load<String>>,
    columns: HashMap<(Scope, String), Load<Vec<ColumnInfo>>>,
    children: HashMap<(Scope, String), Load<TableChildren>>,
    tasks: HashMap<String, Task<()>>,
    /// Bumps on every cache change. Panes that draw from a snapshot (the
    /// navigator tree) compare this, because a catalog read during render
    /// makes gpui skip observers and only redraw the pane that read it.
    revision: u64,
    /// One token per in-flight load, so a result from a load that
    /// `invalidate_connection` already replaced cannot overwrite the new one.
    generation: HashMap<String, u64>,
    generation_clock: u64,
}

struct GlobalCatalog(Entity<Catalog>);
impl Global for GlobalCatalog {}

type Slot<K, T> = fn(&mut Catalog) -> &mut HashMap<K, Load<T>>;

impl Catalog {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalCatalog>().0.clone()
    }

    pub fn schemas(&self, conn: &str, db: &str) -> Option<&Load<Vec<String>>> {
        self.schemas.get(&(conn.to_string(), db.to_string()))
    }

    pub fn objects(&self, scope: &Scope, kind: ObjectKind) -> Option<&Load<Vec<ObjectInfo>>> {
        self.objects.get(&(scope.clone(), kind))
    }

    pub fn object(&self, obj: &ObjectRef) -> Option<&ObjectInfo> {
        match self.objects(&obj.scope, obj.kind)? {
            Load::Ready(list) => list.iter().find(|o| o.name == obj.name),
            _ => None,
        }
    }

    pub fn ddl(&self, obj: &ObjectRef) -> Option<&Load<String>> {
        self.ddl.get(obj)
    }

    pub fn columns(&self, scope: &Scope, table: &str) -> Option<&Load<Vec<ColumnInfo>>> {
        self.columns.get(&(scope.clone(), table.to_string()))
    }

    pub fn table_children(&self, scope: &Scope, table: &str) -> Option<&Load<TableChildren>> {
        self.children.get(&(scope.clone(), table.to_string()))
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// `job` starts the query, so it runs only on a cache miss: `run_rel` begins executing as
    /// soon as it is called.
    fn load<K, T>(
        &mut self,
        slot_name: &'static str,
        key: K,
        slot: Slot<K, T>,
        job: impl FnOnce(&App) -> Option<JoinHandle<DbResult<T>>>,
        force: bool,
        cx: &mut Context<Self>,
    ) where
        K: Hash + Eq + Clone + std::fmt::Debug + 'static,
        T: Send + 'static,
    {
        if !force && slot(self).contains_key(&key) {
            return;
        }
        let Some(job) = job(cx) else { return };
        self.generation_clock = self.generation_clock.wrapping_add(1);
        let generation = self.generation_clock;
        // Columns and table children use the same `(scope, table)` key. The task map
        // drops the previous task, which cancels it, so the slot name has to be part of the id.
        let task_key = format!("{slot_name}:{key:?}");
        self.generation.insert(task_key.clone(), generation);
        slot(self).insert(key.clone(), Load::Loading);
        self.bump();
        let task_key_for_task = task_key.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(Ok(v)) => Load::Ready(v),
                Ok(Err(e)) => Load::Failed(crate::db_text(&e).into()),
                Err(e) => Load::Failed(e.to_string().into()),
            };
            let _ = this.update(cx, |this, cx| {
                // A newer load, or an invalidation, owns this key now.
                if this.generation.get(&task_key_for_task) != Some(&generation)
                    || !slot(this).contains_key(&key)
                {
                    return;
                }
                slot(this).insert(key, result);
                this.bump();
                cx.notify();
            });
        });
        self.tasks.insert(task_key, task);
        cx.notify();
    }

    pub fn ensure_schemas(&mut self, conn: &str, db: &str, force: bool, cx: &mut Context<Self>) {
        let (c, d) = (conn.to_string(), db.to_string());
        let job = move |cx: &App| {
            Sessions::global(cx)
                .read(cx)
                .run_rel(&c, Some(&d), |r| r.list_schemas())
        };
        let key = (conn.to_string(), db.to_string());
        self.load("schemas", key, |c| &mut c.schemas, job, force, cx);
    }

    pub fn ensure_objects(
        &mut self,
        scope: &Scope,
        kind: ObjectKind,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        // Saved queries are files in the profile folder: listed here, never by the driver
        if kind == ObjectKind::Query {
            let key = (scope.clone(), kind);
            if force || !self.objects.contains_key(&key) {
                self.objects.insert(key, Load::Ready(query_infos(scope)));
                self.bump();
                cx.notify();
            }
            return;
        }
        let s = scope.clone();
        let job = move |cx: &App| {
            let (conn, db) = (s.connection.clone(), s.database.clone());
            Sessions::global(cx)
                .read(cx)
                .run_rel(&conn, Some(&db), move |r| {
                    Box::pin(async move { r.list_objects(&s, kind).await })
                })
        };
        let key = (scope.clone(), kind);
        self.load("objects", key, |c| &mut c.objects, job, force, cx);
    }

    pub fn ensure_ddl(&mut self, obj: &ObjectRef, force: bool, cx: &mut Context<Self>) {
        let o = obj.clone();
        let job = move |cx: &App| {
            let (conn, db) = (o.scope.connection.clone(), o.scope.database.clone());
            Sessions::global(cx)
                .read(cx)
                .run_rel(&conn, Some(&db), move |r| {
                    Box::pin(async move { r.object_ddl(&o.scope, o.kind, &o.name).await })
                })
        };
        self.load("ddl", obj.clone(), |c| &mut c.ddl, job, force, cx);
    }

    pub fn ensure_columns(
        &mut self,
        scope: &Scope,
        table: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (s, t) = (scope.clone(), table.to_string());
        let job = move |cx: &App| {
            let (conn, db) = (s.connection.clone(), s.database.clone());
            Sessions::global(cx)
                .read(cx)
                .run_rel(&conn, Some(&db), move |r| {
                    Box::pin(async move { r.table_columns(&s, &t).await })
                })
        };
        let key = (scope.clone(), table.to_string());
        self.load("columns", key, |c| &mut c.columns, job, force, cx);
    }

    pub fn ensure_table_children(
        &mut self,
        scope: &Scope,
        table: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (s, t) = (scope.clone(), table.to_string());
        let job = move |cx: &App| {
            let (conn, db) = (s.connection.clone(), s.database.clone());
            Sessions::global(cx)
                .read(cx)
                .run_rel(&conn, Some(&db), move |r| {
                    Box::pin(async move { r.table_children(&s, &t).await })
                })
        };
        let key = (scope.clone(), table.to_string());
        self.load("children", key, |c| &mut c.children, job, force, cx);
    }

    /// Drops everything cached for a connection (closed or refreshed).
    pub fn invalidate_connection(&mut self, conn: &str, cx: &mut Context<Self>) {
        self.bump();
        self.schemas.retain(|(c, _), _| c != conn);
        self.objects.retain(|(s, _), _| s.connection != conn);
        self.ddl.retain(|o, _| o.scope.connection != conn);
        self.columns.retain(|(s, _), _| s.connection != conn);
        self.children.retain(|(s, _), _| s.connection != conn);
        cx.notify();
    }

    /// Reloads what is loaded under a scope (a database or schema).
    pub fn refresh_scope(&mut self, scope: &Scope, cx: &mut Context<Self>) {
        self.bump();
        let kinds: Vec<_> = self
            .objects
            .keys()
            .filter(|(s, _)| s == scope)
            .map(|(_, k)| *k)
            .collect();
        self.ddl.retain(|o, _| &o.scope != scope);
        self.columns.retain(|(s, _), _| s != scope);
        self.children.retain(|(s, _), _| s != scope);
        for kind in kinds {
            self.ensure_objects(scope, kind, true, cx);
        }
        if scope.schema.is_none() && self.schemas(&scope.connection, &scope.database).is_some() {
            self.ensure_schemas(&scope.connection, &scope.database, true, cx);
        }
    }

    /// Reloads existing entries after SQL on this connection, without expanding unopened nodes.
    fn refresh_connection(&mut self, conn: &str, cx: &mut Context<Self>) {
        Sessions::global(cx).update(cx, |sessions, cx| sessions.refresh(conn, cx));
        let schemas: Vec<_> = self
            .schemas
            .keys()
            .filter(|(c, _)| c == conn)
            .cloned()
            .collect();
        let objects: Vec<_> = self
            .objects
            .keys()
            .filter(|(s, kind)| s.connection == conn && *kind != ObjectKind::Query)
            .cloned()
            .collect();
        let ddl: Vec<_> = self
            .ddl
            .keys()
            .filter(|o| o.scope.connection == conn)
            .cloned()
            .collect();
        let columns: Vec<_> = self
            .columns
            .keys()
            .filter(|(s, _)| s.connection == conn)
            .cloned()
            .collect();
        let children: Vec<_> = self
            .children
            .keys()
            .filter(|(s, _)| s.connection == conn)
            .cloned()
            .collect();
        for (connection, database) in schemas {
            self.ensure_schemas(&connection, &database, true, cx);
        }
        for (scope, kind) in objects {
            self.ensure_objects(&scope, kind, true, cx);
        }
        for object in ddl {
            self.ensure_ddl(&object, true, cx);
        }
        for (scope, table) in columns {
            self.ensure_columns(&scope, &table, true, cx);
        }
        for (scope, table) in children {
            self.ensure_table_children(&scope, &table, true, cx);
        }
    }
}

/// Saved queries of a scope as object rows: name, modification date and size (30 §7).
fn query_infos(scope: &Scope) -> Vec<ObjectInfo> {
    navidog_core::queries::list(scope)
        .into_iter()
        .map(|q| {
            let mut properties = Vec::new();
            if let Some(t) = q.modified {
                let t: chrono::DateTime<chrono::Local> = t.into();
                properties.push(("modified".into(), t.format("%Y-%m-%d %H:%M:%S").to_string()));
            }
            properties.push(("data_length".into(), q.size.to_string()));
            ObjectInfo {
                name: q.name,
                kind: ObjectKind::Query,
                comment: None,
                properties,
            }
        })
        .collect()
}

pub(crate) fn init(cx: &mut App) {
    let catalog = cx.new(|cx: &mut Context<Catalog>| {
        cx.subscribe(
            &Workspace::global(cx),
            |this, _, event: &WorkspaceEvent, cx| match event {
                WorkspaceEvent::DatabaseChanged(connection) => {
                    this.refresh_connection(connection, cx);
                }
                WorkspaceEvent::QueriesChanged => {
                    let scopes: Vec<Scope> = this
                        .objects
                        .keys()
                        .filter(|(_, k)| *k == ObjectKind::Query)
                        .map(|(s, _)| s.clone())
                        .collect();
                    for scope in scopes {
                        this.ensure_objects(&scope, ObjectKind::Query, true, cx);
                    }
                }
                WorkspaceEvent::SelectionChanged => {}
            },
        )
        .detach();
        cx.subscribe(&Sessions::global(cx), |this, _, e: &SessionsEvent, cx| {
            if let SessionsEvent::Closed(id)
            | SessionsEvent::Reopening(id)
            | SessionsEvent::Opened(id) = e
            {
                this.invalidate_connection(id, cx);
            }
        })
        .detach();
        Catalog::default()
    });
    cx.set_global(GlobalCatalog(catalog));
}

#[cfg(test)]
mod tests {
    use super::*;
    use navidog_db::runtime;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            type_name: "int".into(),
            nullable: false,
            primary_key: true,
            default: None,
            comment: None,
        }
    }

    /// A cached key must not even start the query: `run_rel` executes as soon as it is called.
    #[gpui_kit::test]
    fn a_cache_hit_starts_no_query(cx: &mut gpui_kit::TestAppContext) {
        let catalog = cx.new(|_| Catalog::default());
        let scope = Scope::new("c", "db", None);
        let key = (scope.clone(), "actor".to_string());
        catalog.update(cx, |cat, cx| {
            cat.columns
                .insert(key.clone(), Load::Ready(vec![column("id")]));
            cat.load(
                "columns",
                key,
                |c| &mut c.columns,
                |_| -> Option<JoinHandle<DbResult<Vec<ColumnInfo>>>> {
                    panic!("a cached load started a query")
                },
                false,
                cx,
            );
        });
    }

    /// Fields and the other table children share a `(scope, table)` key. Both loads must finish.
    #[gpui_kit::test]
    fn fields_and_children_of_one_table_both_finish(cx: &mut gpui_kit::TestAppContext) {
        // The loads await `runtime::spawn` handles, so their completion wakes the
        // foreground task from the db thread. That is real IO: allow it, or the
        // scheduler records the cross-thread wake as non-determinism.
        cx.executor().allow_parking();
        let catalog = cx.new(|_| Catalog::default());
        let scope = Scope::new("c", "db", None);
        let columns = runtime::spawn(async { Ok(vec![column("id")]) });
        let children = runtime::spawn(async {
            Ok(TableChildren {
                indexes: vec!["idx".into()],
                ..TableChildren::default()
            })
        });
        catalog.update(cx, |cat, cx| {
            cat.load(
                "columns",
                (scope.clone(), "actor".into()),
                |c| &mut c.columns,
                move |_| Some(columns),
                true,
                cx,
            );
            cat.load(
                "children",
                (scope.clone(), "actor".into()),
                |c| &mut c.children,
                move |_| Some(children),
                true,
                cx,
            );
        });
        for _ in 0..50 {
            cx.run_until_parked();
            let done = catalog.read_with(cx, |cat, _| {
                matches!(cat.columns(&scope, "actor"), Some(Load::Ready(_)))
                    && matches!(cat.table_children(&scope, "actor"), Some(Load::Ready(_)))
            });
            if done {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let state = catalog.read_with(cx, |cat, _| {
            (
                format!(
                    "{:?}",
                    cat.columns(&scope, "actor")
                        .map(|l| matches!(l, Load::Ready(_)))
                ),
                format!(
                    "{:?}",
                    cat.table_children(&scope, "actor")
                        .map(|l| matches!(l, Load::Ready(_)))
                ),
            )
        });
        panic!("fields and children did not both finish: {state:?}");
    }

    #[gpui_kit::test]
    fn a_replaced_metadata_load_cannot_restore_older_values(cx: &mut gpui_kit::TestAppContext) {
        cx.executor().allow_parking();
        let catalog = cx.new(|_| Catalog::default());
        let scope = Scope::new("c", "db", None);
        let (release, pending) = tokio::sync::oneshot::channel();
        let (completed, finished) = std::sync::mpsc::channel();
        let older = runtime::spawn(async move {
            pending.await.unwrap();
            completed.send(()).unwrap();
            Ok(vec![column("older")])
        });
        let newer = runtime::spawn(async { Ok(vec![column("newer")]) });
        catalog.update(cx, |catalog, cx| {
            for job in [older, newer] {
                catalog.load(
                    "columns",
                    (scope.clone(), "items".into()),
                    |catalog| &mut catalog.columns,
                    move |_| Some(job),
                    true,
                    cx,
                );
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if catalog.read_with(cx, |catalog, _| {
                matches!(
                    catalog.columns(&scope, "items"),
                    Some(Load::Ready(columns)) if columns[0].name == "newer"
                )
            }) {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        release.send(()).unwrap();
        finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        cx.run_until_parked();
        catalog.read_with(cx, |catalog, _| {
            assert!(matches!(
                catalog.columns(&scope, "items"),
                Some(Load::Ready(columns)) if columns[0].name == "newer"
            ));
        });
    }

    #[gpui_kit::test]
    fn a_saved_query_is_listed_under_queries(cx: &mut gpui_kit::TestAppContext) {
        let _home = crate::test_env::lock();
        let dir = std::env::temp_dir().join(format!("navidog-query-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: `lock` is held until this test returns.
        unsafe { std::env::set_var("NAVIDOG_HOME", &dir) };
        let scope = Scope::new("c1", "sakila", None);
        let path = navidog_core::queries::path(&scope, "actor");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "SELECT 1").unwrap();
        let catalog = cx.new(|_| Catalog::default());
        catalog.update(cx, |cat, cx| {
            cat.ensure_objects(&scope, ObjectKind::Query, true, cx);
        });
        let names: Vec<String> =
            catalog.read_with(cx, |cat, _| match cat.objects(&scope, ObjectKind::Query) {
                Some(Load::Ready(list)) => list.iter().map(|o| o.name.clone()).collect(),
                other => panic!("query category was not ready: {other:?}"),
            });
        assert_eq!(names, vec!["actor".to_string()]);
    }
}
