//! 跨 feature 共享的工作区状态：导航树的当前选择与状态栏文本。

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
    /// PostgreSQL 的模式
    Schema(Scope),
    /// 数据库或模式下的对象分类节点
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

    /// 选择所在的数据库 / 模式作用域（选中连接或组时为 `None`）。
    pub fn scope(&self) -> Option<Scope> {
        match self {
            Self::Database(c, d) => Some(Scope::new(c.clone(), d.clone(), None)),
            Self::Schema(s) | Self::Category(s, _) => Some(s.clone()),
            Self::Object(o) => Some(o.scope.clone()),
            _ => None,
        }
    }
}

pub enum WorkspaceEvent {
    SelectionChanged,
}

#[derive(Default)]
pub struct Workspace {
    selection: Selection,
    status: SharedString,
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

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection != selection {
            self.selection = selection;
            cx.emit(WorkspaceEvent::SelectionChanged);
            cx.notify();
        }
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
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
