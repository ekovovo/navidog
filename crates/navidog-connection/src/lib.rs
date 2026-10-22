//! Connection management: type picker, connection form, secret resolution and connection commands.

rust_i18n::i18n!("locales", fallback = "en");

mod form;
mod manage;
mod picker;
pub mod profile;
mod reload;
mod secrets;

pub use secrets::resolve;

use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, InteractiveElement, SharedString, Window};
use navidog_core::actions::*;
use navidog_core::connection::SecretField;
use navidog_core::registry::{CommandInfo, CommandRegistry, Source};
use navidog_core::store::{ConnectionStore, StoreError};
use navidog_core::workspace::{Selection, Workspace};
use navidog_db::sessions::Sessions;
use rust_i18n::t;

pub fn init(cx: &mut App) {
    let commands = &mut cx.global_mut::<CommandRegistry>().0;
    let _ = commands.register(
        "connection.new",
        Source::Builtin,
        CommandInfo {
            title: "conn.new_connection".into(),
            icon: Some("icons/plug.svg".into()),
            shortcut: None,
            action: || Box::new(NewConnection),
            enabled: None,
        },
    );
}

/// Opens the connections marked "auto connect" at startup.
pub fn auto_connect(window: &mut Window, cx: &mut App) {
    let ids: Vec<_> = ConnectionStore::global(cx)
        .read(cx)
        .connections()
        .iter()
        .filter(|c| c.advanced.auto_connect)
        .map(|c| c.id.clone())
        .collect();
    for id in ids {
        open_connection(id, window, cx);
    }
}

/// Binds the connection commands on the window root, shared by the menu bar, toolbar and context menus.
pub fn bind_actions<E: InteractiveElement>(el: E) -> E {
    el.on_action(|_: &NewConnection, window, cx| picker::open(window, cx))
        .on_action(|a: &NewConnectionOf, window, cx| form::open_new(a.driver.clone(), window, cx))
        .on_action(|_: &EditConnection, window, cx| edit_selected(window, cx))
        .on_action(|_: &DuplicateConnection, window, cx| {
            let Some(id) = selected_connection(cx) else {
                return;
            };
            let store = ConnectionStore::global(cx);
            let result = store.update(cx, |s, cx| s.duplicate(&id, &t!("conn.copy_suffix"), cx));
            if let Err(e) = result {
                show_error(
                    t!("conn.duplicate_failed").into(),
                    store_error_message(&e).into(),
                    window,
                    cx,
                );
            }
        })
        .on_action(|_: &DeleteConnection, window, cx| delete_selected(window, cx))
        .on_action(|a: &navidog_core::actions::OpenConnectionId, window, cx| {
            open_connection(a.id.to_string(), window, cx)
        })
        .on_action(|_: &OpenConnection, window, cx| {
            if let Some(id) = selected_connection(cx) {
                open_connection(id, window, cx);
            }
        })
        .on_action(|_: &CloseConnection, _, cx| {
            if let Some(id) = selected_connection(cx) {
                Sessions::global(cx).update(cx, |s, cx| s.close(&id, cx));
            }
        })
        .on_action(|_: &Refresh, _, cx| {
            if let Some(id) = selected_connection(cx) {
                Sessions::global(cx).update(cx, |s, cx| s.refresh(&id, cx));
            }
        })
        .on_action(|_: &DeleteGroup, window, cx| confirm_delete_group(window, cx))
        .on_action(|a: &SetColor, window, cx| {
            let Some(id) = selected_connection(cx) else {
                return;
            };
            let color = a.color.as_ref().map(|c| c.to_string());
            let result = ConnectionStore::global(cx)
                .update(cx, |s, cx| s.update(&id, |c| c.color = color, cx));
            if let Err(error) = result {
                window.push_notification(store_error_message(&error), cx);
            }
        })
        .on_action(|_: &ToggleStar, window, cx| toggle_star(window, cx))
        .on_action(|_: &ToggleHidden, window, cx| toggle_hidden(window, cx))
        .on_action(|_: &ManageConnections, _, cx| manage::open(cx))
        .on_action(|a: &SwitchProfile, window, cx| {
            if let Some(id) = selected_connection(cx) {
                profile::switch(&id, &a.name, window, cx);
            }
        })
        .on_action(|_: &NewProfile, window, cx| {
            if let Some(id) = selected_connection(cx) {
                profile::new(id, window, cx);
            }
        })
        .on_action(|_: &DeleteProfile, window, cx| {
            if let Some(id) = selected_connection(cx) {
                profile::delete(id, window, cx);
            }
        })
        .on_action(|_: &ReloadConnection, window, cx| {
            if let Some(id) = selected_connection(cx) {
                reload::open(id, window, cx);
            }
        })
        .on_action(|a: &MoveToGroup, window, cx| {
            let ids = selected_connections(cx);
            let group = a.group.as_ref().map(|g| g.to_string());
            let result = ConnectionStore::global(cx).update(cx, |s, cx| {
                s.update_many(&ids, |c| c.group = group.clone(), cx)
            });
            if let Err(error) = result {
                window.push_notification(store_error_message(&error), cx);
            }
        })
}

/// Toolbar and row actions call these directly. `Window::dispatch_action` starts at the
/// focused node, and the manage window's buttons refuse focus on mouse down, so a click
/// before anything else was focused never reached `bind_actions`.
pub(crate) fn edit_selected(window: &mut Window, cx: &mut App) {
    if let Some(id) = selected_connection(cx) {
        form::open_edit(id, window, cx);
    }
}

pub(crate) fn toggle_star(window: &mut Window, cx: &mut App) {
    let ids = selected_connections(cx);
    let store = ConnectionStore::global(cx);
    // Star them all unless every selected connection is starred already.
    let star = ids
        .iter()
        .any(|id| store.read(cx).get(id).is_some_and(|c| !c.starred));
    let result = store.update(cx, |s, cx| s.update_many(&ids, |c| c.starred = star, cx));
    if let Err(error) = result {
        window.push_notification(store_error_message(&error), cx);
    }
}

pub(crate) fn toggle_hidden(window: &mut Window, cx: &mut App) {
    let ids = selected_connections(cx);
    let store = ConnectionStore::global(cx);
    let hide = ids
        .iter()
        .any(|id| store.read(cx).get(id).is_some_and(|c| !c.hidden));
    let result = store.update(cx, |s, cx| s.update_many(&ids, |c| c.hidden = hide, cx));
    if let Err(error) = result {
        window.push_notification(store_error_message(&error), cx);
    }
}

pub(crate) fn delete_selected(window: &mut Window, cx: &mut App) {
    confirm_delete_connection(window, cx);
}

/// All selected connections (⌘-click selects several in the navigator).
fn selected_connections(cx: &App) -> Vec<String> {
    Workspace::global(cx)
        .read(cx)
        .selections()
        .filter_map(|s| match s {
            Selection::Connection(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn selected_connection(cx: &App) -> Option<String> {
    Workspace::global(cx)
        .read(cx)
        .selection()
        .connection_id()
        .cloned()
}

/// Opens a connection after resolving its secrets (unlocking the master password or asking for passwords if needed).
pub fn open_connection(id: String, window: &mut Window, cx: &mut App) {
    let target = id.clone();
    secrets::resolve(id, window, cx, move |secrets, _, cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(target, secrets, cx));
    });
}

/// Closes and opens again so new settings take effect. Editor tabs stay and reconnect, unlike a
/// real close (`SessionsEvent::Reopening`).
pub(crate) fn reopen_connection(id: String, window: &mut Window, cx: &mut App) {
    Sessions::global(cx).update(cx, |s, cx| s.close_for_reopen(&id, cx));
    open_connection(id, window, cx);
}

fn confirm_delete_connection(window: &mut Window, cx: &mut App) {
    let Some(id) = selected_connection(cx) else {
        return;
    };
    let Some(name) = ConnectionStore::global(cx)
        .read(cx)
        .get(&id)
        .map(|c| c.name.clone())
    else {
        return;
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let id = id.clone();
        alert
            .icon(navidog_ui::assets::lucide("triangle-alert"))
            .title(SharedString::from(
                t!("conn.confirm_delete", name = name).to_string(),
            ))
            .show_cancel(true)
            .ok_text(t!("conn.delete").to_string())
            .cancel_text(t!("conn.cancel").to_string())
            .on_ok(move |_, window, cx| {
                let result = ConnectionStore::global(cx).update(cx, |s, cx| s.remove(&id, cx));
                if let Err(error) = result {
                    window.push_notification(store_error_message(&error), cx);
                    return false;
                }
                Sessions::global(cx).update(cx, |s, cx| s.close(&id, cx));
                navidog_core::vgroups::VirtualGroups::drop_connection(&id, cx);
                navidog_db::outline::OutlineCache::drop_connection(&id, cx);
                let _ = std::fs::remove_dir_all(navidog_core::paths::profile_dir(&id));
                Workspace::global(cx).update(cx, |w, cx| w.select(Selection::None, cx));
                true
            })
    });
}

fn confirm_delete_group(window: &mut Window, cx: &mut App) {
    let Selection::Group(id) = Workspace::global(cx).read(cx).selection().clone() else {
        return;
    };
    let Some(name) = ConnectionStore::global(cx)
        .read(cx)
        .groups()
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.name.clone())
    else {
        return;
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let id = id.clone();
        alert
            .icon(navidog_ui::assets::lucide("triangle-alert"))
            .title(SharedString::from(
                t!("conn.confirm_delete_group", name = name).to_string(),
            ))
            .show_cancel(true)
            .ok_text(t!("conn.delete").to_string())
            .cancel_text(t!("conn.cancel").to_string())
            .on_ok(move |_, window, cx| {
                let result =
                    ConnectionStore::global(cx).update(cx, |s, cx| s.remove_group(&id, cx));
                if let Err(error) = result {
                    window.push_notification(store_error_message(&error), cx);
                    return false;
                }
                Workspace::global(cx).update(cx, |w, cx| w.select(Selection::None, cx));
                true
            })
    });
}

/// A store or key error in the interface language. The errors' own text is English, for logs.
pub fn store_error_message(e: &StoreError) -> String {
    navidog_core::store::error_message(e)
}

pub(crate) fn db_text(e: &navidog_db::DbError) -> String {
    navidog_ui::db_error_text(
        e.code.as_deref(),
        &e.message,
        e.disconnected,
        e.outcome_unknown,
    )
}

pub(crate) fn show_error(
    title: SharedString,
    message: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .icon(navidog_ui::assets::lucide("circle-x"))
            .title(title.clone())
            .description(message.clone())
            .ok_text(t!("conn.ok").to_string())
    });
}

pub(crate) fn driver_name(driver: &str, cx: &App) -> SharedString {
    navidog_db::DriverRegistry::get(cx, driver)
        .map(|d| d.info().name.clone())
        .unwrap_or_else(|| driver.to_string().into())
}

pub(crate) fn drivers(cx: &App) -> Vec<Arc<dyn navidog_db::Driver>> {
    navidog_db::DriverRegistry::all(cx)
}

pub(crate) fn secret_label(field: SecretField) -> String {
    match field {
        SecretField::Password => t!("conn.password").to_string(),
        SecretField::SshPassword => t!("conn.ssh_password").to_string(),
        SecretField::SshPassphrase => t!("conn.passphrase").to_string(),
    }
}
