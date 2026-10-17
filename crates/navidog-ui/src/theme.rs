use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

const NAVICAT_THEME: &str = include_str!("../themes/navicat.json");
pub const LIGHT: &str = "Navicat Light";
pub const DARK: &str = "Navicat Dark";
pub const DEFAULT_LIGHT: &str = "Default Light";
pub const DEFAULT_DARK: &str = "Default Dark";

/// Navidog (Navicat colors) or gpui-kit's built-in Default (shadcn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeFamily {
    Navidog,
    Default,
}

/// Registers the Navicat themes as the default light and dark themes. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(NAVICAT_THEME)
        .expect("bundled navicat.json must parse");
    let registry = ThemeRegistry::global(cx);
    let light = registry
        .themes()
        .get(LIGHT)
        .cloned()
        .expect("missing light theme");
    let dark = registry
        .themes()
        .get(DARK)
        .cloned()
        .expect("missing dark theme");
    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
    Theme::change(ThemeMode::Light, None, cx);
    // Show the bar when the pointer is over the scroll area.
    Theme::set_scrollbar_mode(ScrollbarMode::Hover, cx);
}

/// Points the light and dark slots at one family. Call [`apply_appearance`] afterwards so
/// `Theme::change` loads the slot for the current mode.
pub fn apply_family(family: ThemeFamily, cx: &mut App) {
    let (light_name, dark_name) = match family {
        ThemeFamily::Navidog => (LIGHT, DARK),
        ThemeFamily::Default => (DEFAULT_LIGHT, DEFAULT_DARK),
    };
    let registry = ThemeRegistry::global(cx);
    let Some(light) = registry.themes().get(light_name).cloned() else {
        tracing::error!("missing theme {light_name}");
        return;
    };
    let Some(dark) = registry.themes().get(dark_name).cloned() else {
        tracing::error!("missing theme {dark_name}");
        return;
    };
    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
}

/// Keeps the rem base at 13pt. The Default theme omits `font.size`, and gpui's built-in size is 16.
pub fn pin_layout_font(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_size = gpui_kit::px(13.); // raw-px: layout rem base
        theme.mono_font_size = gpui_kit::px(12.); // raw-px: mono size from navicat.json
    });
}

/// Applies the appearance: `None` follows the system, `Some(true)` forces dark, `Some(false)` forces light.
pub fn apply_appearance(dark: Option<bool>, window: Option<&mut Window>, cx: &mut App) {
    match dark {
        None => Theme::sync_system_appearance(window, cx),
        Some(true) => Theme::change(ThemeMode::Dark, None, cx),
        Some(false) => Theme::change(ThemeMode::Light, None, cx),
    }
}

/// Syntax color keys the Editor preferences can override (F-PREF-004).
pub const SYNTAX_KEYS: [&str; 6] = [
    "text",
    "keyword",
    "comment",
    "string",
    "number",
    "background",
];

/// Puts the user's syntax colors (`#rrggbb` by key) over the active theme's editor colors.
/// Theme switches reload the theme, so call this after every appearance change.
pub fn apply_syntax_colors(colors: &std::collections::BTreeMap<String, String>, cx: &mut App) {
    if colors.is_empty() {
        return;
    }
    let mut theme = (*Theme::global(cx).highlight_theme).clone();
    for (key, hex) in colors {
        let Some(color) = parse_hex(hex) else {
            continue;
        };
        let style = || serde_json::from_value(serde_json::json!({ "color": hex })).ok();
        let syntax = &mut theme.style.syntax;
        match key.as_str() {
            "text" => theme.style.editor_foreground = Some(color),
            "background" => theme.style.editor_background = Some(color),
            "keyword" => syntax.keyword = style(),
            "comment" => {
                syntax.comment = style();
                syntax.comment_doc = style();
            }
            "string" => syntax.string = style(),
            "number" => syntax.number = style(),
            _ => {}
        }
    }
    Theme::global_mut(cx).highlight_theme = std::sync::Arc::new(theme);
    cx.refresh_windows();
}

/// The color of a syntax key in the active theme, for the color pickers.
pub fn syntax_color(key: &str, cx: &App) -> Option<gpui_kit::Hsla> {
    let style = &Theme::global(cx).highlight_theme.style;
    let pick = |s: Option<gpui_kit::component::highlighter::ThemeStyle>| {
        s.and_then(|s| gpui_kit::HighlightStyle::from(s).color)
    };
    match key {
        "text" => style.editor_foreground,
        "background" => style.editor_background,
        "keyword" => pick(style.syntax.keyword),
        "comment" => pick(style.syntax.comment),
        "string" => pick(style.syntax.string),
        "number" => pick(style.syntax.number),
        _ => None,
    }
}

pub fn parse_hex(hex: &str) -> Option<gpui_kit::Hsla> {
    let digits = hex.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(digits, 16).ok()?;
    // raw-px: a color the user picked
    Some(gpui_kit::rgb(v).into())
}

pub fn to_hex(color: gpui_kit::Hsla) -> String {
    let c = gpui_kit::Rgba::from(color);
    let byte = |f: f32| (f.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{dark_palette, light_palette, palette};
    use gpui_kit::component::ThemeSet;

    #[test]
    fn theme_parses_with_both_modes_on_13pt_base() {
        let set: ThemeSet = serde_json::from_str(NAVICAT_THEME).unwrap();
        let names: Vec<_> = set.themes.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, [LIGHT, DARK]);
        assert!(set.themes.iter().all(|t| t.font_size == Some(13.)));
        assert!(set.themes[1].mode.is_dark());
    }

    #[test]
    fn hex_round_trips() {
        let c = parse_hex("#0433ff").unwrap();
        assert_eq!(to_hex(c), "#0433ff");
        assert!(parse_hex("blue").is_none());
    }

    #[gpui_kit::test]
    fn a_missing_theme_leaves_the_current_one(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            // Navicat themes are not loaded. This used to panic at startup.
            apply_family(ThemeFamily::Navidog, cx);
        });
    }

    /// The user's colors replace the theme's editor colors.
    #[gpui_kit::test]
    fn syntax_colors_override_the_theme(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let colors = std::collections::BTreeMap::from([
                ("keyword".to_string(), "#ff0000".to_string()),
                ("background".to_string(), "#000000".to_string()),
            ]);
            apply_syntax_colors(&colors, cx);
            assert_eq!(
                syntax_color("keyword", cx).map(to_hex).as_deref(),
                Some("#ff0000")
            );
            assert_eq!(
                syntax_color("background", cx).map(to_hex).as_deref(),
                Some("#000000")
            );
        });
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

    /// Default uses gpui-kit's shadcn themes; Navidog restores the measured palette. Font stays 13pt.
    #[gpui_kit::test]
    fn switching_family_swaps_the_theme_and_palette(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
        });
        cx.run_until_parked();
        for (family, mode, name) in [
            (ThemeFamily::Default, ThemeMode::Light, DEFAULT_LIGHT),
            (ThemeFamily::Default, ThemeMode::Dark, DEFAULT_DARK),
            (ThemeFamily::Navidog, ThemeMode::Light, LIGHT),
            (ThemeFamily::Navidog, ThemeMode::Dark, DARK),
        ] {
            cx.update(|cx| {
                apply_family(family, cx);
                Theme::change(mode, None, cx);
                if family == ThemeFamily::Default {
                    // gpui's built-in size, which Default does not override
                    Theme::update(cx, |theme| theme.font_size = gpui_kit::px(16.)); // raw-px: gpui built-in size
                    pin_layout_font(cx);
                }
            });
            cx.update(|cx| {
                assert_eq!(Theme::global(cx).theme_name().as_ref(), name);
                let colors = palette(cx);
                if family == ThemeFamily::Navidog {
                    let expected = if mode.is_dark() {
                        dark_palette()
                    } else {
                        light_palette()
                    };
                    assert_eq!(colors, expected, "{name}");
                } else {
                    assert_eq!(colors.window_bg, cx.theme().background, "{name}");
                    assert_eq!(colors.selection, cx.theme().list_active_border, "{name}");
                    assert_eq!(colors.selection_text, cx.theme().foreground, "{name}");
                    assert_eq!(colors.row_selected, cx.theme().list_active, "{name}");
                    assert_eq!(cx.theme().font_size, gpui_kit::px(13.), "{name}"); // raw-px: layout rem base
                    assert_eq!(cx.theme().mono_font_size, gpui_kit::px(12.), "{name}"); // raw-px: mono size from navicat.json
                }
            });
        }
    }
}
