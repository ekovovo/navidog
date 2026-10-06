//! state.json: window position and size, Dock layout, visibility of each pane (10-shell.md §11).

use gpui_kit::{Bounds, Global, Pixels, WindowBounds, point, px, size};
use navidog_core::paths;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Frame {
    pub fn from_bounds(bounds: WindowBounds) -> Self {
        let b = bounds.get_bounds();
        Self {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
            maximized: matches!(bounds, WindowBounds::Maximized(_)),
        }
    }

    pub fn to_bounds(self) -> WindowBounds {
        // raw-px: window geometry is measured values from the previous run
        let origin = point(px(self.x), px(self.y));
        // raw-px: same as above
        let b: Bounds<Pixels> = Bounds::new(origin, size(px(self.width), px(self.height)));
        if self.maximized {
            WindowBounds::Maximized(b)
        } else {
            WindowBounds::Windowed(b)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub version: u32,
    pub window: Option<Frame>,
    pub dock: Option<serde_json::Value>,
    pub show_toolbar: bool,
    pub show_status_bar: bool,
    /// View > Navigation Pane options (flattened: older files kept these at the top level)
    #[serde(flatten)]
    pub view: navidog_navigator::ViewOptions,
}

/// The menu bar reads its check marks from this global copy, which the main window keeps in sync.
impl Global for AppState {}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: 1,
            window: None,
            dock: None,
            show_toolbar: true,
            show_status_bar: true,
            view: Default::default(),
        }
    }
}

impl AppState {
    fn path() -> std::path::PathBuf {
        paths::config_dir().join("state.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let bytes = serde_json::to_vec_pretty(self).expect("状态可序列化");
        if let Err(e) = paths::write_atomic(&Self::path(), &bytes) {
            tracing::error!("保存 state.json 失败: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips_through_window_bounds() {
        let f = Frame {
            x: 10.,
            y: 20.,
            width: 1240.,
            height: 800.,
            maximized: true,
        };
        assert_eq!(Frame::from_bounds(f.to_bounds()), f);
        let s: AppState = serde_json::from_str(r#"{"show_toolbar":false}"#).unwrap();
        assert!(!s.show_toolbar && s.show_status_bar);
    }
}
