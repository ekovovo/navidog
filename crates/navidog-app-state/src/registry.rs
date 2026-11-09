//! Extension-point registries (03-plugin-api.md section 2). Built-in features and plugins
//! share one registry; every entry carries a `Source` and plugins are removed as a group.

use gpui_kit::{Action, App, Global, SharedString};
use rust_i18n::t;

/// Stable dot-separated identifier shared by menus, shortcuts, plugins and the command palette.
pub type CommandId = SharedString;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Builtin,
    Plugin(SharedString),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("`{0}` is already registered")]
    Duplicate(CommandId),
}

/// A registry indexed by key that keeps insertion order.
pub struct Registry<V> {
    entries: Vec<(CommandId, Source, V)>,
}

impl<V> Default for Registry<V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<V> Registry<V> {
    pub fn register(
        &mut self,
        key: impl Into<CommandId>,
        source: Source,
        value: V,
    ) -> Result<(), RegistryError> {
        let key = key.into();
        if self.get(&key).is_some() {
            return Err(RegistryError::Duplicate(key));
        }
        self.entries.push((key, source, value));
        Ok(())
    }

    pub fn unregister_source(&mut self, source: &Source) {
        self.entries.retain(|(_, s, _)| s != source);
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&CommandId, &Source, &V)> {
        self.entries.iter().map(|(k, s, v)| (k, s, v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Stable groups used by the command palette and future command settings. Plugins can use
/// [`CommandGroup::Other`] for a group this build does not know.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommandGroup {
    App,
    File,
    Edit,
    View,
    Connection,
    Navigator,
    Query,
    Table,
    Tools,
    Window,
    Help,
    Tabs,
    Other(SharedString),
}

impl CommandGroup {
    pub fn title(&self) -> SharedString {
        match self {
            Self::App => t!("command.group.app").to_string().into(),
            Self::File => t!("command.group.file").to_string().into(),
            Self::Edit => t!("command.group.edit").to_string().into(),
            Self::View => t!("command.group.view").to_string().into(),
            Self::Connection => t!("command.group.connection").to_string().into(),
            Self::Navigator => t!("command.group.navigator").to_string().into(),
            Self::Query => t!("command.group.query").to_string().into(),
            Self::Table => t!("command.group.table").to_string().into(),
            Self::Tools => t!("command.group.tools").to_string().into(),
            Self::Window => t!("command.group.window").to_string().into(),
            Self::Help => t!("command.group.help").to_string().into(),
            Self::Tabs => t!("command.group.tabs").to_string().into(),
            Self::Other(title) => title.clone(),
        }
    }
}

/// What a command needs beyond its Action. `None` is a global command; `Workspace` uses the
/// current navigator selection; the rest name the kind of object a target-aware caller must
/// resolve (for example the Quick Open target).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CommandTarget {
    #[default]
    None,
    Workspace,
    Connection,
    Database,
    Object,
    Panel,
    Tab,
}

impl CommandTarget {
    pub fn is_contextual(self) -> bool {
        !matches!(self, Self::None | Self::Workspace)
    }
}

/// A display title that stays current after a language switch. Built-ins keep their i18n key and
/// a translator from the registering crate; plugin commands can provide already-localized text.
#[derive(Clone, Debug)]
pub enum CommandTitle {
    Localized {
        key: &'static str,
        translator: fn(&str) -> SharedString,
    },
    Static(SharedString),
}

impl CommandTitle {
    pub fn localized(key: &'static str, translator: fn(&str) -> SharedString) -> Self {
        Self::Localized { key, translator }
    }

    pub fn static_text(title: impl Into<SharedString>) -> Self {
        Self::Static(title.into())
    }

    pub fn key(&self) -> Option<&'static str> {
        match self {
            Self::Localized { key, .. } => Some(key),
            Self::Static(_) => None,
        }
    }

    pub fn resolve(&self) -> SharedString {
        match self {
            Self::Localized { key, translator } => translator(key),
            Self::Static(title) => title.clone(),
        }
    }
}

/// A command: one Action shared by menus, toolbars, context menus and shortcuts. This is catalog
/// metadata only; invoking it still dispatches the typed GPUI Action.
#[derive(Debug)]
pub struct CommandInfo {
    pub group: CommandGroup,
    pub title: CommandTitle,
    /// Icon asset path, for example `icons/plug.svg`
    pub icon: Option<SharedString>,
    /// Shortcut text used by a UI that cannot inspect the key-binding table yet. The binding
    /// table remains authoritative until the shortcut catalogue is unified.
    pub shortcut: Option<SharedString>,
    /// Extra search terms, in addition to the command id and localized title.
    pub keywords: &'static [&'static str],
    pub target: CommandTarget,
    /// Whether a palette without a target picker should list this command.
    pub show_in_palette: bool,
    pub action: fn() -> Box<dyn Action>,
    /// Whether it is currently enabled (for example a database must be selected); `None` means always
    pub enabled: Option<fn(&App) -> bool>,
}

impl CommandInfo {
    pub fn new(group: CommandGroup, title: CommandTitle, action: fn() -> Box<dyn Action>) -> Self {
        Self {
            group,
            title,
            icon: None,
            shortcut: None,
            keywords: &[],
            target: CommandTarget::None,
            show_in_palette: true,
            action,
            enabled: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    pub fn target(mut self, target: CommandTarget) -> Self {
        self.target = target;
        self
    }

    pub fn enabled(mut self, enabled: fn(&App) -> bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    pub fn hidden_from_palette(mut self) -> Self {
        self.show_in_palette = false;
        self
    }

    pub fn label(&self) -> SharedString {
        self.title.resolve()
    }

    pub fn is_enabled(&self, cx: &App) -> bool {
        self.enabled.is_none_or(|f| f(cx))
    }
}

macro_rules! global_registry {
    ($name:ident, $value:ty) => {
        #[derive(Default)]
        pub struct $name(pub Registry<$value>);
        impl Global for $name {}
        impl std::ops::Deref for $name {
            type Target = Registry<$value>;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl std::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

global_registry!(CommandRegistry, CommandInfo);

pub(crate) fn init(cx: &mut App) {
    cx.set_global(CommandRegistry::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicates_and_removes_by_source() {
        let mut r: Registry<u8> = Registry::default();
        r.register("a", Source::Builtin, 1).unwrap();
        r.register("p/b", Source::Plugin("p".into()), 2).unwrap();
        assert_eq!(
            r.register("a", Source::Builtin, 3),
            Err(RegistryError::Duplicate("a".into()))
        );
        r.unregister_source(&Source::Plugin("p".into()));
        assert_eq!(
            r.iter().map(|(k, _, _)| k.to_string()).collect::<Vec<_>>(),
            ["a"]
        );
    }

    #[test]
    fn command_metadata_has_safe_defaults() {
        let info = CommandInfo::new(
            CommandGroup::Query,
            CommandTitle::static_text("Run"),
            || Box::new(Noop),
        );

        assert_eq!(info.label(), "Run");
        assert_eq!(info.target, CommandTarget::None);
        assert!(info.keywords.is_empty());
        assert!(info.show_in_palette);
        assert!(info.shortcut.is_none());
        assert!(info.enabled.is_none());
    }

    gpui_kit::actions!(test, [Noop]);
}
