//! Driver abstraction: a `Driver` creates a `Session`, and the session exposes `Relational`,
//! `Document` or `KeyValue` depending on the database family.
//! Phase one implements only `Relational`; the other two are interfaces only, implemented when
//! MongoDB and Redis land in M14.

use std::sync::Arc;

use futures::future::BoxFuture;
use gpui_kit::{App, Global, SharedString};
use navidog_core::object::{ObjectKind, Scope};
use navidog_core::registry::Registry;

use crate::dialect::Dialect;
use crate::params::ConnectParams;

/// Driver error. `code` keeps the database's original error code, displayed in Navicat's
/// "1045 - Access denied …" format.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct DbError {
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{code} - {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl DbError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn coded(code: impl ToString, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.to_string()),
            message: message.into(),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Relational,
    Document,
    KeyValue,
}

#[derive(Clone, Debug)]
pub struct DriverInfo {
    /// Registry key, also the `driver` field in connections.json
    pub id: SharedString,
    pub name: SharedString,
    pub family: Family,
    pub default_port: Option<u16>,
    pub supports_ssl: bool,
    pub supports_ssh: bool,
    /// A session can access only one database (PostgreSQL); browsing another database needs
    /// a separate session
    pub per_database_sessions: bool,
}

pub trait Driver: Send + Sync + 'static {
    fn info(&self) -> &DriverInfo;
    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>>;
}

/// Cancels the statement currently executing.
pub type CancelHandle = Arc<dyn Fn() -> BoxFuture<'static, DbResult<()>> + Send + Sync>;

pub trait Session: Send + 'static {
    /// Server version obtained when connecting
    fn server_version(&self) -> &str;
    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>>;
    fn cancel_handle(&self) -> Option<CancelHandle>;
    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>>;

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        None
    }
    fn document(&mut self) -> Option<&mut dyn Document> {
        None
    }
    fn key_value(&mut self) -> Option<&mut dyn KeyValue> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// Type name; empty when the text protocol does not provide the type
    pub type_name: String,
}

/// Result set in text form; `None` means NULL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected_rows: Option<u64>,
}

/// One entry in an object list (the List and Detail views of the Objects tab).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub name: String,
    pub kind: ObjectKind,
    pub comment: Option<String>,
    /// Detail view columns; keys are stable English names (rows, data_length, engine, created, …)
    pub properties: Vec<(String, String)>,
}

/// Elements shown under a table in the navigator, besides its fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableChildren {
    pub indexes: Vec<String>,
    pub foreign_keys: Vec<String>,
    pub triggers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default: Option<String>,
    pub comment: Option<String>,
}

pub trait Relational: Send {
    fn dialect(&self) -> Dialect;
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>>;
    /// Execute one or more statements over the text protocol; each returns one result set.
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>>;

    // Metadata: default implementations generate SQL per dialect and run it via `execute`
    // (metadata.rs)

    /// Schemas in the database this session is connected to; dialects without a schema level
    /// return an empty list.
    fn list_schemas(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>> {
        Box::pin(crate::metadata::list_schemas(self))
    }
    fn object_kinds(&self) -> &'static [ObjectKind] {
        self.dialect().object_kinds()
    }
    fn list_objects<'a>(
        &'a mut self,
        scope: &'a Scope,
        kind: ObjectKind,
    ) -> BoxFuture<'a, DbResult<Vec<ObjectInfo>>> {
        Box::pin(crate::metadata::list_objects(self, scope, kind))
    }
    fn object_ddl<'a>(
        &'a mut self,
        scope: &'a Scope,
        kind: ObjectKind,
        name: &'a str,
    ) -> BoxFuture<'a, DbResult<String>> {
        Box::pin(crate::metadata::object_ddl(self, scope, kind, name))
    }
    fn table_columns<'a>(
        &'a mut self,
        scope: &'a Scope,
        table: &'a str,
    ) -> BoxFuture<'a, DbResult<Vec<ColumnInfo>>> {
        Box::pin(crate::metadata::table_columns(self, scope, table))
    }
    fn table_children<'a>(
        &'a mut self,
        scope: &'a Scope,
        table: &'a str,
    ) -> BoxFuture<'a, DbResult<TableChildren>> {
        Box::pin(crate::metadata::table_children(self, scope, table))
    }
}

pub trait Document: Send {
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>>;
    fn list_collections<'a>(
        &'a mut self,
        database: &'a str,
    ) -> BoxFuture<'a, DbResult<Vec<String>>>;
    /// Query with a JSON filter; returns documents as JSON text.
    fn find<'a>(
        &'a mut self,
        database: &'a str,
        collection: &'a str,
        filter_json: &'a str,
        limit: u64,
    ) -> BoxFuture<'a, DbResult<Vec<String>>>;
}

pub trait KeyValue: Send {
    fn databases(&mut self) -> BoxFuture<'_, DbResult<Vec<u32>>>;
    /// SCAN a batch of keys; returns the next cursor and the keys.
    fn scan<'a>(
        &'a mut self,
        db: u32,
        pattern: &'a str,
        cursor: u64,
        count: u32,
    ) -> BoxFuture<'a, DbResult<(u64, Vec<String>)>>;
    fn get<'a>(&'a mut self, db: u32, key: &'a str) -> BoxFuture<'a, DbResult<Option<String>>>;
}

/// Registered drivers, shared by built-in drivers and plugin JS drivers (M5).
#[derive(Default)]
pub struct DriverRegistry(pub Registry<Arc<dyn Driver>>);

impl Global for DriverRegistry {}

impl DriverRegistry {
    pub fn get(cx: &App, id: &str) -> Option<Arc<dyn Driver>> {
        cx.global::<Self>().0.get(id).cloned()
    }

    pub fn all(cx: &App) -> Vec<Arc<dyn Driver>> {
        cx.global::<Self>()
            .0
            .iter()
            .map(|(_, _, d)| d.clone())
            .collect()
    }
}
