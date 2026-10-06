//! 跨 feature 共享的命令。feature crate 之间不互相依赖，通过这里的 Action 协作：
//! 发出方只派发，处理方在窗口根元素或自己的面板上绑定。
//! 除带参数的命令外，都作用于工作区当前选择。

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
    ]
);

/// 直接新建某种数据库类型的连接（工具栏下拉、菜单栏子菜单）。
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct NewConnectionOf {
    pub driver: SharedString,
}

/// 设置连接颜色，`None` 为清除。
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct SetColor {
    pub color: Option<SharedString>,
}

/// 移动连接到组，`None` 为顶层。
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = connection, no_json)]
pub struct MoveToGroup {
    pub group: Option<SharedString>,
}

/// 在对象窗格中打开对象：表打开表查看器，其他对象在 M4 之前打开只读 DDL。
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct OpenObject {
    pub object: ObjectRef,
}
