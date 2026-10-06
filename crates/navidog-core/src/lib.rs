//! 领域模型、配置目录、加密存储、注册表与跨 feature 命令。不依赖任何界面组件。

// Action 派生宏输出 `gpui::` 路径
extern crate gpui_kit as gpui;

pub mod actions;
pub mod connection;
pub mod crypto;
pub mod object;
pub mod paths;
pub mod registry;
pub mod settings;
pub mod store;
pub mod workspace;

use gpui_kit::App;

/// 初始化全局状态：设置、连接存储、注册表、工作区选择。
pub fn init(cx: &mut App) {
    settings::init(cx);
    registry::init(cx);
    store::init(cx);
    workspace::init(cx);
}
