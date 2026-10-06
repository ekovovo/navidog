//! Domain model, config directory, encrypted storage, registries and cross-feature actions.
//! Depends on no UI components.

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

pub mod actions;
pub mod connection;
pub mod crypto;
pub mod favorites;
pub mod object;
pub mod paths;
pub mod registry;
pub mod settings;
pub mod store;
pub mod uri;
pub mod vgroups;
pub mod workspace;

use gpui_kit::App;

/// Initialize global state: settings, connection store, registries, favorites, virtual groups and
/// the workspace selection.
pub fn init(cx: &mut App) {
    settings::init(cx);
    registry::init(cx);
    store::init(cx);
    favorites::init(cx);
    vgroups::init(cx);
    workspace::init(cx);
}
