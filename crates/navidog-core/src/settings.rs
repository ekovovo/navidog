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

/// Which theme pair to load. Light and dark still come from [`Appearance`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeFamily {
    #[default]
    Navidog,
    Default,
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

/// How data rows are tinted (F-PREF-005). `Alternate` matches the grid's built-in stripe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowShade {
    None,
    #[default]
    Alternate,
    EveryThird,
}

impl RowShade {
    /// Whether data row `row` (0 is the first) gets the alternate background.
    pub fn paints(self, row: usize) -> bool {
        match self {
            Self::None => false,
            Self::Alternate => row % 2 == 1,
            Self::EveryThird => row % 3 == 2,
        }
    }
}

/// Where a query editor shows its results (View > Results, 31-query-results.md §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultsPosition {
    #[default]
    Below,
    Beside,
    NewPage,
}

/// The results tab shown after a run (View > Show After Execution).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowAfterRun {
    #[default]
    Result,
    Messages,
    Summary,
}

/// Header fields shown in the history log viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryHeader {
    pub date: bool,
    pub time: bool,
    pub server: bool,
    pub session: bool,
    pub kind: bool,
    pub elapsed: bool,
}

impl Default for HistoryHeader {
    fn default() -> Self {
        Self {
            date: true,
            time: true,
            server: true,
            session: true,
            kind: true,
            elapsed: true,
        }
    }
}

/// Syntax colors chosen on the Editor page, as `#rrggbb` per token key (`text`, `keyword`,
/// `comment`, `string`, `number`, `background`). A missing key keeps the theme color.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyntaxColors {
    pub light: std::collections::BTreeMap<String, String>,
    pub dark: std::collections::BTreeMap<String, String>,
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
    /// Navidog (Navicat) or gpui-kit's Default (shadcn). Missing means Navidog.
    pub theme: ThemeFamily,
    pub language: Language,
    /// Open the connections marked "auto connect" at startup
    pub auto_connect_on_startup: bool,
    /// General: list object categories under each database or schema in the navigator
    pub show_schema_objects: bool,
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
    /// Reload the page after a successful commit so generated values show up.
    pub sync_record: bool,
    /// A single click starts editing; otherwise a double-click or Return does.
    pub click_to_edit: bool,
    /// When false, the grid uses `grid_font_pt` instead of the interface font size.
    /// ponytail: size only — there is no system font panel, so the family stays the default.
    pub use_default_grid_font: bool,
    pub grid_font_pt: f32,
    pub row_shade: RowShade,
    /// Objects tab detail view: hidden property columns per object kind ("Choose Columns")
    pub object_hidden_columns: std::collections::BTreeMap<String, Vec<String>>,

    // Query editor and results (30-query-editor.md, 31-query-results.md)
    /// Run: keep going after a failed statement
    pub continue_on_error: bool,
    /// Run: leave previous result and explain tabs in place (31 §2)
    pub keep_results: bool,
    pub results_position: ResultsPosition,
    pub show_after_run: ShowAfterRun,
    /// MySQL / MariaDB: profile the run and diff the session status
    pub show_profile: bool,
    /// Editor height (or width beside the results) in pixels after the user dragged the divider,
    /// for every query editor; `None` splits evenly
    pub editor_split: Option<f32>,
    /// Beautify SQL options: short parentheses length and uppercase keywords
    pub beautify_max_inline: usize,
    pub beautify_uppercase: bool,

    // Editor page (F-PREF-004)
    /// Above this many megabytes the editor turns plain text, without completion
    pub highlight_limit_mb: u32,
    pub show_line_numbers: bool,
    pub code_folding: bool,
    pub bracket_highlight: bool,
    pub syntax_highlight: bool,
    pub insert_spaces: bool,
    pub tab_width: u8,
    pub word_wrap: bool,
    /// When false, the editor uses `editor_font_pt`. ponytail: size only, like the grid font.
    pub use_default_editor_font: bool,
    pub editor_font_pt: f32,
    pub syntax_colors: SyntaxColors,

    // Code completion page (F-PREF-006)
    pub code_completion: bool,
    pub auto_update_outline: bool,
    pub outline_system_objects: bool,
    pub auto_select_first: bool,

    /// Auto recovery page (F-PREF-008): seconds between autosaves; `None` is "Never"
    pub autosave_secs: Option<u32>,

    pub history_header: HistoryHeader,

    // Advanced page (F-PREF-010)
    /// Log folder; `None` is the default (`paths::log_dir`)
    pub log_dir: Option<String>,
    /// The app log records debug messages
    pub diagnostic_log: bool,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Appearance::System,
            theme: ThemeFamily::Navidog,
            language: Language::System,
            auto_connect_on_startup: true,
            show_schema_objects: true,
            show_table_children: true,
            open_in_new_window: false,
            startup_tabs: Vec::new(),
            limit_records: true,
            records_per_page: 1000,
            auto_begin_transaction: false,
            sync_record: true,
            click_to_edit: false,
            use_default_grid_font: true,
            grid_font_pt: 13.,
            row_shade: RowShade::Alternate,
            object_hidden_columns: Default::default(),
            continue_on_error: false,
            keep_results: false,
            results_position: ResultsPosition::Below,
            show_after_run: ShowAfterRun::Result,
            show_profile: false,
            editor_split: None,
            beautify_max_inline: 50,
            beautify_uppercase: true,
            highlight_limit_mb: 10,
            show_line_numbers: true,
            code_folding: true,
            bracket_highlight: true,
            syntax_highlight: true,
            insert_spaces: false,
            tab_width: 4,
            word_wrap: false,
            use_default_editor_font: true,
            editor_font_pt: 12.,
            syntax_colors: SyntaxColors::default(),
            code_completion: true,
            auto_update_outline: true,
            outline_system_objects: false,
            auto_select_first: true,
            autosave_secs: Some(30),
            history_header: HistoryHeader::default(),
            log_dir: None,
            diagnostic_log: false,
            extra: Default::default(),
        }
    }
}

impl Global for Settings {}

impl Settings {
    fn path() -> std::path::PathBuf {
        paths::config_dir().join("settings.json")
    }

    /// Grid font sizes the Records page accepts, in points.
    pub const GRID_FONT_PT: std::ops::RangeInclusive<f32> = 8.0..=72.0;

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    fn load_from(path: &std::path::Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            // The next save writes defaults over this file: keep a copy, as connections.json does
            tracing::error!("settings.json cannot be parsed, backed up: {err}");
            let _ = std::fs::copy(path, path.with_extension("json.bak"));
            Self::default()
        })
    }

    pub fn save(&self) {
        let bytes = serde_json::to_vec_pretty(self).expect("settings serialize");
        if let Err(err) = paths::write_atomic(&Self::path(), &bytes) {
            tracing::error!("failed to save settings.json: {err}");
        }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// The folder for the app log and QueryExec.log.
    pub fn log_dir(&self) -> std::path::PathBuf {
        self.log_dir
            .as_deref()
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(paths::log_dir)
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
    fn an_unreadable_file_is_backed_up_before_defaults_replace_it() {
        let dir = std::env::temp_dir().join(format!("navidog-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"row_shade":"not-a-variant"}"#).unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let backup = std::fs::read_to_string(dir.join("settings.json.bak")).unwrap();
        assert!(backup.contains("not-a-variant"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_unknown_fields_and_defaults_missing_ones() {
        let s: Settings = serde_json::from_str(r#"{"appearance":"dark","future":1}"#).unwrap();
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.theme, ThemeFamily::Navidog);
        assert_eq!(s.language, Language::System);
        assert!(s.auto_connect_on_startup);
        assert!(s.show_schema_objects);
        assert!(s.show_table_children);
        assert!(s.sync_record);
        assert!(!s.click_to_edit);
        assert!(s.use_default_grid_font);
        assert_eq!(s.grid_font_pt, 13.);
        assert_eq!(s.row_shade, RowShade::Alternate);
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains("\"future\":1"));
    }

    #[test]
    fn theme_defaults_to_navidog_and_reads_default() {
        let missing: Settings = serde_json::from_str(r#"{"appearance":"dark"}"#).unwrap();
        assert_eq!(missing.theme, ThemeFamily::Navidog);
        let chosen: Settings = serde_json::from_str(r#"{"theme":"default"}"#).unwrap();
        assert_eq!(chosen.theme, ThemeFamily::Default);
    }

    #[test]
    fn row_shade_paints_the_matching_rows() {
        assert!(!RowShade::None.paints(1));
        assert!(RowShade::Alternate.paints(1));
        assert!(!RowShade::Alternate.paints(0));
        assert!(RowShade::EveryThird.paints(2));
        assert!(!RowShade::EveryThird.paints(1));
    }

    #[test]
    fn explicit_language_locale() {
        assert_eq!(Language::ZhCn.locale(), "zh-CN");
        assert_eq!(Language::En.locale(), "en");
        assert!(system_prefers_chinese("zh_CN.UTF-8"));
    }
}
