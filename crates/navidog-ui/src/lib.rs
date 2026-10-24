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
    db_error_text_with(
        code,
        message,
        disconnected,
        outcome_unknown,
        |detail| format!("{detail} {}", t!("ui.session_lost")),
        |detail| format!("{detail} {}", t!("ui.outcome_unknown")),
    )
}

/// Formats the original driver diagnostic and delegates recovery copy to its UI surface.
/// Unknown execution outcomes take precedence over session loss. A healthy-session error
/// returns the diagnostic unchanged and does not invoke either recovery formatter.
pub fn db_error_text_with(
    code: Option<&str>,
    message: &str,
    disconnected: bool,
    outcome_unknown: bool,
    session_lost_text: impl FnOnce(&str) -> String,
    outcome_unknown_text: impl FnOnce(&str) -> String,
) -> String {
    let detail = match code {
        Some(code) => format!("{code} - {message}"),
        None => message.to_string(),
    };
    if outcome_unknown {
        outcome_unknown_text(&detail)
    } else if disconnected {
        session_lost_text(&detail)
    } else {
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::db_error_text_with;

    #[test]
    fn database_recovery_selects_one_formatter_and_preserves_the_diagnostic() {
        let message = "original driver diagnostic\n数据库诊断";
        for code in [None, Some("08006")] {
            let expected = match code {
                Some(code) => format!("{code} - {message}"),
                None => message.to_string(),
            };
            assert_eq!(
                db_error_text_with(
                    code,
                    message,
                    false,
                    false,
                    |_| panic!("a plain diagnostic needs no recovery text"),
                    |_| panic!("a plain diagnostic needs no recovery text"),
                ),
                expected,
            );
            assert_eq!(
                db_error_text_with(
                    code,
                    message,
                    true,
                    false,
                    |detail| format!("session lost: {detail}"),
                    |_| panic!("a known outcome must not use unknown-outcome text"),
                ),
                format!("session lost: {expected}"),
            );
            for disconnected in [false, true] {
                assert_eq!(
                    db_error_text_with(
                        code,
                        message,
                        disconnected,
                        true,
                        |_| panic!("unknown outcomes take precedence over session loss"),
                        |detail| format!("outcome unknown: {detail}"),
                    ),
                    format!("outcome unknown: {expected}"),
                );
            }
        }
    }
}
