//! Query editor, results, explain, console and history log viewer (30-query-editor.md,
//! 31-query-results.md).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

mod completion;
mod editor;
mod explain;
mod explain_visual;
mod find;
mod format;
mod params;
mod recovery;
mod results;
mod run;
mod save;
mod snippet;

#[cfg(test)]
mod editor_tests;

pub use editor::*;
pub use find::FindPrevious;
pub use format::{
    Beautify, BeautifyOptionsDialog, CommentLines, LowerCase, Minify, PasteFromStack,
    ToggleComment, UncommentLines, UpperCase,
};

pub use recovery::{check as check_recovery, flush_on_quit};
pub use save::{SaveAsExternal, SaveAsNavicat, SaveQuery, SaveQueryAs, editor_for};

pub fn init(cx: &mut gpui_kit::App) {
    editor::init(cx);
    save::init(cx);
    recovery::init(cx);
}
