use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

const NAVICAT_THEME: &str = include_str!("../themes/navicat.json");
pub const LIGHT: &str = "Navicat Light";
pub const DARK: &str = "Navicat Dark";

/// Registers the Navicat themes as the default light and dark themes. Call after `gpui_kit::init`.
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
    // Like NSScrollView: always-visible scroll bars when macOS says so (e.g. a mouse is connected)
    Theme::sync_scrollbar_appearance(cx);
}

/// Applies the appearance: `None` follows the system, `Some(true)` forces dark, `Some(false)` forces light.
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
    use crate::tokens::{dark_palette, light_palette};
    use gpui_kit::component::ThemeSet;

    #[test]
    fn theme_parses_with_both_modes_on_13pt_base() {
        let set: ThemeSet = serde_json::from_str(NAVICAT_THEME).unwrap();
        let names: Vec<_> = set.themes.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, [LIGHT, DARK]);
        assert!(set.themes.iter().all(|t| t.font_size == Some(13.)));
        assert!(set.themes[1].mode.is_dark());
    }

    /// A theme without `highlight` keeps the previous mode's editor colors (white DDL in dark mode).
    #[gpui_kit::test]
    fn editor_colors_follow_appearance(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
        });
        cx.run_until_parked();
        for (mode, palette) in [
            (ThemeMode::Dark, dark_palette()),
            (ThemeMode::Light, light_palette()),
        ] {
            cx.update(|cx| Theme::change(mode, None, cx));
            let style = cx.update(|cx| Theme::global(cx).highlight_theme.style.clone());
            assert_eq!(
                style.editor_background,
                Some(palette.control_bg),
                "{mode:?}"
            );
            assert!(style.syntax.keyword.is_some(), "{mode:?}");
        }
    }
}
