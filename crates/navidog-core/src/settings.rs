use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// `None` means follow the system.
    pub fn is_dark(self) -> Option<bool> {
        match self {
            Self::System => None,
            Self::Light => Some(false),
            Self::Dark => Some(true),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    System,
    ZhCn,
    En,
}

impl Language {
    /// The rust-i18n locale.
    pub fn locale(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
            Self::System => {
                let sys = std::env::var("LANG").unwrap_or_default();
                if system_prefers_chinese(&sys) {
                    "zh-CN"
                } else {
                    "en"
                }
            }
        }
    }
}

fn system_prefers_chinese(lang: &str) -> bool {
    if lang.starts_with("zh") {
        return true;
    }
    // GUI apps usually start without LANG; read macOS's preferred languages
    #[cfg(target_os = "macos")]
    if lang.is_empty()
        && let Ok(out) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text
            .split(['(', ',', '"', '\n', ' ', ')'])
            .find(|s| !s.is_empty());
        return first.is_some_and(|s| s.starts_with("zh"));
    }
    false
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub appearance: Appearance,
    pub language: Language,
    /// Open the connections marked "auto connect" at startup
    pub auto_connect_on_startup: bool,
    /// General: show fields, indexes, foreign keys and triggers under tables in the navigator
    pub show_table_children: bool,
    /// Tabs: open editors in a new window instead of a new tab
    pub open_in_new_window: bool,
    /// Tabs: editors reopened at startup (the startup screen), as dock panel states
    pub startup_tabs: Vec<serde_json::Value>,
    /// Records: limit the records loaded per page
    pub limit_records: bool,
    pub records_per_page: u64,
    /// Records: begin a transaction automatically when a table viewer opens
    pub auto_begin_transaction: bool,
    /// Objects tab detail view: hidden property columns per object kind ("Choose Columns")
    pub object_hidden_columns: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Appearance::System,
            language: Language::System,
            auto_connect_on_startup: true,
            show_table_children: false,
            open_in_new_window: false,
            startup_tabs: Vec::new(),
            limit_records: true,
            records_per_page: 1000,
            auto_begin_transaction: false,
            object_hidden_columns: Default::default(),
            extra: Default::default(),
        }
    }
}

impl Global for Settings {}

impl Settings {
    fn path() -> std::path::PathBuf {
        paths::config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let bytes = serde_json::to_vec_pretty(self).expect("设置可序列化");
        if let Err(err) = paths::write_atomic(&Self::path(), &bytes) {
            tracing::error!("保存 settings.json 失败: {err}");
        }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Changes the settings and saves them right away.
    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let settings = cx.global_mut::<Self>();
        f(settings);
        settings.save();
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(Settings::load());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unknown_fields_and_defaults_missing_ones() {
        let s: Settings = serde_json::from_str(r#"{"appearance":"dark","future":1}"#).unwrap();
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.language, Language::System);
        assert!(s.auto_connect_on_startup);
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains("\"future\":1"));
    }

    #[test]
    fn explicit_language_locale() {
        assert_eq!(Language::ZhCn.locale(), "zh-CN");
        assert_eq!(Language::En.locale(), "en");
        assert!(system_prefers_chinese("zh_CN.UTF-8"));
    }
}
