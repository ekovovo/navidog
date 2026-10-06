use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

const NAVICAT_THEME: &str = include_str!("../themes/navicat.json");
pub const LIGHT: &str = "Navicat Light";
pub const DARK: &str = "Navicat Dark";

/// 注册 Navicat 主题并设为浅色/深色两套默认主题。在 `gpui_kit::init` 之后调用。
pub fn init(cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(NAVICAT_THEME)
        .expect("内置主题 navicat.json 必须能解析");
    let registry = ThemeRegistry::global(cx);
    let light = registry.themes().get(LIGHT).cloned().expect("缺少浅色主题");
    let dark = registry.themes().get(DARK).cloned().expect("缺少深色主题");
    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
    Theme::change(ThemeMode::Light, None, cx);
}

/// 应用外观：`None` 跟随系统，`Some(true)` 固定深色，`Some(false)` 固定浅色。
pub fn apply_appearance(dark: Option<bool>, window: Option<&mut Window>, cx: &mut App) {
    match dark {
        None => Theme::sync_system_appearance(window, cx),
        Some(true) => Theme::change(ThemeMode::Dark, None, cx),
        Some(false) => Theme::change(ThemeMode::Light, None, cx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeSet;

    #[test]
    fn theme_parses_with_both_modes_on_13pt_base() {
        let set: ThemeSet = serde_json::from_str(NAVICAT_THEME).unwrap();
        let names: Vec<_> = set.themes.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, [LIGHT, DARK]);
        assert!(set.themes.iter().all(|t| t.font_size == Some(13.)));
        assert!(set.themes[1].mode.is_dark());
    }
}
