//! Result grid and table viewer (21-table-viewer.md). The result grid is reused by the query editor (M3).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

mod date;
mod filter;
pub mod grid;
pub mod model;
mod viewer;

pub use grid::{GridEvent, GridTarget, ResultGrid, ToggleRawData};
pub use viewer::TableViewer;

pub fn init(cx: &mut gpui_kit::App) {
    grid::init(cx);
    viewer::init(cx);
}
