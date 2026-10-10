//! Pure helpers shared across crates. No UI and no database drivers.

pub mod format;
pub mod sql_type;

/// `s` starts with `prefix`, ignoring ASCII case. Compares bytes, so a multi-byte character at
/// the cut point cannot panic the way `s[..prefix.len()]` does.
pub fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.as_bytes()
        .get(..prefix.len())
        .is_some_and(|b| b.eq_ignore_ascii_case(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_check_survives_multibyte_text() {
        assert!(starts_with_ignore_ascii_case("or x", "OR"));
        assert!(!starts_with_ignore_ascii_case("名称 = 1", "OR"));
        assert!(!starts_with_ignore_ascii_case("o", "OR"));
    }
}
