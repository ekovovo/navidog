//! Object browsing: navigator, objects tab, info pane, read-only DDL tab and the metadata cache (20-objects.md).

// The Action derive macro emits `gpui::` paths
extern crate gpui_kit as gpui;

rust_i18n::i18n!("locales", fallback = "en");

pub mod catalog;
mod ddl;
mod errors;
mod identifiers;
mod info;
mod navigator;
mod objects;
mod ops;
mod selection;
mod snippets;

#[cfg(test)]
mod info_tests;

#[cfg(test)]
mod test_env {
    use std::sync::{Mutex, MutexGuard};

    /// Serializes tests that write `NAVIDOG_HOME`. The variable is process-global.
    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

use gpui_kit::{Action, App, SharedString};
use navidog_app_state::actions::OpenTool;
use navidog_app_state::registry::{CommandInfo, CommandRegistry, Source};
use navidog_app_state::store::ConnectionStore;
use navidog_app_state::workspace::{Selection, Workspace};
use navidog_model::object::ObjectKind;
use rust_i18n::t;

pub use ddl::DdlPanel;
pub use info::InfoPanel;
pub use navigator::{NavigatorPanel, ViewOptions, bind_actions, set_view_options};
pub use objects::ObjectsPanel;
pub use snippets::{snippet_form, snippet_form_entity};

/// Category buttons of the main toolbar: show that category's objects in the current database or schema.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = objects, no_json)]
pub struct ShowCategory {
    pub kind: SharedString,
}

pub fn init(cx: &mut App) {
    catalog::init(cx);
    navigator::init(cx);
    objects::init(cx);
    info::init(cx);
    ddl::init(cx);
    snippets::init(cx);
    cx.on_action(|a: &ShowCategory, cx| {
        let Some(kind) = ObjectKind::from_key(&a.kind) else {
            return;
        };
        let Some(scope) = category_scope(cx) else {
            return;
        };
        Workspace::global(cx).update(cx, |w, cx| w.select(Selection::Category(scope, kind), cx));
    });
    register_commands(cx);
}

/// Scope the category buttons act on: the database for MySQL / SQLite; PostgreSQL needs a schema.
fn category_scope(cx: &App) -> Option<navidog_model::object::Scope> {
    let scope = Workspace::global(cx).read(cx).selection().scope()?;
    let dialect = navidog_session::sessions::Sessions::global(cx)
        .read(cx)
        .dialect(&scope.connection)?;
    (!dialect.features().has_schemas || scope.schema.is_some()).then_some(scope)
}

/// A category button is enabled when there is a scope and its dialect supports the category.
/// Saved queries exist in every scope.
fn category_enabled(cx: &App, kind: ObjectKind) -> bool {
    if kind == ObjectKind::Query {
        return category_scope(cx).is_some();
    }
    let Some(scope) = category_scope(cx) else {
        return false;
    };
    let sessions = navidog_session::sessions::Sessions::global(cx).read(cx);
    let Some(dialect) = sessions.dialect(&scope.connection) else {
        return false;
    };
    let driver = navidog_app_state::store::ConnectionStore::global(cx)
        .read(cx)
        .get(&scope.connection)
        .map(|c| c.driver.clone())
        .unwrap_or_default();
    let version = match sessions.state(&scope.connection) {
        navidog_session::sessions::ConnState::Open { version, .. } => version.clone(),
        _ => String::new(),
    };
    dialect.lists_kind(kind, &driver, &version)
}

fn register_commands(cx: &mut App) {
    let registry = &mut cx.global_mut::<CommandRegistry>().0;
    type Entry = (
        &'static str,
        &'static str,
        &'static str,
        fn() -> Box<dyn Action>,
        fn(&App) -> bool,
    );
    let commands: [Entry; 5] = [
        (
            "objects.tables",
            "kind.table",
            "icons/table.svg",
            || {
                Box::new(ShowCategory {
                    kind: "table".into(),
                })
            },
            |cx| category_enabled(cx, ObjectKind::Table),
        ),
        (
            "objects.views",
            "kind.view",
            "icons/eye.svg",
            || {
                Box::new(ShowCategory {
                    kind: "view".into(),
                })
            },
            |cx| category_enabled(cx, ObjectKind::View),
        ),
        (
            "objects.functions",
            "kind.function",
            "icons/square-function.svg",
            || {
                Box::new(ShowCategory {
                    kind: "function".into(),
                })
            },
            |cx| category_enabled(cx, ObjectKind::Function),
        ),
        (
            "objects.queries",
            "kind.query",
            "icons/file-code.svg",
            || {
                Box::new(ShowCategory {
                    kind: "query".into(),
                })
            },
            |cx| category_enabled(cx, ObjectKind::Query),
        ),
        (
            "objects.users",
            "toolbar.user",
            "icons/user.svg",
            || {
                Box::new(OpenTool {
                    scope: None,
                    tool: "users".into(),
                    target: None,
                })
            },
            |cx| category_scope(cx).is_some(),
        ),
    ];
    for (id, title, icon, action, enabled) in commands {
        let _ = registry.register(
            id,
            Source::Builtin,
            CommandInfo {
                title: title.into(),
                icon: Some(icon.into()),
                shortcut: None,
                action,
                enabled: Some(enabled),
            },
        );
    }
}

/// Status bar text (10-shell.md §8): item count of the objects tab, or the selected connection's type and address.
pub fn status_text(cx: &App) -> SharedString {
    let selection = Workspace::global(cx).read(cx).selection().clone();
    // The cached count avoids rebuilding the whole object list for the status bar.
    let count = ObjectsPanel::cached_rows(cx).or_else(|| match ObjectsPanel::content(cx) {
        objects::Content::Rows(rows, _) => Some(rows.len()),
        _ => None,
    });
    if let Some(count) = count {
        let selected = matches!(selection, Selection::Object(_));
        return if selected {
            t!("obj.items_selected", count = count).to_string().into()
        } else {
            t!("obj.items", count = count).to_string().into()
        };
    }
    let store = ConnectionStore::global(cx);
    match selection
        .connection_id()
        .and_then(|id| store.read(cx).get(id))
    {
        Some(c) if c.supports_network() => format!(
            "{}  {}:{}",
            navidog_session::DriverRegistry::name(cx, &c.driver),
            c.general.host,
            c.general.port.unwrap_or_default()
        )
        .into(),
        Some(c) => format!(
            "{}  {}",
            navidog_session::DriverRegistry::name(cx, &c.driver),
            c.general.file.clone().unwrap_or_default()
        )
        .into(),
        None => SharedString::default(),
    }
}
