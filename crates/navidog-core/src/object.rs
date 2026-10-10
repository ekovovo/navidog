//! Domain types of database objects, shared by the navigator, objects tab, drivers and editors.

use serde::{Deserialize, Serialize};

use crate::connection::ConnectionId;

/// Object category (a category node under a database or schema in the navigator).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Function,
    Event,
    Trigger,
    Index,
    Sequence,
    /// A saved query: a `.sql` file in the profile folder, not a server object (30 §7)
    Query,
}

impl ObjectKind {
    /// Stable English key for ElementIds, i18n keys and persistence.
    pub fn key(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Function => "function",
            Self::Event => "event",
            Self::Trigger => "trigger",
            Self::Index => "index",
            Self::Sequence => "sequence",
            Self::Query => "query",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "table" => Self::Table,
            "view" => Self::View,
            "materialized_view" => Self::MaterializedView,
            "function" => Self::Function,
            "event" => Self::Event,
            "trigger" => Self::Trigger,
            "index" => Self::Index,
            "sequence" => Self::Sequence,
            "query" => Self::Query,
            _ => return None,
        })
    }
}

/// Where an object lives: connection, database, and (PostgreSQL) schema.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    pub connection: ConnectionId,
    pub database: String,
    pub schema: Option<String>,
}

impl Scope {
    pub fn new(
        connection: impl Into<ConnectionId>,
        database: impl Into<String>,
        schema: Option<String>,
    ) -> Self {
        Self {
            connection: connection.into(),
            database: database.into(),
            schema,
        }
    }
}

/// A specific database object.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectRef {
    pub scope: Scope,
    pub kind: ObjectKind,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_keys_round_trip() {
        for k in [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Function,
            ObjectKind::Event,
            ObjectKind::Trigger,
            ObjectKind::Index,
            ObjectKind::Sequence,
            ObjectKind::Query,
        ] {
            assert_eq!(ObjectKind::from_key(k.key()), Some(k));
        }
        assert_eq!(ObjectKind::from_key("nope"), None);
    }
}
