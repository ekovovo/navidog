//! Domain model, config directory, encrypted storage, registries and cross-feature actions.
//! Depends on no UI components. Localized store errors live here so every feature crate
//! can show them without depending on one another.

rust_i18n::i18n!("locales", fallback = "en");
// The proc-macro reads locales itself; include them so Cargo also tracks future edits.
const _: &str = include_str!("../locales/core.yml");

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

pub mod actions;
pub mod column_widths;
pub mod connection;
pub mod crypto;
pub mod favorites;
pub mod object;
pub mod paths;
pub mod queries;
pub mod registry;
pub mod settings;
pub mod snippets;
pub mod store;
pub mod uri;
pub mod vgroups;
pub mod workspace;

use gpui_kit::App;

/// Initialize global state: settings, connection store, registries, favorites, virtual groups and
/// the workspace selection.
pub fn init(cx: &mut App) {
    let _ = _rust_i18n_backend();
    settings::init(cx);
    registry::init(cx);
    store::init(cx);
    favorites::init(cx);
    vgroups::init(cx);
    snippets::init(cx);
    workspace::init(cx);
}
