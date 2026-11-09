//! The command catalogue is metadata-only in this stage, but it must still initialize as one
//! complete inventory: unique ids, localized labels, parseable shortcuts, and working actions.

use std::collections::HashSet;

use gpui_kit::{Action, Keystroke, TestAppContext};

#[derive(Debug)]
struct Command {
    id: String,
    group: String,
    label: String,
    title_key: Option<String>,
    shortcut: Option<String>,
    action: String,
}

#[gpui_kit::test]
fn built_in_commands_form_a_complete_localized_catalogue(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("navidog-command-catalog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this test binary has a single test; nothing else reads the environment concurrently.
    unsafe { std::env::set_var("NAVIDOG_HOME", &home) };

    cx.update(|cx| {
        navidog::init(cx);
        rust_i18n::set_locale("en");
    });

    let commands = read_commands(cx);
    assert_eq!(
        commands.len(),
        113,
        "update this count deliberately when adding or removing a built-in command"
    );
    let ids: HashSet<_> = commands.iter().map(|command| command.id.as_str()).collect();
    assert_eq!(ids.len(), commands.len(), "command ids must be unique");

    for command in &commands {
        assert!(!command.id.is_empty());
        assert!(!command.group.is_empty(), "{} has no group", command.id);
        assert!(!command.action.is_empty(), "{} has no action", command.id);
        assert!(!command.label.is_empty(), "{} has no label", command.id);
        assert_ne!(
            Some(command.label.as_str()),
            command.title_key.as_deref(),
            "{} resolves to its raw i18n key",
            command.id
        );
        if let Some(shortcut) = &command.shortcut {
            Keystroke::parse(shortcut)
                .unwrap_or_else(|error| panic!("{} has an invalid shortcut: {error}", command.id));
        }
    }

    for id in [
        "app.about",
        "query.new",
        "query.run",
        "table.refresh",
        "view.navigation_pane",
        "tools.console",
        "file.close_tab",
        "connection.new",
        "connection.close",
        "objects.tables",
        "objects.users",
    ] {
        assert!(ids.contains(id), "missing command {id}");
    }

    let english = commands
        .iter()
        .find(|command| command.id == "query.new")
        .unwrap()
        .label
        .clone();
    assert_eq!(english, "New Query");
    assert_eq!(
        commands
            .iter()
            .find(|command| command.id == "query.new")
            .unwrap()
            .action,
        navidog_app_state::actions::NewQuery::name_for_type()
    );

    cx.update(|_| rust_i18n::set_locale("zh-CN"));
    let chinese = read_commands(cx)
        .into_iter()
        .find(|command| command.id == "query.new")
        .unwrap()
        .label;
    assert_eq!(chinese, "新建查询");

    cx.update(|_| rust_i18n::set_locale("en"));
    let _ = std::fs::remove_dir_all(&home);
}

fn read_commands(cx: &mut TestAppContext) -> Vec<Command> {
    cx.update(|cx| {
        let registry = cx.global::<navidog_app_state::registry::CommandRegistry>();
        registry
            .iter()
            .map(|(id, _, info)| Command {
                id: id.to_string(),
                group: info.group.title().to_string(),
                label: info.label().to_string(),
                title_key: info.title.key().map(str::to_string),
                shortcut: info.shortcut.as_ref().map(ToString::to_string),
                action: (info.action)().name().to_string(),
            })
            .collect()
    })
}
