//! Metadata for static built-in menu commands. Feature behavior stays in the feature crates;
//! menus, key bindings and window handlers still provide execution. The command palette and
//! settings UI will consume this inventory.

use crate::actions::{
    About, AddActiveToFavorites, BringAllToFront, CheckForUpdates, ClearFavorites, ClearRecent,
    CloseWindow, Hide, HideOthers, Minimize, OpenHelp, OpenUri, Preferences, Quit, ReportIssue,
    ResetLayout, ShowAll, ShowMainWindow, ToggleFilterOnTop, ToggleFullScreen, ToggleHideGroups,
    ToggleInfoPane, ToggleNavigator, ToggleStatusBar, ToggleToolbar, Zoom,
};
use gpui_kit::{Action, App, SharedString};
use navidog_app_state::actions::{
    ManageConnections, NewGroup, NewQuery, OpenConsole, OpenExternalQuery, OpenHistoryLog,
    OpenTool, ToggleFocusMode, ToggleHideObjectGroups, ToggleOnlyActive, ToggleShowHidden,
    ToggleShowTableProfile,
};
use navidog_app_state::registry::{
    CommandGroup, CommandInfo, CommandRegistry, CommandTarget, CommandTitle, Source,
};
use rust_i18n::t;

type MakeAction = fn() -> Box<dyn Action>;

struct Spec {
    id: &'static str,
    group: CommandGroup,
    title: &'static str,
    action: MakeAction,
    icon: Option<&'static str>,
    shortcut: Option<&'static str>,
    target: CommandTarget,
    keywords: &'static [&'static str],
}

impl Spec {
    fn new(id: &'static str, group: CommandGroup, title: &'static str, action: MakeAction) -> Self {
        Self {
            id,
            group,
            title,
            action,
            icon: None,
            shortcut: None,
            target: CommandTarget::None,
            keywords: &[],
        }
    }

    fn shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    fn target(mut self, target: CommandTarget) -> Self {
        self.target = target;
        self
    }

    fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }
}

fn translated_title(key: &str) -> SharedString {
    t!(key).to_string().into()
}

fn tool(tool: &'static str) -> MakeAction {
    match tool {
        "users" => || {
            Box::new(OpenTool {
                scope: None,
                tool: "users".into(),
                target: None,
            })
        },
        "privileges" => || {
            Box::new(OpenTool {
                scope: None,
                tool: "privileges".into(),
                target: None,
            })
        },
        "monitor" => || {
            Box::new(OpenTool {
                scope: None,
                tool: "monitor".into(),
                target: None,
            })
        },
        "find" => || {
            Box::new(OpenTool {
                scope: None,
                tool: "find".into(),
                target: None,
            })
        },
        _ => unreachable!("unknown built-in tool"),
    }
}

fn commands() -> Vec<Spec> {
    use CommandGroup as G;
    use CommandTarget as Target;

    vec![
        // Application
        Spec::new("app.about", G::App, "menu.about", || Box::new(About)),
        Spec::new("app.preferences", G::App, "menu.preferences", || {
            Box::new(Preferences)
        })
        .shortcut("cmd-,"),
        Spec::new("app.check_updates", G::App, "menu.check_updates", || {
            Box::new(CheckForUpdates)
        }),
        Spec::new("app.hide", G::App, "menu.hide", || Box::new(Hide)).shortcut("cmd-h"),
        Spec::new("app.hide_others", G::App, "menu.hide_others", || {
            Box::new(HideOthers)
        })
        .shortcut("alt-cmd-h"),
        Spec::new("app.show_all", G::App, "menu.show_all", || {
            Box::new(ShowAll)
        }),
        Spec::new("app.quit", G::App, "menu.quit", || Box::new(Quit)).shortcut("cmd-q"),
        // File and tabs
        Spec::new("query.new", G::Query, "toolbar.new_query", || {
            Box::new(NewQuery)
        })
        .icon("icons/square-terminal.svg")
        .shortcut("cmd-y")
        .target(Target::Workspace)
        .keywords(&["sql", "editor", "tab"]),
        Spec::new("file.save_query", G::File, "menu.save", || {
            Box::new(navidog_query::SaveQuery)
        })
        .shortcut("cmd-s")
        .target(Target::Panel),
        Spec::new("file.save_query_as", G::File, "menu.save_as", || {
            Box::new(navidog_query::SaveQueryAs)
        })
        .shortcut("shift-cmd-s")
        .target(Target::Panel),
        Spec::new(
            "file.save_query_as_external",
            G::File,
            "menu.save_as_external",
            || Box::new(navidog_query::SaveAsExternal),
        )
        .target(Target::Panel),
        Spec::new(
            "file.save_query_as_navicat",
            G::File,
            "menu.save_as_navicat",
            || Box::new(navidog_query::SaveAsNavicat),
        )
        .target(Target::Panel),
        Spec::new("file.new_group", G::File, "menu.group", || {
            Box::new(NewGroup)
        })
        .target(Target::Workspace),
        Spec::new(
            "file.open_external_query",
            G::File,
            "menu.open_external_query",
            || Box::new(OpenExternalQuery),
        ),
        Spec::new("file.open_uri", G::File, "menu.open_uri", || {
            Box::new(OpenUri)
        })
        .shortcut("cmd-u"),
        Spec::new(
            "file.manage_connections",
            G::File,
            "menu.manage_connections",
            || Box::new(ManageConnections),
        ),
        Spec::new("file.close_tab", G::Tabs, "menu.close_tab", || {
            Box::new(navidog_ui::tabs::CloseTab)
        })
        .shortcut("cmd-w")
        .target(Target::Tab),
        Spec::new("file.close_window", G::File, "menu.close_window", || {
            Box::new(CloseWindow)
        })
        .shortcut("shift-cmd-w"),
        Spec::new("tab.next", G::Tabs, "menu.next_tab", || {
            Box::new(navidog_ui::tabs::NextTab)
        })
        .shortcut("cmd-}")
        .target(Target::Tab),
        Spec::new("tab.previous", G::Tabs, "menu.previous_tab", || {
            Box::new(navidog_ui::tabs::PrevTab)
        })
        .shortcut("cmd-{")
        .target(Target::Tab),
        // Favorites and recent objects
        Spec::new("favorites.add_active", G::File, "menu.add_favorite", || {
            Box::new(AddActiveToFavorites)
        })
        .target(Target::Tab),
        Spec::new("favorites.clear", G::File, "menu.clear_favorites", || {
            Box::new(ClearFavorites)
        }),
        Spec::new(
            "favorites.clear_recent",
            G::File,
            "menu.clear_recent",
            || Box::new(ClearRecent),
        ),
        // Edit and find
        Spec::new(
            "edit.paste_from_stack",
            G::Edit,
            "menu.paste_from_stack",
            || Box::new(navidog_query::PasteFromStack),
        )
        .target(Target::Panel),
        Spec::new("edit.find", G::Edit, "menu.find_find", || {
            Box::new(navidog_grid::ShowFind)
        })
        .shortcut("cmd-f")
        .target(Target::Panel),
        Spec::new("edit.replace", G::Edit, "menu.find_replace", || {
            Box::new(navidog_grid::ShowReplace)
        })
        .shortcut("alt-cmd-f")
        .target(Target::Panel),
        Spec::new("edit.find_next", G::Edit, "menu.find_next", || {
            Box::new(navidog_grid::FindNext)
        })
        .shortcut("cmd-g")
        .target(Target::Panel),
        Spec::new("edit.find_previous", G::Edit, "menu.find_previous", || {
            Box::new(navidog_query::FindPrevious)
        })
        .shortcut("shift-cmd-g")
        .target(Target::Panel),
        Spec::new(
            "edit.update_outline",
            G::Edit,
            "menu.update_outline",
            || Box::new(navidog_query::UpdateOutline),
        )
        .target(Target::Panel),
        // View
        Spec::new(
            "view.navigation_pane",
            G::View,
            "menu.show_navigation_pane",
            || Box::new(ToggleNavigator),
        )
        .shortcut("cmd-1"),
        Spec::new("view.info_pane", G::View, "menu.show_info_pane", || {
            Box::new(ToggleInfoPane)
        })
        .shortcut("cmd-2"),
        Spec::new("view.hide_groups", G::View, "menu.hide_groups", || {
            Box::new(ToggleHideGroups)
        }),
        Spec::new("view.filter_on_top", G::View, "menu.filter_on_top", || {
            Box::new(ToggleFilterOnTop)
        }),
        Spec::new("view.only_active", G::View, "menu.only_active", || {
            Box::new(ToggleOnlyActive)
        }),
        Spec::new("view.show_hidden", G::View, "menu.show_hidden", || {
            Box::new(ToggleShowHidden)
        }),
        Spec::new(
            "view.hide_object_groups",
            G::View,
            "menu.hide_object_groups",
            || Box::new(ToggleHideObjectGroups),
        ),
        Spec::new(
            "view.show_table_profile",
            G::View,
            "menu.show_table_profile",
            || Box::new(ToggleShowTableProfile),
        ),
        Spec::new("view.raw_data", G::View, "menu.raw_data", || {
            Box::new(navidog_grid::ToggleRawData)
        })
        .target(Target::Panel),
        Spec::new("view.word_wrap", G::View, "menu.word_wrap", || {
            Box::new(navidog_query::ToggleWordWrap)
        })
        .target(Target::Panel),
        Spec::new("view.zoom_in", G::View, "menu.zoom_in", || {
            Box::new(navidog_query::ZoomIn)
        })
        .shortcut("cmd-=")
        .target(Target::Panel),
        Spec::new("view.zoom_out", G::View, "menu.zoom_out", || {
            Box::new(navidog_query::ZoomOut)
        })
        .shortcut("cmd--")
        .target(Target::Panel),
        Spec::new("view.actual_size", G::View, "menu.actual_size", || {
            Box::new(navidog_query::ActualSize)
        })
        .shortcut("cmd-0")
        .target(Target::Panel),
        Spec::new("view.results_below", G::View, "menu.results_below", || {
            Box::new(navidog_query::ResultsBelow)
        })
        .target(Target::Panel),
        Spec::new(
            "view.results_beside",
            G::View,
            "menu.results_beside",
            || Box::new(navidog_query::ResultsBeside),
        )
        .target(Target::Panel),
        Spec::new(
            "view.results_new_page",
            G::View,
            "menu.results_new_page",
            || Box::new(navidog_query::ResultsNewPage),
        )
        .target(Target::Panel),
        Spec::new(
            "view.show_result_after_run",
            G::View,
            "menu.show_result_tab",
            || Box::new(navidog_query::ShowResultAfterRun),
        )
        .target(Target::Panel),
        Spec::new(
            "view.show_messages_after_run",
            G::View,
            "menu.show_messages_tab",
            || Box::new(navidog_query::ShowMessagesAfterRun),
        )
        .target(Target::Panel),
        Spec::new(
            "view.show_summary_after_run",
            G::View,
            "menu.show_summary_tab",
            || Box::new(navidog_query::ShowSummaryAfterRun),
        )
        .target(Target::Panel),
        Spec::new("view.query_profile", G::View, "menu.show_profile", || {
            Box::new(navidog_query::ToggleProfile)
        })
        .target(Target::Panel),
        Spec::new("view.history_date", G::View, "menu.history_date", || {
            Box::new(navidog_query::ToggleHistoryDate)
        })
        .target(Target::Panel),
        Spec::new("view.history_time", G::View, "menu.history_time", || {
            Box::new(navidog_query::ToggleHistoryTime)
        })
        .target(Target::Panel),
        Spec::new(
            "view.history_server",
            G::View,
            "menu.history_server",
            || Box::new(navidog_query::ToggleHistoryServer),
        )
        .target(Target::Panel),
        Spec::new(
            "view.history_session",
            G::View,
            "menu.history_session",
            || Box::new(navidog_query::ToggleHistorySession),
        )
        .target(Target::Panel),
        Spec::new("view.history_kind", G::View, "menu.history_kind", || {
            Box::new(navidog_query::ToggleHistoryKind)
        })
        .target(Target::Panel),
        Spec::new(
            "view.history_elapsed",
            G::View,
            "menu.history_elapsed",
            || Box::new(navidog_query::ToggleHistoryElapsed),
        )
        .target(Target::Panel),
        Spec::new("view.status_bar", G::View, "menu.show_status_bar", || {
            Box::new(ToggleStatusBar)
        }),
        Spec::new("view.toolbar", G::View, "menu.show_toolbar", || {
            Box::new(ToggleToolbar)
        })
        .shortcut("alt-cmd-t"),
        Spec::new("view.focus_mode", G::View, "menu.focus_mode", || {
            Box::new(ToggleFocusMode)
        }),
        Spec::new("view.full_screen", G::View, "menu.full_screen", || {
            Box::new(ToggleFullScreen)
        })
        .shortcut("ctrl-cmd-f"),
        // Query
        Spec::new("query.run", G::Query, "menu.run", || {
            Box::new(navidog_query::Run)
        })
        .shortcut("cmd-r")
        .target(Target::Panel)
        .keywords(&["execute", "sql"]),
        Spec::new("query.run_current", G::Query, "menu.run_current", || {
            Box::new(navidog_query::RunCurrent)
        })
        .shortcut("shift-cmd-r")
        .target(Target::Panel)
        .keywords(&["execute", "statement", "sql"]),
        Spec::new("query.stop", G::Query, "menu.stop", || {
            Box::new(navidog_query::Stop)
        })
        .shortcut("cmd-.")
        .target(Target::Panel)
        .keywords(&["cancel", "sql"]),
        Spec::new("query.explain", G::Query, "menu.explain", || {
            Box::new(navidog_query::Explain)
        })
        .target(Target::Panel)
        .keywords(&["plan", "sql"]),
        Spec::new(
            "query.continue_on_error",
            G::Query,
            "menu.continue_on_error",
            || Box::new(navidog_query::ToggleContinueOnError),
        )
        .target(Target::Panel),
        Spec::new("query.keep_results", G::Query, "menu.keep_results", || {
            Box::new(navidog_query::ToggleKeepResults)
        })
        .target(Target::Panel),
        // Format
        Spec::new(
            "format.comment_lines",
            G::Edit,
            "menu.comment_lines",
            || Box::new(navidog_query::CommentLines),
        )
        .target(Target::Panel),
        Spec::new(
            "format.uncomment_lines",
            G::Edit,
            "menu.uncomment_lines",
            || Box::new(navidog_query::UncommentLines),
        )
        .target(Target::Panel),
        Spec::new(
            "format.toggle_comment",
            G::Edit,
            "menu.toggle_comment",
            || Box::new(navidog_query::ToggleComment),
        )
        .shortcut("cmd-/")
        .target(Target::Panel),
        Spec::new("format.upper_case", G::Edit, "menu.upper_case", || {
            Box::new(navidog_query::UpperCase)
        })
        .target(Target::Panel),
        Spec::new("format.lower_case", G::Edit, "menu.lower_case", || {
            Box::new(navidog_query::LowerCase)
        })
        .target(Target::Panel),
        Spec::new("format.beautify", G::Edit, "menu.beautify", || {
            Box::new(navidog_query::Beautify)
        })
        .target(Target::Panel)
        .keywords(&["format", "sql"]),
        Spec::new(
            "format.beautify_options",
            G::Edit,
            "menu.beautify_options",
            || Box::new(navidog_query::BeautifyOptionsDialog),
        )
        .target(Target::Panel),
        Spec::new("format.minify", G::Edit, "menu.minify", || {
            Box::new(navidog_query::Minify)
        })
        .target(Target::Panel),
        // Table
        Spec::new(
            "table.begin_transaction",
            G::Table,
            "menu.begin_transaction",
            || Box::new(navidog_grid::BeginTransaction),
        )
        .target(Target::Panel),
        Spec::new("table.commit_transaction", G::Table, "menu.commit", || {
            Box::new(navidog_grid::CommitTransaction)
        })
        .target(Target::Panel),
        Spec::new(
            "table.rollback_transaction",
            G::Table,
            "menu.rollback",
            || Box::new(navidog_grid::RollbackTransaction),
        )
        .target(Target::Panel),
        Spec::new("table.design", G::Table, "menu.design_table", || {
            Box::new(navidog_app_state::actions::DesignTable { object: None })
        })
        .target(Target::Panel),
        Spec::new("table.query", G::Table, "menu.query_table", || {
            Box::new(navidog_grid::QueryTable)
        })
        .shortcut("shift-cmd-y")
        .target(Target::Panel),
        Spec::new("table.toggle_filter", G::Table, "menu.show_filter", || {
            Box::new(navidog_grid::ToggleFilter)
        })
        .shortcut("cmd-shift-f")
        .target(Target::Panel),
        Spec::new("table.sort_ascending", G::Table, "menu.sort_asc", || {
            Box::new(navidog_grid::SortAscending)
        })
        .target(Target::Panel),
        Spec::new("table.sort_descending", G::Table, "menu.sort_desc", || {
            Box::new(navidog_grid::SortDescending)
        })
        .target(Target::Panel),
        Spec::new("table.sort_none", G::Table, "menu.sort_none", || {
            Box::new(navidog_grid::SortNone)
        })
        .target(Target::Panel),
        Spec::new("table.apply_filter", G::Table, "menu.apply_filter", || {
            Box::new(navidog_grid::ApplyFilterSort)
        })
        .shortcut("ctrl-r")
        .target(Target::Panel),
        Spec::new("table.clear_filter", G::Table, "menu.clear_filter", || {
            Box::new(navidog_grid::ClearFilterSort)
        })
        .target(Target::Panel),
        Spec::new("table.refresh", G::Table, "menu.refresh", || {
            Box::new(navidog_grid::RefreshTable)
        })
        .shortcut("cmd-r")
        .target(Target::Panel),
        Spec::new("table.stop_loading", G::Table, "menu.stop", || {
            Box::new(navidog_grid::StopLoad)
        })
        .shortcut("cmd-.")
        .target(Target::Panel),
        // Tools
        Spec::new("tools.console", G::Tools, "menu.console", || {
            Box::new(OpenConsole)
        })
        .shortcut("shift-cmd-c")
        .target(Target::Workspace),
        Spec::new("tools.history_log", G::Tools, "menu.history_log", || {
            Box::new(OpenHistoryLog)
        })
        .shortcut("shift-cmd-h")
        .target(Target::Workspace),
        Spec::new("tools.users", G::Tools, "menu.users", tool("users")).target(Target::Workspace),
        Spec::new(
            "tools.privileges",
            G::Tools,
            "menu.privileges",
            tool("privileges"),
        )
        .target(Target::Workspace),
        Spec::new(
            "tools.server_monitor",
            G::Tools,
            "menu.server_monitor",
            tool("monitor"),
        )
        .target(Target::Workspace),
        Spec::new(
            "tools.find_in_database",
            G::Tools,
            "menu.find_in_database",
            tool("find"),
        )
        .target(Target::Workspace),
        // Window
        Spec::new("window.minimize", G::Window, "menu.minimize", || {
            Box::new(Minimize)
        })
        .shortcut("cmd-m"),
        Spec::new("window.zoom", G::Window, "menu.zoom", || Box::new(Zoom)),
        Spec::new(
            "window.reset_layout",
            G::Window,
            "menu.reset_layout",
            || Box::new(ResetLayout),
        ),
        Spec::new(
            "window.show_main",
            G::Window,
            "menu.show_main_window",
            || Box::new(ShowMainWindow),
        ),
        Spec::new(
            "window.bring_all_to_front",
            G::Window,
            "menu.bring_all_to_front",
            || Box::new(BringAllToFront),
        ),
        // Help
        Spec::new("help.open", G::Help, "menu.navidog_help", || {
            Box::new(OpenHelp)
        }),
        Spec::new("help.report_issue", G::Help, "menu.report_issue", || {
            Box::new(ReportIssue)
        }),
    ]
}

pub(crate) fn init(cx: &mut App) {
    let commands = commands();
    let registry = &mut cx.global_mut::<CommandRegistry>().0;
    for spec in commands {
        let info = CommandInfo::new(
            spec.group,
            CommandTitle::localized(spec.title, translated_title),
            spec.action,
        )
        .target(spec.target)
        .keywords(spec.keywords);
        let info = match spec.shortcut {
            Some(shortcut) => info.shortcut(shortcut),
            None => info,
        };
        let info = match spec.icon {
            Some(icon) => info.icon(icon),
            None => info,
        };
        registry
            .register(spec.id, Source::Builtin, info)
            .unwrap_or_else(|error| panic!("built-in command catalogue: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_and_shortcuts_are_well_formed() {
        let commands = commands();
        assert_eq!(
            commands
                .iter()
                .map(|command| command.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            commands.len(),
            "command ids must be unique"
        );
        for command in commands {
            assert!(!command.id.is_empty());
            assert!(
                command.id.contains('.'),
                "{} must use a namespace",
                command.id
            );
            assert!(
                !command.title.is_empty(),
                "{} must have a title",
                command.id
            );
            if let Some(shortcut) = command.shortcut {
                gpui_kit::Keystroke::parse(shortcut).unwrap_or_else(|error| {
                    panic!("{} has an invalid shortcut: {error}", command.id)
                });
            }
            assert!(
                !(command.action)().name().is_empty(),
                "{} must build an action",
                command.id
            );
        }
    }
}
