//! Query editor, results, explain, console and history log viewer (30-query-editor.md,
//! 31-query-results.md).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub fn init(_cx: &mut gpui_kit::App) {}
