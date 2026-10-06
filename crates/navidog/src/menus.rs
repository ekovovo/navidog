//! 快捷键与菜单栏。`bind_keys` 必须在 `set_menus` 之前调用，菜单才会显示快捷键。

use gpui_kit::base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType};
use navidog_connection::{CloseConnection, NewConnection, NewConnectionOf, NewGroup, Refresh};
use rust_i18n::t;

use crate::actions::*;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", Preferences, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("shift-cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-1", ToggleNavigator, None),
        KeyBinding::new("cmd-2", ToggleInfoPane, None),
        KeyBinding::new("alt-cmd-t", ToggleToolbar, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("shift-cmd-.", CloseConnection, None),
    ]);
}

pub fn set_menus(cx: &mut App) {
    let state = cx
        .try_global::<crate::state::AppState>()
        .cloned()
        .unwrap_or_default();
    let drivers = navidog_db::DriverRegistry::all(cx);
    let new_connection =
        Menu::new(t!("menu.connection").to_string()).items(drivers.iter().map(|d| {
            let info = d.info();
            MenuItem::action(
                info.name.clone(),
                NewConnectionOf {
                    driver: info.id.clone(),
                },
            )
        }));
    let checked =
        |name: String, action: Box<dyn gpui_kit::Action>, checked: bool| MenuItem::Action {
            name: name.into(),
            action,
            os_action: None,
            checked,
            disabled: false,
        };
    cx.set_menus(vec![
        Menu::new("navidog").items([
            MenuItem::action(t!("menu.about").to_string(), About),
            MenuItem::separator(),
            MenuItem::action(t!("menu.preferences").to_string(), Preferences),
            MenuItem::separator(),
            MenuItem::action(t!("menu.check_updates").to_string(), CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu(t!("menu.services").to_string(), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(t!("menu.hide").to_string(), Hide),
            MenuItem::action(t!("menu.hide_others").to_string(), HideOthers),
            MenuItem::action(t!("menu.show_all").to_string(), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t!("menu.quit").to_string(), Quit),
        ]),
        Menu::new(t!("menu.file").to_string()).items([
            MenuItem::submenu(Menu::new(t!("menu.new").to_string()).items([
                MenuItem::submenu(new_connection),
                MenuItem::action(t!("menu.connection").to_string() + "…", NewConnection),
                MenuItem::separator(),
                MenuItem::action(t!("menu.group").to_string(), NewGroup),
            ])),
            MenuItem::separator(),
            MenuItem::action(t!("menu.close_tab").to_string(), CloseTab),
            MenuItem::action(t!("menu.close_window").to_string(), CloseWindow),
        ]),
        Menu::new(t!("menu.edit").to_string()).items([
            MenuItem::os_action(t!("menu.undo").to_string(), Undo, OsAction::Undo),
            MenuItem::os_action(t!("menu.redo").to_string(), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t!("menu.cut").to_string(), Cut, OsAction::Cut),
            MenuItem::os_action(t!("menu.copy").to_string(), Copy, OsAction::Copy),
            MenuItem::os_action(t!("menu.paste").to_string(), Paste, OsAction::Paste),
            MenuItem::os_action(
                t!("menu.select_all").to_string(),
                SelectAll,
                OsAction::SelectAll,
            ),
        ]),
        Menu::new(t!("menu.view").to_string()).items([
            MenuItem::submenu(Menu::new(t!("menu.navigation_pane").to_string()).items([
                MenuItem::action(t!("menu.show_navigation_pane").to_string(), ToggleNavigator),
                checked(
                    t!("menu.hide_groups").to_string(),
                    Box::new(ToggleHideGroups),
                    state.hide_groups,
                ),
                checked(
                    t!("menu.filter_on_top").to_string(),
                    Box::new(ToggleFilterOnTop),
                    state.filter_on_top,
                ),
            ])),
            MenuItem::submenu(Menu::new(t!("menu.info_pane").to_string()).items([
                MenuItem::action(t!("menu.show_info_pane").to_string(), ToggleInfoPane),
            ])),
            MenuItem::separator(),
            checked(
                t!("menu.show_status_bar").to_string(),
                Box::new(ToggleStatusBar),
                state.show_status_bar,
            ),
            checked(
                t!("menu.show_toolbar").to_string(),
                Box::new(ToggleToolbar),
                state.show_toolbar,
            ),
            MenuItem::separator(),
            MenuItem::action(t!("menu.full_screen").to_string(), ToggleFullScreen),
        ]),
        Menu::new(t!("menu.window").to_string()).items([
            MenuItem::action(t!("menu.minimize").to_string(), Minimize),
            MenuItem::action(t!("menu.zoom").to_string(), Zoom),
            MenuItem::separator(),
            MenuItem::action(t!("menu.reset_layout").to_string(), ResetLayout),
            MenuItem::separator(),
            MenuItem::action(t!("menu.show_main_window").to_string(), ShowMainWindow),
            MenuItem::action(t!("menu.bring_all_to_front").to_string(), BringAllToFront),
        ]),
        Menu::new(t!("menu.help").to_string()).items([
            MenuItem::action(t!("menu.navidog_help").to_string(), OpenHelp),
            MenuItem::action(t!("menu.report_issue").to_string(), ReportIssue),
        ]),
    ]);
}
