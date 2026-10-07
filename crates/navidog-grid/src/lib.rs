//! 结果网格与表查看器（21-table-viewer.md）。结果网格会被查询编辑器（M3）复用。

// Action 派生宏输出 `gpui::` 路径
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod grid;
pub mod model;
mod viewer;

pub use grid::{GridEvent, GridTarget, ResultGrid, ToggleRawData};
pub use viewer::TableViewer;

pub fn init(cx: &mut gpui_kit::App) {
    grid::init(cx);
    viewer::init(cx);
}
