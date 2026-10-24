//! Query-owned recovery guidance around the original driver diagnostic.

use navidog_db::DbError;
use rust_i18n::t;

pub(crate) fn error_message(error: &DbError) -> String {
    localized_message(error, &rust_i18n::locale())
}

fn localized_message(error: &DbError, locale: &str) -> String {
    navidog_ui::db_error_text_with(
        error.code.as_deref(),
        &error.message,
        error.is_session_lost(),
        error.is_outcome_unknown(),
        |detail| t!("query.session_lost", locale = locale, detail = detail).to_string(),
        |detail| t!("query.outcome_unknown", locale = locale, detail = detail).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_recovery_copy_preserves_diagnostics_and_uses_both_query_locales() {
        let mut error = DbError::coded("08006", "original query diagnostic");
        let diagnostic = error.to_string();
        for locale in ["en", "zh-CN"] {
            assert_eq!(localized_message(&error, locale), diagnostic);
            error.disconnected = true;
            for unknown in [false, true] {
                error.outcome_unknown = unknown;
                let text = localized_message(&error, locale);
                assert!(text.starts_with(&diagnostic));
                assert!(!text.contains('\n'));
                if locale == "en" {
                    assert!(text.contains(if unknown {
                        "query execution"
                    } else {
                        "query session"
                    }));
                } else {
                    assert!(text.contains(if unknown {
                        "查询执行结果"
                    } else {
                        "查询会话"
                    }));
                }
                assert!(text.contains(if locale == "en" {
                    "Reopen the connection"
                } else {
                    "重新打开连接"
                }));
                assert_eq!(
                    text.contains(if locale == "en" {
                        "outcome may be unknown"
                    } else {
                        "执行结果可能未知"
                    }),
                    unknown,
                );
                if locale == "zh-CN" {
                    assert!(!text.contains("Reopen"));
                }
            }
            error.disconnected = false;
            error.outcome_unknown = false;
        }
        assert_eq!(error.to_string(), diagnostic);
    }
}
