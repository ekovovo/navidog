//! 连接管理：类型选择、连接表单、秘密解析、连接组，以及连接命令的处理。

rust_i18n::i18n!("locales", fallback = "en");

mod form;
mod groups;
mod picker;
mod secrets;

use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, InteractiveElement, SharedString, Window};
use navidog_core::actions::*;
use navidog_core::connection::SecretField;
use navidog_core::registry::{CommandInfo, CommandRegistry, Source};
use navidog_core::store::ConnectionStore;
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

/// 启动时打开勾选了“自动连接”的连接。
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

/// 在窗口根元素上绑定连接命令，菜单栏、工具栏、右键菜单共用。
pub fn bind_actions<E: InteractiveElement>(el: E) -> E {
    el.on_action(|_: &NewConnection, window, cx| picker::open(window, cx))
        .on_action(|a: &NewConnectionOf, window, cx| form::open_new(a.driver.clone(), window, cx))
        .on_action(|_: &EditConnection, window, cx| {
            if let Some(id) = selected_connection(cx) {
                form::open_edit(id, window, cx);
            }
        })
        .on_action(|_: &DuplicateConnection, window, cx| {
            let Some(id) = selected_connection(cx) else {
                return;
            };
            let store = ConnectionStore::global(cx);
            let result = store.update(cx, |s, cx| s.duplicate(&id, &t!("conn.copy_suffix"), cx));
            if let Err(e) = result {
                show_error(
                    t!("conn.open_failed").into(),
                    e.to_string().into(),
                    window,
                    cx,
                );
            }
        })
        .on_action(|_: &DeleteConnection, window, cx| confirm_delete_connection(window, cx))
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
        .on_action(|_: &NewGroup, window, cx| groups::new_group(window, cx))
        .on_action(|_: &RenameGroup, window, cx| groups::rename_group(window, cx))
        .on_action(|_: &DeleteGroup, window, cx| confirm_delete_group(window, cx))
        .on_action(|a: &SetColor, _, cx| {
            let Some(id) = selected_connection(cx) else {
                return;
            };
            let color = a.color.as_ref().map(|c| c.to_string());
            ConnectionStore::global(cx).update(cx, |s, cx| s.update(&id, |c| c.color = color, cx));
        })
        .on_action(|a: &MoveToGroup, _, cx| {
            let Some(id) = selected_connection(cx) else {
                return;
            };
            let group = a.group.as_ref().map(|g| g.to_string());
            ConnectionStore::global(cx).update(cx, |s, cx| s.move_to_group(&id, group, cx));
        })
}

pub(crate) fn selected_connection(cx: &App) -> Option<String> {
    Workspace::global(cx)
        .read(cx)
        .selection()
        .connection_id()
        .cloned()
}

/// 解析秘密字段（必要时解锁主密码、询问密码）后打开连接。
pub fn open_connection(id: String, window: &mut Window, cx: &mut App) {
    let target = id.clone();
    secrets::resolve(id, window, cx, move |secrets, _, cx| {
        Sessions::global(cx).update(cx, |s, cx| s.open(target, secrets, cx));
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
            .on_ok(move |_, _, cx| {
                Sessions::global(cx).update(cx, |s, cx| s.close(&id, cx));
                ConnectionStore::global(cx).update(cx, |s, cx| s.remove(&id, cx));
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
            .on_ok(move |_, _, cx| {
                ConnectionStore::global(cx).update(cx, |s, cx| s.remove_group(&id, cx));
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
