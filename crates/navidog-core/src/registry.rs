//! 扩展点注册表（03-plugin-api.md 第 2 节）。内置功能与插件走同一套注册表，
//! 条目带 `Source`，插件停用时按来源整体移除。

use gpui_kit::{Action, App, Global, SharedString};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Builtin,
    Plugin(SharedString),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("`{0}` 已被注册")]
    Duplicate(String),
}

/// 按键索引并保持注册顺序的注册表。
pub struct Registry<V> {
    entries: Vec<(SharedString, Source, V)>,
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
        key: impl Into<SharedString>,
        source: Source,
        value: V,
    ) -> Result<(), RegistryError> {
        let key = key.into();
        if self.get(&key).is_some() {
            return Err(RegistryError::Duplicate(key.to_string()));
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

    pub fn iter(&self) -> impl Iterator<Item = (&SharedString, &Source, &V)> {
        self.entries.iter().map(|(k, s, v)| (k, s, v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 命令：一个命令只对应一个 Action，菜单、工具栏、右键菜单、快捷键共用。
pub struct CommandInfo {
    /// i18n 键
    pub title: SharedString,
    /// 图标资源路径，如 `icons/plug.svg`
    pub icon: Option<SharedString>,
    pub shortcut: Option<SharedString>,
    pub action: fn() -> Box<dyn Action>,
}

/// Dock 面板元数据；面板构造由各 feature crate 通过 gpui-base 的 `register_panel` 完成。
pub struct PanelInfo {
    pub title: SharedString,
    pub placement: PanelPlacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPlacement {
    Left,
    Center,
    Right,
    Bottom,
}

/// 导入导出格式（M6 实现）。
pub struct FormatInfo {
    pub name: SharedString,
    pub extensions: Vec<SharedString>,
    pub import: bool,
    pub export: bool,
}

/// 声明式单元格格式化规则（M5 实现，见 03-plugin-api.md 第 7 节）。
pub struct CellFormatterInfo {
    pub rule: serde_json::Value,
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
global_registry!(PanelRegistry, PanelInfo);
global_registry!(FormatRegistry, FormatInfo);
global_registry!(CellFormatterRegistry, CellFormatterInfo);

pub(crate) fn init(cx: &mut App) {
    cx.set_global(CommandRegistry::default());
    cx.set_global(PanelRegistry::default());
    cx.set_global(FormatRegistry::default());
    cx.set_global(CellFormatterRegistry::default());
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
}
