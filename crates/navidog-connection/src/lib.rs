//! Connection management: type picker, connection form, secret resolution and connection commands.

rust_i18n::i18n!("locales", fallback = "en");
// The proc-macro reads locales itself; include them so Cargo also tracks future edits.
const _: &str = include_str!("../locales/connection.yml");

mod form;
mod manage;
mod picker;
pub mod profile;
mod reload;
mod secrets;

pub use form::open_new;
pub use secrets::{request_unlock, resolve};

pub fn open_type_picker(window: &mut Window, cx: &mut App) {
    picker::open(window, cx);
}

use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Action, App, InteractiveElement, SharedString, Window};
use navidog_app_state::actions::*;
use navidog_app_state::registry::{
    CommandGroup, CommandInfo, CommandRegistry, CommandTarget, CommandTitle, Source,
};
use navidog_app_state::store::ConnectionStore;
use navidog_app_state::workspace::{Selection, Workspace};
use navidog_model::connection::SecretField;
use navidog_session::sessions::Sessions;
use rust_i18n::t;

pub fn init(cx: &mut App) {
    let commands = &mut cx.global_mut::<CommandRegistry>().0;
    type Entry = (
        &'static str,
        &'static str,
        fn() -> Box<dyn Action>,
        CommandTarget,
        Option<&'static str>,
    );
    let entries: [Entry; 9] = [
        (
            "connection.new",
            "conn.new_connection",
            || Box::new(NewConnection),
            CommandTarget::None,
            Some("icons/plug.svg"),
        ),
        (
            "connection.edit",
            "conn.edit",
            || Box::new(EditConnection),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.duplicate",
            "conn.duplicate",
            || Box::new(DuplicateConnection),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.delete",
            "conn.delete",
            || Box::new(DeleteConnection),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.open",
            "conn.open",
            || Box::new(OpenConnection),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.close",
            "conn.close",
            || Box::new(CloseConnection),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.refresh",
            "conn.refresh",
            || Box::new(Refresh),
            CommandTarget::Connection,
            None,
        ),
        (
            "connection.new_group",
            "conn.new_group",
            || Box::new(NewGroup),
            CommandTarget::Workspace,
            None,
        ),
        (
            "connection.manage",
            "conn.manage_title",
            || Box::new(ManageConnections),
            CommandTarget::None,
            None,
        ),
    ];
    for (id, title, action, target, icon) in entries {
        let info = CommandInfo::new(
            CommandGroup::Connection,
            CommandTitle::localized(title, translated_command_title),
            action,
        )
        .target(target);
        let info = if let Some(icon) = icon {
            info.icon(icon)
        } else {
            info
        };
        commands
            .register(id, Source::Builtin, info)
            .unwrap_or_else(|error| panic!("built-in connection commands: {error}"));
    }
}

fn translated_command_title(key: &str) -> SharedString {
    t!(key).to_string().into()
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
            let run = move |id: String, window: &mut Window, cx: &mut App| {
                let suffix = t!("conn.copy_suffix").to_string();
                let result =
                    ConnectionStore::global(cx).update(cx, |s, cx| s.duplicate(&id, &suffix, cx));
                if let Err(e) = result {
                    show_error(
                        t!("conn.duplicate_failed").into(),
                        e.localized_message().into(),
                        window,
                        cx,
                    );
                }
            };
            if ConnectionStore::global(cx).read(cx).is_locked() {
                request_unlock(window, cx, move |ok, window, cx| {
                    if ok {
                        run(id, window, cx);
                    } else {
                        show_error(
                            t!("conn.duplicate_failed").into(),
                            navidog_app_state::store::StoreError::Locked
                                .localized_message()
                                .into(),
                            window,
                            cx,
                        );
                    }
                });
            } else {
                run(id, window, cx);
            }
        })
        .on_action(|_: &DeleteConnection, window, cx| delete_selected(window, cx))
        .on_action(
            |a: &navidog_app_state::actions::OpenConnectionId, window, cx| {
                open_connection(a.id.to_string(), window, cx)
            },
        )
        .on_action(|a: &ReopenConnectionId, window, cx| {
            confirm_reopen_connection(a.id.to_string(), window, cx);
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
                window.push_notification(error.localized_message(), cx);
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
                window.push_notification(error.localized_message(), cx);
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
        window.push_notification(error.localized_message(), cx);
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
        window.push_notification(error.localized_message(), cx);
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

fn confirm_reopen_connection(id: String, window: &mut Window, cx: &mut App) {
    let Some(name) = ConnectionStore::global(cx)
        .read(cx)
        .get(&id)
        .map(|connection| connection.name.clone())
    else {
        return;
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let id = id.clone();
        alert
            .icon(navidog_ui::assets::lucide("triangle-alert"))
            .title(t!("conn.confirm_reopen", name = name).to_string())
            .description(t!("conn.reopen_warning").to_string())
            .show_cancel(true)
            .ok_text(t!("conn.reopen").to_string())
            .cancel_text(t!("conn.cancel").to_string())
            .on_ok(move |_, window, cx| {
                let id = id.clone();
                // Close this confirmation before resolving passwords, which can open a dialog.
                window.defer(cx, move |window, cx| reopen_connection(id, window, cx));
                true
            })
    });
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
                    window.push_notification(error.localized_message(), cx);
                    return false;
                }
                Sessions::global(cx).update(cx, |s, cx| s.close(&id, cx));
                navidog_app_state::vgroups::VirtualGroups::drop_connection(&id, cx);
                navidog_session::outline::OutlineCache::drop_connection(&id, cx);
                let _ = std::fs::remove_dir_all(navidog_model::paths::profile_dir(&id));
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
                    window.push_notification(error.localized_message(), cx);
                    return false;
                }
                Workspace::global(cx).update(cx, |w, cx| w.select(Selection::None, cx));
                true
            })
    });
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

pub(crate) fn drivers(cx: &App) -> Vec<Arc<dyn navidog_db::Driver>> {
    navidog_session::DriverRegistry::all(cx)
}

pub(crate) fn secret_label(field: SecretField) -> String {
    match field {
        SecretField::Password => t!("conn.password").to_string(),
        SecretField::SshPassword => t!("conn.ssh_password").to_string(),
        SecretField::SshPassphrase => t!("conn.passphrase").to_string(),
    }
}

pub(crate) fn db_text(e: &navidog_db::DbError) -> String {
    navidog_ui::db_error_text(
        e.code.as_deref(),
        &e.message,
        e.disconnected,
        e.outcome_unknown,
    )
}
