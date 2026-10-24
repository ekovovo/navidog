//! Localized recovery guidance around unmodified database diagnostics.

use navidog_db::DbError;
use rust_i18n::t;

pub(crate) fn database_message(error: &DbError) -> String {
    message(error, &rust_i18n::locale())
}

fn message(error: &DbError, locale: &str) -> String {
    navidog_ui::db_error_text_with(
        error.code.as_deref(),
        &error.message,
        error.is_session_lost(),
        error.is_outcome_unknown(),
        |detail| {
            t!(
                "nav.database_session_lost",
                locale = locale,
                detail = detail
            )
            .to_string()
        },
        |detail| {
            t!(
                "nav.database_outcome_unknown",
                locale = locale,
                detail = detail
            )
            .to_string()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_guidance_is_localized_without_changing_the_driver_diagnostic() {
        let mut error = DbError::coded("08006", "original driver diagnostic");
        let detail = error.to_string();
        for locale in ["en", "zh-CN"] {
            assert_eq!(message(&error, locale), detail);
            error.disconnected = true;
            for unknown in [false, true] {
                error.outcome_unknown = unknown;
                let text = message(&error, locale);
                assert!(text.starts_with(&detail));
                assert!(!text.contains('\n'));
                assert!(text.contains(if locale == "en" {
                    "Reopen"
                } else {
                    "重新打开"
                }));
                if unknown {
                    assert!(text.contains(if locale == "en" {
                        "outcome may be unknown"
                    } else {
                        "执行结果可能未知"
                    }));
                }
                if locale == "zh-CN" {
                    assert!(!text.contains("Reopen"));
                }
            }
            error.disconnected = false;
            error.outcome_unknown = false;
        }
        assert_eq!(error.to_string(), detail);
    }
}
