use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

// 主工具栏、导航树等用到、但不在 gpui-kit 默认 101 个图标里的 Lucide 图标
icon_assets!(pub ExtraIcons, [
    Plug, SquareTerminal, Table, Eye, SquareFunction, User, Shapes, FileCode,
    DatabaseBackup, CalendarClock, Workflow, ChartPie, Database, Folder, FolderOpen,
    Server, Star, RefreshCw, Layers, Maximize2, Minimize2, CircleCheck, TriangleAlert,
    Plus, Minus, X, Search, Cable, Unplug, Palette, FolderInput, Copy, Trash, Pencil,
]);

/// 自绘的彩色图标，按 `img()` 方式渲染（`svg()` 只能单色着色）。
const OWN: &[(&str, &[u8])] = &[
    (
        "icons/db/mysql.svg",
        include_bytes!("../assets/icons/db/mysql.svg"),
    ),
    (
        "icons/db/mariadb.svg",
        include_bytes!("../assets/icons/db/mariadb.svg"),
    ),
    (
        "icons/db/postgresql.svg",
        include_bytes!("../assets/icons/db/postgresql.svg"),
    ),
    (
        "icons/db/sqlite.svg",
        include_bytes!("../assets/icons/db/sqlite.svg"),
    ),
];

/// 应用资源：自绘图标 → 挑选的 Lucide 图标 → gpui-kit 默认资源。
/// `Assets` 遇到未知路径会报错，所以必须放在最后。
#[derive(Clone, Copy, Default)]
pub struct NavidogAssets;

impl AssetSource for NavidogAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items: Vec<SharedString> = OWN
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect();
        items.extend(ExtraIcons.list(path)?);
        items.extend(Assets.list(path)?);
        items.sort();
        items.dedup();
        Ok(items)
    }
}

/// 按 Lucide 名称取图标（`IconName` 只覆盖默认的 101 个图标，其余走 `ExtraIcons`）。
pub fn lucide(name: &str) -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().path(format!("icons/{name}.svg"))
}

/// 数据库类型图标路径。
pub fn driver_icon(driver: &str) -> &'static str {
    match driver {
        "mariadb" => "icons/db/mariadb.svg",
        "postgres" => "icons/db/postgresql.svg",
        "sqlite" => "icons/db/sqlite.svg",
        _ => "icons/db/mysql.svg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_own_lucide_and_default_icons() {
        let a = NavidogAssets;
        assert!(a.load("icons/db/sqlite.svg").unwrap().is_some());
        assert!(a.load("icons/plug.svg").unwrap().is_some());
        assert!(a.load("icons/check.svg").unwrap().is_some());
        assert!(a.list("icons/db/").unwrap().len() == 4);
    }
}
