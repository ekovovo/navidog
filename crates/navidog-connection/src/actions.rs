//! 连接管理的命令。除 `NewConnectionOf` 外都作用于工作区当前选择（导航树右键时先选中节点）。

use gpui_kit::{Action, SharedString};

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
