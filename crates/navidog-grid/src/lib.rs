//! Result grid and table viewer (21-table-viewer.md). The result grid is reused by the query editor (M3).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod commit;
mod date;
mod editor;
mod filter;
pub mod grid;
pub mod model;
mod viewer;

pub use grid::{GridEvent, GridTarget, ResultGrid, ToggleRawData};
pub use model::{Cell, GridColumn};
pub use viewer::{
    ApplyFilterSort, BeginTransaction, ClearFilterSort, CommitTransaction, FindNext, QueryTable,
    RefreshTable, RollbackTransaction, ShowFind, ShowReplace, SortAscending, SortDescending,
    SortNone, StopLoad, TableViewer, ToggleFilter,
};

pub fn init(cx: &mut gpui_kit::App) {
    grid::init(cx);
    viewer::init(cx);
}

pub(crate) fn db_text(e: &navidog_db::DbError) -> String {
    navidog_ui::db_error_text(
        e.code.as_deref(),
        &e.message,
        e.disconnected,
        e.outcome_unknown,
    )
}
