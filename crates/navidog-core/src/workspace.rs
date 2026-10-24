//! Workspace state shared across features: the navigator selection and the status bar text.

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString};

use crate::connection::{ConnectionId, GroupId};
use crate::object::{ObjectKind, ObjectRef, Scope};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    Group(GroupId),
    Connection(ConnectionId),
    Database(ConnectionId, String),
    /// A PostgreSQL schema
    Schema(Scope),
    /// An object category node under a database or schema
    Category(Scope, ObjectKind),
    Object(ObjectRef),
}

impl Selection {
    pub fn connection_id(&self) -> Option<&ConnectionId> {
        match self {
            Self::Connection(id) | Self::Database(id, _) => Some(id),
            Self::Schema(s) | Self::Category(s, _) => Some(&s.connection),
            Self::Object(o) => Some(&o.scope.connection),
            Self::None | Self::Group(_) => None,
        }
    }

    /// Database / schema scope of the selection (`None` for connections and groups).
    pub fn scope(&self) -> Option<Scope> {
        match self {
            Self::Database(c, d) => Some(Scope::new(c.clone(), d.clone(), None)),
            Self::Schema(s) | Self::Category(s, _) => Some(s.clone()),
            Self::Object(o) => Some(o.scope.clone()),
            _ => None,
        }
    }
}

/// Where the active query editor points; the info pane's snippets and identifiers pages use it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryContext {
    pub connection: Option<ConnectionId>,
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl QueryContext {
    pub fn scope(&self) -> Option<Scope> {
        Some(Scope::new(
            self.connection.clone()?,
            self.database.clone()?,
            self.schema.clone(),
        ))
    }
}

pub enum WorkspaceEvent {
    SelectionChanged,
    /// Saved query files were written, renamed or deleted; lists of them reload
    QueriesChanged,
    /// User SQL finished on this connection; cached data and metadata may have changed.
    DatabaseChanged(ConnectionId),
}

#[derive(Default)]
pub struct Workspace {
    selection: Selection,
    /// Other selected nodes besides the primary one (⌘- or ⇧-click in the navigator)
    also: Vec<Selection>,
    status: SharedString,
    /// Column focused in the table viewer. The info pane's Fields page shows this column.
    focused_column: Option<String>,
    /// Set while a query editor tab is active
    query_context: Option<QueryContext>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

struct GlobalWorkspace(Entity<Workspace>);
impl Global for GlobalWorkspace {}

impl Workspace {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalWorkspace>().0.clone()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// All selected nodes, the primary one first.
    pub fn selections(&self) -> impl Iterator<Item = &Selection> {
        std::iter::once(&self.selection).chain(&self.also)
    }

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.select_many(selection, Vec::new(), cx);
    }

    /// Selects several nodes; `primary` drives the objects tab and info pane.
    pub fn select_many(
        &mut self,
        primary: Selection,
        mut also: Vec<Selection>,
        cx: &mut Context<Self>,
    ) {
        also.retain(|s| *s != primary && *s != Selection::None);
        if self.selection != primary || self.also != also {
            self.selection = primary;
            self.also = also;
            cx.emit(WorkspaceEvent::SelectionChanged);
            cx.notify();
        }
    }

    pub fn query_context(&self) -> Option<&QueryContext> {
        self.query_context.as_ref()
    }

    pub fn set_query_context(&mut self, context: Option<QueryContext>, cx: &mut Context<Self>) {
        if self.query_context != context {
            self.query_context = context;
            cx.notify();
        }
    }

    pub fn queries_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::QueriesChanged);
    }

    /// Invalidates consumers of a completed SQL run without changing the current selection.
    pub fn database_changed(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::DatabaseChanged(connection));
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    pub fn focused_column(&self) -> Option<&str> {
        self.focused_column.as_deref()
    }

    pub fn set_focused_column(&mut self, column: Option<String>, cx: &mut Context<Self>) {
        if self.focused_column != column {
            self.focused_column = column;
            cx.notify();
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    let ws = cx.new(|_| Workspace::default());
    cx.set_global(GlobalWorkspace(ws));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_connection_of_selections() {
        let scope = Scope::new("c-1", "app", Some("public".into()));
        let obj = Selection::Object(ObjectRef {
            scope: scope.clone(),
            kind: ObjectKind::Table,
            name: "t".into(),
        });
        assert_eq!(obj.connection_id().map(String::as_str), Some("c-1"));
        assert_eq!(obj.scope(), Some(scope));
        assert_eq!(Selection::Connection("c-1".into()).scope(), None);
        assert_eq!(Selection::Group("g".into()).connection_id(), None);
    }
}
