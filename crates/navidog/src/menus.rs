//! Key bindings and the menu bar. Call `bind_keys` before `set_menus`, or menus show no shortcuts.

use gpui_kit::base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType};
use navidog_core::actions::{
    CloseConnection, ManageConnections, NewConnection, NewConnectionOf, NewGroup, Refresh,
    ToggleFocusMode, ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden,
    ToggleShowTableProfile,
};
use navidog_core::favorites::Favorites;
use navidog_ui::tabs::{CloseTab, NextTab, PrevTab};
use rust_i18n::t;

use crate::actions::*;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", Preferences, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("cmd-}", NextTab, None),
        KeyBinding::new("cmd-{", PrevTab, None),
        KeyBinding::new("shift-cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-1", ToggleNavigator, None),
        KeyBinding::new("cmd-2", ToggleInfoPane, None),
        KeyBinding::new("alt-cmd-t", ToggleToolbar, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("shift-cmd-.", CloseConnection, None),
        KeyBinding::new("cmd-u", OpenUri, None),
    ]);
    for id in 1..=9u8 {
        cx.bind_keys([
            KeyBinding::new(&format!("alt-shift-{id}"), AddFavoriteWithId { id }, None),
            KeyBinding::new(&format!("ctrl-shift-{id}"), OpenFavoriteId { id }, None),
        ]);
    }
}

/// Favorites menu (F-APP-011): add, the favorites (⌃⇧<id> shown for those with an ID), clear.
fn favorites_menu(cx: &App) -> Menu {
    let favorites = Favorites::global(cx).read(cx).favorites().to_vec();
    let mut items = vec![MenuItem::action(
        t!("menu.add_favorite").to_string(),
        AddActiveToFavorites,
    )];
    if !favorites.is_empty() {
        items.push(MenuItem::separator());
    }
    items.extend(favorites.iter().enumerate().map(|(ix, f)| match f.id {
        Some(id) => MenuItem::action(f.name.clone(), OpenFavoriteId { id }),
        None => MenuItem::action(f.name.clone(), OpenFavorite { ix }),
    }));
    let mut clear: Vec<MenuItem> = favorites
        .iter()
        .enumerate()
        .map(|(ix, f)| MenuItem::action(f.name.clone(), RemoveFavorite { ix }))
        .collect();
    if !clear.is_empty() {
        clear.push(MenuItem::separator());
    }
    clear.push(MenuItem::action(
        t!("menu.clear_all").to_string(),
        ClearFavorites,
    ));
    items.push(MenuItem::separator());
    items.push(MenuItem::submenu(
        Menu::new(t!("menu.clear_favorites").to_string()).items(clear),
    ));
    Menu::new(t!("menu.favorites").to_string()).items(items)
}

/// File > Open Recent (F-APP-013).
fn recent_menu(cx: &App) -> Menu {
    let recent = Favorites::global(cx).read(cx).recent().to_vec();
    let mut items: Vec<MenuItem> = recent
        .iter()
        .enumerate()
        .map(|(ix, o)| {
            MenuItem::action(
                format!(
                    "{}.{}",
                    o.scope.schema.as_ref().unwrap_or(&o.scope.database),
                    o.name
                ),
                OpenRecent { ix },
            )
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::action(
        t!("menu.clear_recent").to_string(),
        ClearRecent,
    ));
    Menu::new(t!("menu.open_recent").to_string()).items(items)
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
            MenuItem::submenu(recent_menu(cx)),
            MenuItem::action(t!("menu.open_uri").to_string(), OpenUri),
            MenuItem::separator(),
            MenuItem::action(t!("menu.manage_connections").to_string(), ManageConnections),
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
            MenuItem::separator(),
            MenuItem::submenu(Menu::new(t!("menu.find").to_string()).items([
                MenuItem::action(t!("menu.find_find").to_string(), navidog_grid::ShowFind),
                MenuItem::action(
                    t!("menu.find_replace").to_string(),
                    navidog_grid::ShowReplace,
                ),
                MenuItem::action(t!("menu.find_next").to_string(), navidog_grid::FindNext),
            ])),
        ]),
        Menu::new(t!("menu.view").to_string()).items([
            MenuItem::submenu(Menu::new(t!("menu.navigation_pane").to_string()).items([
                MenuItem::action(t!("menu.show_navigation_pane").to_string(), ToggleNavigator),
                checked(
                    t!("menu.hide_groups").to_string(),
                    Box::new(ToggleHideGroups),
                    state.view.hide_groups,
                ),
                checked(
                    t!("menu.filter_on_top").to_string(),
                    Box::new(ToggleFilterOnTop),
                    state.view.filter_on_top,
                ),
                checked(
                    t!("menu.only_active").to_string(),
                    Box::new(ToggleOnlyActive),
                    state.view.only_active,
                ),
            ])),
            checked(
                t!("menu.show_hidden").to_string(),
                Box::new(ToggleShowHidden),
                state.view.show_hidden,
            ),
            checked(
                t!("menu.hide_object_groups").to_string(),
                Box::new(ToggleHideObjectGroups),
                state.view.hide_object_groups,
            ),
            checked(
                t!("menu.show_table_profile").to_string(),
                Box::new(ToggleShowTableProfile),
                state.view.show_table_profile,
            ),
            MenuItem::submenu(Menu::new(t!("menu.info_pane").to_string()).items([
                MenuItem::action(t!("menu.show_info_pane").to_string(), ToggleInfoPane),
            ])),
            MenuItem::separator(),
            MenuItem::action(t!("menu.raw_data").to_string(), navidog_grid::ToggleRawData),
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
            MenuItem::action(t!("menu.focus_mode").to_string(), ToggleFocusMode),
            MenuItem::action(t!("menu.full_screen").to_string(), ToggleFullScreen),
        ]),
        favorites_menu(cx),
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
