//! navidog's design system: the Navicat theme, named tokens, AppKit-look skins, icon assets,
//! and the editor tabs shared by every panel in the object pane.

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod assets;
pub mod skin;
pub mod tabs;
pub mod theme;
pub mod tokens;

pub use assets::NavidogAssets;
pub use tokens::{Pt, palette};
