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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Choices {
    /// MySQL `enum`: one value.
    One(Vec<String>),
    /// MySQL `set`: a comma-separated subset.
    Many(Vec<String>),
}

pub fn encode_hex(text: &str) -> String {
    text.bytes()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `None` when the hex is malformed or the bytes are not UTF-8.
pub fn decode_hex(text: &str) -> Option<String> {
    let hex: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut i = 0;
    while i < hex.len() {
        let hi = hex_val(hex[i])?;
        let lo = hex_val(hex[i + 1])?;
        bytes.push((hi << 4) | lo);
        i += 2;
    }
    String::from_utf8(bytes).ok()
}

pub fn pretty_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Parses `enum(...)` / `set(...)` only when the type string itself lists the values.
pub fn choices(type_name: &str) -> Option<Choices> {
    let t = type_name.trim();
    let (many, rest) = if let Some(rest) = strip_kind(t, "set") {
        (true, rest)
    } else {
        let rest = strip_kind(t, "enum")?;
        (false, rest)
    };
    let values = parse_sql_list(rest)?;
    if values.is_empty() {
        return None;
    }
    Some(if many {
        Choices::Many(values)
    } else {
        Choices::One(values)
    })
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

fn strip_kind<'a>(type_name: &'a str, kind: &str) -> Option<&'a str> {
    if type_name.len() < kind.len() || !type_name[..kind.len()].eq_ignore_ascii_case(kind) {
        return None;
    }
    let rest = type_name[kind.len()..].trim_start();
    let rest = rest.strip_prefix('(')?.trim_end();
    rest.strip_suffix(')')
}

fn parse_sql_list(s: &str) -> Option<Vec<String>> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'\'' {
            return None;
        }
        i += 1;
        let mut value = String::new();
        let mut closed = false;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    value.push('\'');
                    i += 2;
                    continue;
                }
                i += 1;
                closed = true;
                break;
            }
            let ch = s[i..].chars().next()?;
            value.push(ch);
            i += ch.len_utf8();
        }
        if !closed {
            return None;
        }
        out.push(value);
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() {
            if bytes[i] != b',' {
                return None;
            }
            i += 1;
        }
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_bad_bytes() {
        let text = "Pénélope\n";
        let hex = encode_hex(text);
        assert_eq!(decode_hex(&hex).as_deref(), Some(text));
        assert_eq!(decode_hex("70 65"), Some("pe".into()));
        assert!(decode_hex("zz").is_none());
        assert!(decode_hex("f").is_none());
        assert!(decode_hex("ff").is_none());
    }

    #[test]
    fn enum_and_set_lists_come_from_the_type_string() {
        assert_eq!(
            choices("enum('a','b','it''s')"),
            Some(Choices::One(vec!["a".into(), "b".into(), "it's".into()]))
        );
        assert_eq!(
            choices("set('red','blue')"),
            Some(Choices::Many(vec!["red".into(), "blue".into()]))
        );
        assert!(choices("varchar(20)").is_none());
        assert!(choices("enum").is_none());
        assert!(choices("ENUM('x', y)").is_none());
    }

    #[test]
    fn json_pretty_print_and_image_magic() {
        assert_eq!(
            pretty_json("{\"a\":1}").as_deref(),
            Some("{\n  \"a\": 1\n}")
        );
        assert!(pretty_json("not json").is_none());
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
