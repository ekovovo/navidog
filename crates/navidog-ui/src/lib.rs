//! navidog's design system: the Navicat theme, named tokens, AppKit-look skins, icon assets,
//! and the editor tabs shared by every panel in the object pane.

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod assets;
pub mod find_bar;
pub mod skin;
pub mod tabs;
pub mod theme;
pub mod tokens;

pub use assets::NavidogAssets;
use rust_i18n::t;
pub use tokens::{Pt, palette};

/// User-facing text for a driver error. The database message stays as the server sent it;
/// a dead session adds a localized note about reopening.
pub fn db_error_text(
    code: Option<&str>,
    message: &str,
    disconnected: bool,
    outcome_unknown: bool,
) -> String {
    let mut text = match code {
        Some(code) => format!("{code} - {message}"),
        None => message.to_string(),
    };
    if disconnected {
        let note = if outcome_unknown {
            t!("ui.outcome_unknown")
        } else {
            t!("ui.session_lost")
        };
        text.push(' ');
        text.push_str(note.as_ref());
    }
    text
}
