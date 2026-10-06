//! 数据库对象的领域类型，导航树、对象选项卡、驱动、表查看器共用。

use serde::{Deserialize, Serialize};

use crate::connection::ConnectionId;

/// 对象分类（导航树中数据库或模式下的分类节点）。
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
}

impl ObjectKind {
    /// 稳定的英文键，用于 ElementId、i18n 键和持久化。
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
            _ => return None,
        })
    }
}

/// 对象所在的位置：连接、数据库，以及（PostgreSQL 的）模式。
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

/// 一个具体的数据库对象。
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
        ] {
            assert_eq!(ObjectKind::from_key(k.key()), Some(k));
        }
        assert_eq!(ObjectKind::from_key("nope"), None);
    }
}
