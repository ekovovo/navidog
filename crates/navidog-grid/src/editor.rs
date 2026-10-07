//! Cell editors (F-DATA-011). The grid stores text, so hex is the UTF-8 bytes of that text and a
//! bitmap loaded from disk is only previewed until it can be stored as text.
//!
//! ponytail: no syntax-highlight language menu, and the editor is the pane under the grid rather
//! than a second copy in the info pane. Enum values come only from a type string that already
//! lists them (`enum('a','b')` / `set('a','b')`).

use gpui_kit::ImageFormat;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    Hex,
    Image,
    Web,
    Json,
    Enum,
}

pub fn sniff_image(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(ImageFormat::Webp)
    } else if is_svg(bytes) {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(256)];
    let lower = head.to_ascii_lowercase();
    lower.windows(4).any(|w| w == b"<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_magic() {
        assert_eq!(
            sniff_image(b"\x89PNG\r\n\x1a\nrest"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_image(b"<svg xmlns='x'></svg>"),
            Some(ImageFormat::Svg)
        );
        assert!(sniff_image(b"hello").is_none());
    }
}
