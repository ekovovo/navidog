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
use navidog_core::object::ObjectKind;
use navidog_core::registry::{CommandInfo, CommandRegistry, Source};
use navidog_core::store::ConnectionStore;
use navidog_core::workspace::{Selection, Workspace};
use rust_i18n::t;

pub use ddl::DdlPanel;
pub use info::InfoPanel;
pub use navigator::{NavigatorPanel, ViewOptions, bind_actions, set_view_options};
pub use objects::ObjectsPanel;

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
fn category_scope(cx: &App) -> Option<navidog_core::object::Scope> {
    let scope = Workspace::global(cx).read(cx).selection().scope()?;
    let dialect = navidog_db::sessions::Sessions::global(cx)
        .read(cx)
        .dialect(&scope.connection)?;
    (!dialect.has_schemas() || scope.schema.is_some()).then_some(scope)
}

/// A category button is enabled when there is a scope and its dialect supports the category.
/// Saved queries exist in every scope.
fn category_enabled(cx: &App, kind: ObjectKind) -> bool {
    if kind == ObjectKind::Query {
        return category_scope(cx).is_some();
    }
    category_scope(cx)
        .and_then(|s| {
            navidog_db::sessions::Sessions::global(cx)
                .read(cx)
                .dialect(&s.connection)
        })
        .is_some_and(|d| d.object_kinds().contains(&kind))
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
    let commands: [Entry; 4] = [
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

pub(crate) fn driver_name(driver: &str, cx: &App) -> SharedString {
    navidog_db::DriverRegistry::get(cx, driver)
        .map(|d| d.info().name.clone())
        .unwrap_or_else(|| driver.to_string().into())
}

/// Status bar text (10-shell.md §8): item count of the objects tab, or the selected connection's type and address.
pub fn status_text(cx: &App) -> SharedString {
    let selection = Workspace::global(cx).read(cx).selection().clone();
    if let objects::Content::Rows(rows, _) = ObjectsPanel::content(cx) {
        let count = rows.len();
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
            driver_name(&c.driver, cx),
            c.general.host,
            c.general.port.unwrap_or_default()
        )
        .into(),
        Some(c) => format!(
            "{}  {}",
            driver_name(&c.driver, cx),
            c.general.file.clone().unwrap_or_default()
        )
        .into(),
        None => SharedString::default(),
    }
}
