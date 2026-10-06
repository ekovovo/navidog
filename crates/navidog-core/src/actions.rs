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
    ]
);

// View > Navigation Pane options; the shell persists them and passes them to the navigator
gpui_kit::actions!(
    navigator,
    [ToggleOnlyActive, ToggleShowHidden, ToggleHideObjectGroups]
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

/// Open an object in the object pane: tables open in the table viewer; other objects open as
/// read-only DDL until M4.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct OpenObject {
    pub object: ObjectRef,
}
