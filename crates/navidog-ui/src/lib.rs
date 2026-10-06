//! navidog 的设计系统：Navicat 主题、具名 token、AppKit 外观皮肤与图标资源。

pub mod assets;
pub mod skin;
pub mod theme;
pub mod tokens;

pub use assets::NavidogAssets;
pub use tokens::{Pt, palette};
