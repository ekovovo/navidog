//! Design tokens. Every value and its source is registered in docs/design/02-design-tokens.md.
//! This is the only place where measured Navicat values may appear (the gpui-kit guidelines allow
//! pixel values in the token definitions themselves).

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Pixels, Rems, Size, rgba, size};

/// rem base: the default macOS control font size, matching `theme.font_size`.
pub const BASE_PT: f32 = 13.;

/// A size recorded in macOS pt; the UI uses it via [`Pt::rems`] so it scales with the app zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt(pub f32);

impl Pt {
    pub const fn rems(self) -> Rems {
        Rems(self.0 / BASE_PT)
    }

    /// For APIs that only accept `Pixels` (window size, Dock size); converts with the current rem.
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        self.rems().to_pixels(rem_size)
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

pub mod font {
    use super::Pt;
    pub const SIZE_BODY: Pt = Pt(13.);
    pub const SIZE_SMALL: Pt = Pt(11.);
    pub const LINE_BODY: Pt = Pt(16.);
    pub const LINE_SMALL: Pt = Pt(14.);
}

pub mod window {
    use super::Pt;
    pub const DEFAULT_WIDTH: Pt = Pt(1240.);
    pub const DEFAULT_HEIGHT: Pt = Pt(800.);
    // Provisional until measured with AX
    pub const MIN_WIDTH: Pt = Pt(800.);
    pub const MIN_HEIGHT: Pt = Pt(500.);
    /// Traffic-light area height, matching gpui-component `TitleBar`
    pub const TITLE_HEIGHT: Pt = Pt(28.);
}

pub mod toolbar {
    use super::Pt;
    // Provisional until measured with AX
    pub const HEIGHT: Pt = Pt(52.);
    pub const ITEM_ICON: Pt = Pt(24.);
    pub const ITEM_MIN_WIDTH: Pt = Pt(52.);
    pub const ITEM_GAP: Pt = Pt(8.);
    pub const INSET_X: Pt = Pt(12.);
}

pub mod pane {
    use super::Pt;
    pub const NAVIGATOR_WIDTH: Pt = Pt(240.);
    pub const OBJECT_WIDTH: Pt = Pt(734.);
    pub const INFO_WIDTH: Pt = Pt(264.);
    pub const DIVIDER: Pt = Pt(1.);
}

pub mod statusbar {
    use super::Pt;
    pub const HEIGHT: Pt = Pt(24.);
    pub const TEXT_INSET_X: Pt = Pt(18.);
}

pub mod navigator {
    use super::Pt;
    pub const FILTER_HEIGHT: Pt = Pt(34.);
    pub const FILTER_INSET_X: Pt = Pt(10.);
    pub const FILTER_SEARCH_HEIGHT: Pt = Pt(24.);
    pub const FILTER_ICON: Pt = Pt(16.);
    pub const FILTER_ICON_GAP: Pt = Pt(8.);
    // Provisional until measured with AX
    pub const ROW_HEIGHT: Pt = Pt(22.);
    pub const ROW_INDENT: Pt = Pt(16.);
    pub const ROW_ICON: Pt = Pt(16.);
    pub const ROW_INSET_X: Pt = Pt(8.);
}

pub mod tabbar {
    use super::Pt;
    pub const HEIGHT: Pt = Pt(37.);
    pub const SEPARATOR: Pt = Pt(1.);
    // Provisional until measured with AX
    pub const TAB_MIN_WIDTH: Pt = Pt(120.);
}

pub mod list {
    use super::Pt;
    pub const HEADER_HEIGHT: Pt = Pt(23.);
    // Provisional until measured with AX
    pub const ROW_HEIGHT: Pt = Pt(22.);
    pub const NAME_COLUMN_WIDTH: Pt = Pt(220.);
    pub const DETAIL_COLUMN_WIDTH: Pt = Pt(130.);
}

pub mod object_toolbar {
    use super::Pt;
    // Provisional until measured with AX
    pub const HEIGHT: Pt = Pt(36.);
}

pub mod editor_tab {
    use super::Pt;
    // Provisional until measured with AX
    pub const TITLE_MAX_WIDTH: Pt = Pt(160.);
}

pub mod grid {
    use super::Pt;
    pub const HEADER_HEIGHT: Pt = Pt(28.);
    pub const ROW_NUMBER_WIDTH: Pt = Pt(20.);
    // Provisional until measured with AX
    pub const ROW_HEIGHT: Pt = Pt(20.);
    pub const COLUMN_WIDTH: Pt = Pt(140.);
    // nib:NAVDateTimePicker/NSWindowRect (317 × 247)
    pub const DATE_PICKER_WIDTH: Pt = Pt(317.);
}

pub mod viewer {
    use super::Pt;
    pub const BOTTOM_BAR_HEIGHT: Pt = Pt(28.);
    pub const BOTTOM_BUTTON_WIDTH: Pt = Pt(20.);
    pub const BOTTOM_BUTTON_HEIGHT: Pt = Pt(16.);
    pub const LIMIT_WIDTH: Pt = Pt(212.);
    pub const PAGER_WIDTH: Pt = Pt(86.);
    // Provisional until measured with AX
    pub const FILTER_HEIGHT: Pt = Pt(28.);
    pub const CELL_EDITOR_HEIGHT: Pt = Pt(120.);
}

pub mod info {
    use super::Pt;
    pub const HEADER_HEIGHT: Pt = Pt(30.);
    pub const HEADER_BUTTON: Pt = Pt(16.);
    pub const HEADER_BUTTON_INSET: Pt = Pt(8.);
    // Provisional until measured with AX
    pub const ROW_GAP: Pt = Pt(6.);
    pub const INSET: Pt = Pt(12.);
}

pub mod form {
    use super::Pt;
    pub const MARGIN: Pt = Pt(20.);
    pub const ROW_HEIGHT: Pt = Pt(24.);
    pub const ROW_GAP: Pt = Pt(8.);
    pub const COMPACT_ROW_HEIGHT: Pt = Pt(22.);
    pub const COMPACT_ROW_GAP: Pt = Pt(8.);
    pub const LABEL_HEIGHT: Pt = Pt(16.);
    pub const LABEL_GAP: Pt = Pt(6.);
    pub const FIELD_WIDTH: Pt = Pt(300.);
    pub const FIELD_WIDTH_SHORT: Pt = Pt(80.);
    pub const FIELD_WIDTH_WIDE: Pt = Pt(692.);
    pub const POPUP_HEIGHT: Pt = Pt(20.);
    pub const POPUP_WIDTH: Pt = Pt(300.);
    pub const POPUP_WIDTH_MEDIUM: Pt = Pt(280.);
    pub const POPUP_WIDTH_SHORT: Pt = Pt(196.);
    pub const CHECKBOX_HEIGHT: Pt = Pt(16.);
    pub const NOTE_HEIGHT: Pt = Pt(14.);
    pub const LIST_HEADER_HEIGHT: Pt = Pt(23.);
    pub const LIST_ROW_HEIGHT: Pt = Pt(17.);
    pub const LIST_WIDTH: Pt = Pt(500.);
    pub const LIST_HEIGHT: Pt = Pt(300.);
    pub const LIST_BUTTON_WIDTH: Pt = Pt(24.);
    pub const LIST_BUTTON_HEIGHT: Pt = Pt(22.);
    /// Label column width: the nib aligns it to the longest label on each page; this is the largest
    /// value across the three connection forms
    pub const LABEL_COLUMN: Pt = Pt(150.);
}

pub mod button {
    use super::Pt;
    // Bordered button frame height from the source nib; to be measured with AX on macOS 26
    pub const HEIGHT: Pt = Pt(24.);
    pub const MIN_WIDTH: Pt = Pt(80.);
}

pub mod conn_type {
    use super::Pt;
    pub const WIDTH: Pt = Pt(925.);
    pub const HEIGHT: Pt = Pt(365.);
    pub const SIDEBAR_WIDTH: Pt = Pt(317.);
    pub const HEADER_HEIGHT: Pt = Pt(40.);
    pub const SEARCH_ROW_HEIGHT: Pt = Pt(36.);
    pub const SEARCH_WIDTH: Pt = Pt(200.);
    pub const FOOTER_HEIGHT: Pt = Pt(45.);
    // Provisional until measured with AX
    pub const TILE_SIZE: Pt = Pt(96.);
    pub const TILE_ICON: Pt = Pt(48.);
}

pub mod conn_form {
    use super::Pt;
    // The connection form window is built in code, not in a nib; provisional until measured with AX
    pub const WIDTH: Pt = Pt(560.);
    pub const BODY_HEIGHT: Pt = Pt(420.);
}

pub mod dialog {
    use super::Pt;
    // Provisional until measured with AX (small dialogs such as the password prompt and master
    // password unlock)
    pub const SMALL_WIDTH: Pt = Pt(460.);
}

pub mod preferences {
    use super::Pt;
    // nib:PreferencesWindowController/NSWindowRect
    pub const WIDTH: Pt = Pt(640.);
    pub const HEIGHT: Pt = Pt(480.);
    // Provisional until measured with AX (the category list; the nib uses autolayout)
    pub const NAV_WIDTH: Pt = Pt(160.);
}

/// Manage connections window (ManageConnectionViewController.nib).
pub mod manage_conn {
    use super::Pt;
    pub const WIDTH: Pt = Pt(800.);
    pub const HEIGHT: Pt = Pt(600.);
    pub const BAR_HEIGHT: Pt = Pt(28.);
    pub const COLUMN_WIDTH: Pt = Pt(150.);
    pub const SEARCH_WIDTH: Pt = Pt(200.);
}

/// Preferences window size (no rem exists yet when the window is created, so convert 1:1).
pub fn preferences_window_size() -> Size<Pixels> {
    window_size(preferences::WIDTH, preferences::HEIGHT)
}

/// Size of a window opened before any rem exists (converted 1:1).
pub fn window_size(w: Pt, h: Pt) -> Size<Pixels> {
    size(gpui_kit::px(w.0), gpui_kit::px(h.0))
}

/// Default window size (no rem exists yet when the window is created, so convert 1:1).
pub fn default_window_size() -> Size<Pixels> {
    size(
        gpui_kit::px(window::DEFAULT_WIDTH.0),
        gpui_kit::px(window::DEFAULT_HEIGHT.0),
    )
}

pub fn min_window_size() -> Size<Pixels> {
    size(
        gpui_kit::px(window::MIN_WIDTH.0),
        gpui_kit::px(window::MIN_HEIGHT.0),
    )
}

/// AppKit semantic colors outside the gpui-component theme; values are listed in §8 of the token table.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub window_bg: Hsla,
    pub control_bg: Hsla,
    pub label: Hsla,
    pub label_secondary: Hsla,
    pub label_tertiary: Hsla,
    pub separator: Hsla,
    pub grid: Hsla,
    pub row_alt: Hsla,
    pub accent: Hsla,
    pub selection: Hsla,
    pub selection_inactive: Hsla,
    pub selection_text: Hsla,
    pub disabled_text: Hsla,
    pub fill: Hsla,
    pub fill_tertiary: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    /// Modified, uncommitted cells in the grid (provisional until measured in Navicat)
    pub grid_edited: Hsla,
    /// Rows pending deletion in the grid (provisional until measured in Navicat)
    pub grid_deleted: Hsla,
    /// Search matches (NSColor.findHighlightColor = systemYellow), drawn under black text
    pub find_highlight: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub fn light_palette() -> Palette {
    Palette {
        window_bg: c(0xffffffff),
        control_bg: c(0xffffffff),
        label: c(0x000000d8),
        label_secondary: c(0x0000007f),
        label_tertiary: c(0x00000042),
        separator: c(0x00000019),
        grid: c(0xe6e6e6ff),
        row_alt: c(0xf4f5f5ff),
        accent: c(0x007affff),
        selection: c(0x0064e1ff),
        selection_inactive: c(0xdcdcdcff),
        selection_text: c(0xffffffff),
        disabled_text: c(0x0000003f),
        fill: c(0x00000019),
        fill_tertiary: c(0x0000000c),
        danger: c(0xff383cff),
        success: c(0x34c759ff),
        grid_edited: c(0xfff3c4ff),
        grid_deleted: c(0xffd9d9ff),
        find_highlight: c(0xffcc00ff),
    }
}

pub fn dark_palette() -> Palette {
    Palette {
        window_bg: c(0x1e1e1eff),
        control_bg: c(0x1e1e1eff),
        label: c(0xffffffd8),
        label_secondary: c(0xffffff8c),
        label_tertiary: c(0xffffff3f),
        separator: c(0xffffff19),
        grid: c(0x1a1a1aff),
        row_alt: c(0xffffff0c),
        accent: c(0x007affff),
        selection: c(0x0059d1ff),
        selection_inactive: c(0x464646ff),
        selection_text: c(0xffffffff),
        disabled_text: c(0xffffff3f),
        fill: c(0xffffff19),
        fill_tertiary: c(0xffffff0c),
        danger: c(0xff4245ff),
        success: c(0x30d158ff),
        grid_edited: c(0x5c4b1aff),
        grid_deleted: c(0x5c2424ff),
        find_highlight: c(0xffd60aff),
    }
}

/// The palette for the current appearance.
pub fn palette(cx: &App) -> Palette {
    if cx.theme().is_dark() {
        dark_palette()
    } else {
        light_palette()
    }
}

/// Connection colors (F-CONN-014), taken from the AppKit system colors.
pub fn connection_color(name: &str, cx: &App) -> Option<Hsla> {
    let dark = cx.theme().is_dark();
    let hex = match (name, dark) {
        ("red", false) => 0xff383cff,
        ("red", true) => 0xff4245ff,
        ("orange", false) => 0xff8d28ff,
        ("orange", true) => 0xff9230ff,
        ("yellow", false) => 0xffcc00ff,
        ("yellow", true) => 0xffd600ff,
        ("green", false) => 0x34c759ff,
        ("green", true) => 0x30d158ff,
        ("blue", false) => 0x0088ffff,
        ("blue", true) => 0x0091ffff,
        ("purple", false) => 0xcb30e0ff,
        ("purple", true) => 0xdb34f2ff,
        ("gray", false) => 0x8e8e93ff,
        ("gray", true) => 0x98989dff,
        _ => return None,
    };
    Some(c(hex))
}

pub const CONNECTION_COLORS: &[&str] =
    &["red", "orange", "yellow", "green", "blue", "purple", "gray"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pt_converts_to_rem_on_13pt_base() {
        assert_eq!(Pt(13.).rems().0, 1.);
        assert_eq!(Pt(26.).rems().0, 2.);
        assert_eq!(Pt(24.).to_pixels(gpui_kit::px(13.)), gpui_kit::px(24.));
        // Scales up proportionally at a 16pt rem
        assert_eq!(Pt(26.).to_pixels(gpui_kit::px(16.)), gpui_kit::px(32.));
    }
}
