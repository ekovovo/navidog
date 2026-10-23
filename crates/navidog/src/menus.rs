//! Key bindings and the menu bar. Call `bind_keys` before `set_menus`, or menus show no shortcuts.

use gpui_kit::base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType};
use navidog_core::actions::{
    CloseConnection, ManageConnections, NewConnection, NewConnectionOf, NewGroup, NewQuery,
    OpenConsole, OpenExternalQuery, OpenHistoryLog, Refresh, ToggleFocusMode,
    ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden, ToggleShowTableProfile,
};
use navidog_core::favorites::Favorites;
use navidog_core::settings::{ResultsPosition, Settings, ShowAfterRun};
use navidog_query as query;
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
        // No context matches at the full stack depth and would beat Run / RefreshTable,
        // which are bound on QueryEditor and TableViewer. ⌘R stays refresh everywhere else.
        KeyBinding::new("cmd-r", Refresh, Some("!QueryEditor && !TableViewer")),
        KeyBinding::new("shift-cmd-.", CloseConnection, None),
        KeyBinding::new("cmd-u", OpenUri, None),
        KeyBinding::new("cmd-y", NewQuery, None),
        KeyBinding::new("shift-cmd-c", OpenConsole, None),
        KeyBinding::new("shift-cmd-h", OpenHistoryLog, None),
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

fn checked(name: String, action: Box<dyn gpui_kit::Action>, on: bool) -> MenuItem {
    MenuItem::Action {
        name: name.into(),
        action,
        os_action: None,
        checked: on,
        disabled: false,
    }
}

fn format_menu() -> Menu {
    Menu::new(t!("menu.format").to_string()).items([
        MenuItem::submenu(Menu::new(t!("menu.indent").to_string()).items([
            MenuItem::action(
                t!("menu.indent_more").to_string(),
                gpui_kit::component::input::Indent,
            ),
            MenuItem::action(
                t!("menu.indent_less").to_string(),
                gpui_kit::component::input::Outdent,
            ),
        ])),
        MenuItem::submenu(Menu::new(t!("menu.comment").to_string()).items([
            MenuItem::action(t!("menu.comment_lines").to_string(), query::CommentLines),
            MenuItem::action(
                t!("menu.uncomment_lines").to_string(),
                query::UncommentLines,
            ),
        ])),
        MenuItem::action(t!("menu.toggle_comment").to_string(), query::ToggleComment),
        MenuItem::submenu(Menu::new(t!("menu.convert_case").to_string()).items([
            MenuItem::action(t!("menu.upper_case").to_string(), query::UpperCase),
            MenuItem::action(t!("menu.lower_case").to_string(), query::LowerCase),
        ])),
        MenuItem::separator(),
        MenuItem::action(t!("menu.beautify").to_string(), query::Beautify),
        MenuItem::action(
            t!("menu.beautify_options").to_string(),
            query::BeautifyOptionsDialog,
        ),
        MenuItem::action(t!("menu.minify").to_string(), query::Minify),
    ])
}

/// Query menu (31 §8). Shown only while a query editor is the active tab.
fn query_menu(continue_on_error: bool, keep_results: bool) -> Menu {
    Menu::new(t!("menu.query").to_string()).items([
        MenuItem::action(t!("menu.run").to_string(), query::Run),
        MenuItem::action(t!("menu.run_current").to_string(), query::RunCurrent),
        MenuItem::action(t!("menu.stop").to_string(), query::Stop),
        MenuItem::action(t!("menu.explain").to_string(), query::Explain),
        checked(
            t!("menu.continue_on_error").to_string(),
            Box::new(query::ToggleContinueOnError),
            continue_on_error,
        ),
        checked(
            t!("menu.keep_results").to_string(),
            Box::new(query::ToggleKeepResults),
            keep_results,
        ),
    ])
}

/// Table menu (21 §6). Shown only while a table viewer is the active tab.
fn table_menu(in_transaction: bool, filter_open: bool) -> Menu {
    let mut items = Vec::new();
    if in_transaction {
        items.push(MenuItem::action(
            t!("menu.commit").to_string(),
            navidog_grid::CommitTransaction,
        ));
        items.push(MenuItem::action(
            t!("menu.rollback").to_string(),
            navidog_grid::RollbackTransaction,
        ));
    } else {
        items.push(MenuItem::action(
            t!("menu.begin_transaction").to_string(),
            navidog_grid::BeginTransaction,
        ));
    }
    items.push(MenuItem::action(
        t!("menu.design_table").to_string(),
        navidog_core::actions::DesignTable { object: None },
    ));
    items.push(MenuItem::action(
        t!("menu.query_table").to_string(),
        navidog_grid::QueryTable,
    ));
    items.push(MenuItem::submenu(
        Menu::new(t!("menu.filter").to_string()).items([checked(
            t!("menu.show_filter").to_string(),
            Box::new(navidog_grid::ToggleFilter),
            filter_open,
        )]),
    ));
    items.push(MenuItem::submenu(
        Menu::new(t!("menu.sort").to_string()).items([
            MenuItem::action(t!("menu.sort_asc").to_string(), navidog_grid::SortAscending),
            MenuItem::action(
                t!("menu.sort_desc").to_string(),
                navidog_grid::SortDescending,
            ),
            MenuItem::action(t!("menu.sort_none").to_string(), navidog_grid::SortNone),
        ]),
    ));
    items.push(MenuItem::action(
        t!("menu.apply_filter").to_string(),
        navidog_grid::ApplyFilterSort,
    ));
    items.push(MenuItem::action(
        t!("menu.clear_filter").to_string(),
        navidog_grid::ClearFilterSort,
    ));
    items.push(MenuItem::separator());
    items.push(MenuItem::action(
        t!("menu.refresh").to_string(),
        navidog_grid::RefreshTable,
    ));
    items.push(MenuItem::action(
        t!("menu.stop").to_string(),
        navidog_grid::StopLoad,
    ));
    Menu::new(t!("menu.table").to_string()).items(items)
}

pub fn set_menus(cx: &mut App) {
    let state = cx
        .try_global::<crate::state::AppState>()
        .cloned()
        .unwrap_or_default();
    let settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
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
    let surface = navidog_ui::tabs::active_menu(cx);
    let mut menus = vec![
        Menu::new("Navidog").items([
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
                MenuItem::action(t!("menu.new_query").to_string(), NewQuery),
                MenuItem::separator(),
                MenuItem::action(t!("menu.group").to_string(), NewGroup),
            ])),
            MenuItem::submenu(Menu::new(t!("menu.open_external").to_string()).items([
                MenuItem::action(
                    t!("menu.open_external_query").to_string(),
                    OpenExternalQuery,
                ),
            ])),
            MenuItem::submenu(recent_menu(cx)),
            MenuItem::action(t!("menu.open_uri").to_string(), OpenUri),
            MenuItem::separator(),
            MenuItem::action(t!("menu.save").to_string(), query::SaveQuery),
            MenuItem::action(t!("menu.save_as").to_string(), query::SaveQueryAs),
            MenuItem::action(
                t!("menu.save_as_external").to_string(),
                query::SaveAsExternal,
            ),
            MenuItem::action(t!("menu.save_as_navicat").to_string(), query::SaveAsNavicat),
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
            MenuItem::action(
                t!("menu.paste_from_stack").to_string(),
                query::PasteFromStack,
            ),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new(t!("menu.find").to_string()).items([
                MenuItem::action(t!("menu.find_find").to_string(), navidog_grid::ShowFind),
                MenuItem::action(
                    t!("menu.find_replace").to_string(),
                    navidog_grid::ShowReplace,
                ),
                MenuItem::action(t!("menu.find_next").to_string(), navidog_grid::FindNext),
                MenuItem::action(t!("menu.find_previous").to_string(), query::FindPrevious),
            ])),
            MenuItem::submenu(Menu::new(t!("menu.code_completion").to_string()).items([
                MenuItem::action(t!("menu.update_outline").to_string(), query::UpdateOutline),
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
            MenuItem::action(t!("menu.word_wrap").to_string(), query::ToggleWordWrap),
            MenuItem::action(t!("menu.zoom_in").to_string(), query::ZoomIn),
            MenuItem::action(t!("menu.zoom_out").to_string(), query::ZoomOut),
            MenuItem::action(t!("menu.actual_size").to_string(), query::ActualSize),
            MenuItem::submenu(Menu::new(t!("menu.results").to_string()).items([
                checked(
                    t!("menu.results_below").to_string(),
                    Box::new(query::ResultsBelow),
                    settings.results_position == ResultsPosition::Below,
                ),
                checked(
                    t!("menu.results_beside").to_string(),
                    Box::new(query::ResultsBeside),
                    settings.results_position == ResultsPosition::Beside,
                ),
                checked(
                    t!("menu.results_new_page").to_string(),
                    Box::new(query::ResultsNewPage),
                    settings.results_position == ResultsPosition::NewPage,
                ),
            ])),
            MenuItem::submenu(Menu::new(t!("menu.show_after_run").to_string()).items([
                checked(
                    t!("menu.show_result_tab").to_string(),
                    Box::new(query::ShowResultAfterRun),
                    settings.show_after_run == ShowAfterRun::Result,
                ),
                checked(
                    t!("menu.show_messages_tab").to_string(),
                    Box::new(query::ShowMessagesAfterRun),
                    settings.show_after_run == ShowAfterRun::Messages,
                ),
                checked(
                    t!("menu.show_summary_tab").to_string(),
                    Box::new(query::ShowSummaryAfterRun),
                    settings.show_after_run == ShowAfterRun::Summary,
                ),
            ])),
            checked(
                t!("menu.show_profile").to_string(),
                Box::new(query::ToggleProfile),
                settings.show_profile,
            ),
            MenuItem::submenu(Menu::new(t!("menu.history_log").to_string()).items([
                checked(
                    t!("menu.history_date").to_string(),
                    Box::new(query::ToggleHistoryDate),
                    settings.history_header.date,
                ),
                checked(
                    t!("menu.history_time").to_string(),
                    Box::new(query::ToggleHistoryTime),
                    settings.history_header.time,
                ),
                checked(
                    t!("menu.history_server").to_string(),
                    Box::new(query::ToggleHistoryServer),
                    settings.history_header.server,
                ),
                checked(
                    t!("menu.history_session").to_string(),
                    Box::new(query::ToggleHistorySession),
                    settings.history_header.session,
                ),
                checked(
                    t!("menu.history_kind").to_string(),
                    Box::new(query::ToggleHistoryKind),
                    settings.history_header.kind,
                ),
                checked(
                    t!("menu.history_elapsed").to_string(),
                    Box::new(query::ToggleHistoryElapsed),
                    settings.history_header.elapsed,
                ),
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
            MenuItem::action(t!("menu.focus_mode").to_string(), ToggleFocusMode),
            MenuItem::action(t!("menu.full_screen").to_string(), ToggleFullScreen),
        ]),
        favorites_menu(cx),
        Menu::new(t!("menu.tools").to_string()).items([
            MenuItem::action(t!("menu.console").to_string(), OpenConsole),
            MenuItem::action(t!("menu.history_log").to_string(), OpenHistoryLog),
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
    ];
    // After View: query + format, or table, or neither (10-shell.md).
    match surface.surface {
        navidog_ui::tabs::MenuSurface::Query => {
            menus.insert(
                4,
                query_menu(settings.continue_on_error, settings.keep_results),
            );
            menus.insert(5, format_menu());
        }
        navidog_ui::tabs::MenuSurface::Table => {
            menus.insert(
                4,
                table_menu(surface.table_in_transaction, surface.filter_open),
            );
        }
        navidog_ui::tabs::MenuSurface::Other => {}
    }
    cx.set_menus(menus);
}
