//! Metadata cache: schemas, object lists, DDL, fields and table children. Loads only start when a
//! node expands or the selection changes (never in render), and are invalidated when the
//! connection closes or refreshes.

use std::collections::HashMap;
use std::hash::Hash;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Task};
use navidog_core::object::{ObjectKind, ObjectRef, Scope};
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

    fn load<K, T>(
        &mut self,
        key: K,
        slot: Slot<K, T>,
        job: Option<JoinHandle<DbResult<T>>>,
        force: bool,
        cx: &mut Context<Self>,
    ) where
        K: Hash + Eq + Clone + std::fmt::Debug + 'static,
        T: Send + 'static,
    {
        if !force && slot(self).contains_key(&key) {
            return;
        }
        let Some(job) = job else { return };
        self.generation_clock = self.generation_clock.wrapping_add(1);
        let generation = self.generation_clock;
        let task_key = format!("{key:?}");
        self.generation.insert(task_key.clone(), generation);
        slot(self).insert(key.clone(), Load::Loading);
        self.bump();
        let task_key_for_task = task_key.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = match job.await {
                Ok(Ok(v)) => Load::Ready(v),
                Ok(Err(e)) => Load::Failed(e.to_string().into()),
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
        let database = db.to_string();
        let job = Sessions::global(cx)
            .read(cx)
            .run_rel(conn, Some(db), |r| r.list_schemas());
        self.load(
            (conn.to_string(), database),
            |c| &mut c.schemas,
            job,
            force,
            cx,
        );
    }

    pub fn ensure_objects(
        &mut self,
        scope: &Scope,
        kind: ObjectKind,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let s = scope.clone();
        let job = Sessions::global(cx).read(cx).run_rel(
            &scope.connection,
            Some(&scope.database),
            move |r| Box::pin(async move { r.list_objects(&s, kind).await }),
        );
        self.load((scope.clone(), kind), |c| &mut c.objects, job, force, cx);
    }

    pub fn ensure_ddl(&mut self, obj: &ObjectRef, force: bool, cx: &mut Context<Self>) {
        let o = obj.clone();
        let job = Sessions::global(cx).read(cx).run_rel(
            &obj.scope.connection,
            Some(&obj.scope.database),
            move |r| Box::pin(async move { r.object_ddl(&o.scope, o.kind, &o.name).await }),
        );
        self.load(obj.clone(), |c| &mut c.ddl, job, force, cx);
    }

    pub fn ensure_columns(
        &mut self,
        scope: &Scope,
        table: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (s, t) = (scope.clone(), table.to_string());
        let job = Sessions::global(cx).read(cx).run_rel(
            &scope.connection,
            Some(&scope.database),
            move |r| Box::pin(async move { r.table_columns(&s, &t).await }),
        );
        self.load(
            (scope.clone(), table.to_string()),
            |c| &mut c.columns,
            job,
            force,
            cx,
        );
    }

    pub fn ensure_table_children(
        &mut self,
        scope: &Scope,
        table: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (s, t) = (scope.clone(), table.to_string());
        let job = Sessions::global(cx).read(cx).run_rel(
            &scope.connection,
            Some(&scope.database),
            move |r| Box::pin(async move { r.table_children(&s, &t).await }),
        );
        self.load(
            (scope.clone(), table.to_string()),
            |c| &mut c.children,
            job,
            force,
            cx,
        );
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
}

pub(crate) fn init(cx: &mut App) {
    let catalog = cx.new(|cx: &mut Context<Catalog>| {
        cx.subscribe(&Sessions::global(cx), |this, _, e: &SessionsEvent, cx| {
            if let SessionsEvent::Closed(id) | SessionsEvent::Opened(id) = e {
                this.invalidate_connection(id, cx);
            }
        })
        .detach();
        Catalog::default()
    });
    cx.set_global(GlobalCatalog(catalog));
}
