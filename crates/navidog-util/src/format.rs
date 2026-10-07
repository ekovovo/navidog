//! Text conversions that do not belong to a single feature.

/// Bytes of `text` as lowercase hex pairs separated by spaces.
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
    fn json_pretty_print() {
        assert_eq!(
            pretty_json("{\"a\":1}").as_deref(),
            Some("{\n  \"a\": 1\n}")
        );
        assert!(pretty_json("not json").is_none());
    }
}
