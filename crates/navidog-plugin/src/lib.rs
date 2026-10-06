//! gpui-shell 插件宿主（M5 实现，设计见 docs/design/03-plugin-api.md）。
//! M1 只保留插件目录约定，注册表已在 `navidog-core` 中建立。

use std::path::PathBuf;

use gpui_kit::App;

/// 插件目录：`<配置目录>/plugins/<插件 id>/`。
pub fn plugins_dir() -> PathBuf {
    navidog_core::paths::config_dir().join("plugins")
}

pub fn init(_cx: &mut App) {}
