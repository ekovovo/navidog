//! Commands shared across features. Feature crates do not depend on each other and cooperate
//! through these actions: the sender only dispatches, the handler binds them on the window root
//! or its own panel. Unless they carry parameters, commands act on the workspace selection.

use gpui_kit::{Action, SharedString};

use crate::object::ObjectRef;

gpui_kit::actions!(
    connection,
    [
        NewConnection,
        EditConnection,
        DuplicateConnection,
        DeleteConnection,
        OpenConnection,
        CloseConnection,
        NewGroup,
        RenameGroup,
        DeleteGroup,
        Refresh,
        OpenDatabase,
        CloseDatabase,
        ToggleStar,
        ToggleHidden,
        ManageConnections,
        ReloadConnection,
        NewProfile,
        DeleteProfile,
    ]
);

// Hides everything but the active editor and enters full screen (F-APP-008); editors show the toggle
gpui_kit::actions!(navidog, [ToggleFocusMode]);

// Query editor, history log and console (30-query-editor.md, 31-query-results.md). New queries
// take their connection, database and schema from the workspace selection.
gpui_kit::actions!(
    query,
    [
        NewQuery,
        OpenHistoryLog,
        OpenConsole,
        ShowSnippets,
        // File > Open External File > Query…
        OpenExternalQuery,
        // Saved queries selected in the navigator or objects tab
        DuplicateQuery,
        OpenQueryExternally,
        RevealQuery,
    ]
);

/// "Open Query": opens a saved query and runs it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = query, no_json)]
pub struct RunQuery {
    pub object: ObjectRef,
}

/// "Query Table": a new query holding `sql`, aimed at this connection, then run.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = query, no_json)]
pub struct NewQueryText {
    pub connection: SharedString,
    pub database: SharedString,
    pub schema: Option<SharedString>,
    pub sql: SharedString,
}

/// A saved query file was renamed; open editors follow it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = query, no_json)]
pub struct QueryRenamed {
    pub from: ObjectRef,
    pub to: ObjectRef,
}

/// Opens a connection by ID, asking for its password if needed (a query editor picking a closed
/// connection).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct OpenConnectionId {
    pub id: SharedString,
}

/// Explicit recovery after a lost session; confirmation belongs to the connection feature.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
#[non_exhaustive]
pub struct ReopenConnectionId {
    pub id: SharedString,
}

impl ReopenConnectionId {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self { id: id.into() }
    }
}

/// Inserts text at the caret of the active query editor (info pane identifiers and snippets).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = query, no_json)]
pub struct InsertText {
    pub text: SharedString,
    /// The text is a snippet: `${…}` marks placeholders to tab through
    pub snippet: bool,
}

/// Opens the snippet editor with `code` filled in ("Create Snippet…" in the query editor).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = query, no_json)]
pub struct CreateSnippet {
    pub code: SharedString,
}

// View > Navigation Pane options; the shell persists them and passes them to the navigator
gpui_kit::actions!(
    navigator,
    [
        ToggleOnlyActive,
        ToggleShowHidden,
        ToggleHideObjectGroups,
        ToggleShowTableProfile
    ]
);

// Operations on the selected objects (navigator tree, objects tab, menu bar)
gpui_kit::actions!(
    objects,
    [
        DeleteObjects,
        EmptyTables,
        TruncateTables,
        RenameObject,
        NewVirtualGroup,
        RemoveFromVirtualGroup
    ]
);

/// Duplicates the selected table, structure only or with its data.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct DuplicateTable {
    pub with_data: bool,
}

/// Moves the selected objects into a virtual group.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct AddToVirtualGroup {
    pub group: SharedString,
}

/// Create a connection of a specific database type directly (toolbar dropdown, menu bar submenu).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct NewConnectionOf {
    pub driver: SharedString,
}

/// Set the connection color; `None` clears it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct SetColor {
    pub color: Option<SharedString>,
}

/// Move the connection to a group; `None` means the top level.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct MoveToGroup {
    pub group: Option<SharedString>,
}

/// Switch the selected connection to another profile (F-CONN-021); "" is the default profile.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct SwitchProfile {
    pub name: SharedString,
}

/// Open an object in the object pane: tables open in the table viewer; other objects open as
/// read-only DDL until M4.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct OpenObject {
    pub object: ObjectRef,
}

// Table design commands use the selected database or a named table.
gpui_kit::actions!(designer, [NewTable]);
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = designer, no_json)]
pub struct DesignTable {
    /// None resolves the current table viewer; Some names a table from the navigator.
    pub object: Option<ObjectRef>,
}

/// A table was saved; the shell refreshes catalog and completion caches.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = designer, no_json)]
pub struct TableStructureChanged {
    pub scope: crate::object::Scope,
}
