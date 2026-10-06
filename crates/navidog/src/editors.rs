//! 按对象类型选择在对象窗格中打开的编辑器（外壳只负责组合各 feature 的面板）。

use std::sync::Arc;

use gpui_kit::component::dock::{BasePanelView, panel_handle};
use gpui_kit::{App, Window};
use navidog_core::object::{ObjectKind, ObjectRef};
use navidog_grid::TableViewer;
use navidog_navigator::DdlPanel;

pub fn open(object: ObjectRef, window: &mut Window, cx: &mut App) -> Arc<dyn BasePanelView> {
    match object.kind {
        ObjectKind::Table => panel_handle(TableViewer::new(object, window, cx)),
        _ => panel_handle(DdlPanel::new(object, window, cx)),
    }
}
