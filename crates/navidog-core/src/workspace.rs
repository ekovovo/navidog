//! 跨 feature 共享的工作区状态：导航树的当前选择与状态栏文本。

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString};

use crate::connection::{ConnectionId, GroupId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    Group(GroupId),
    Connection(ConnectionId),
    Database(ConnectionId, String),
}

impl Selection {
    pub fn connection_id(&self) -> Option<&ConnectionId> {
        match self {
            Self::Connection(id) | Self::Database(id, _) => Some(id),
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
