//! Downstream callers can keep using the original formatter while migrating to core.

#[allow(deprecated)]
use navidog_connection::store_error_message;
use navidog_core::crypto::CryptoError;
use navidog_core::store::StoreError;

#[test]
#[allow(deprecated)]
fn legacy_formatter_keeps_its_signature_and_localized_messages() {
    let previous_locale = rust_i18n::locale().to_string();
    let formatter: fn(&StoreError) -> String = store_error_message;
    let cases = [
        (StoreError::Locked, "Navidog is locked.", "需要先解锁："),
        (
            StoreError::NotFound,
            "The connection or group could not be found.",
            "找不到这个连接或组。",
        ),
        (
            StoreError::Io(std::io::Error::other("fixture")),
            "Connection settings could not be saved: fixture",
            "无法保存连接设置：fixture",
        ),
        (
            StoreError::Crypto(CryptoError::Decrypt),
            "A saved password cannot be decrypted",
            "有保存的密码无法用当前的密钥解开。",
        ),
        (
            StoreError::Crypto(CryptoError::Format),
            "A saved password cannot be decrypted",
            "有保存的密码无法用当前的密钥解开。",
        ),
        (
            StoreError::Crypto(CryptoError::Keychain("fixture".into())),
            "The machine key cannot be read: fixture",
            "无法读取本机密钥：fixture",
        ),
        (
            StoreError::Crypto(CryptoError::Io("fixture".into())),
            "The machine key cannot be read: fixture",
            "无法读取本机密钥：fixture",
        ),
        (
            StoreError::Crypto(CryptoError::Kdf("fixture".into())),
            "The master password key could not be derived: fixture",
            "主密码派生失败：fixture",
        ),
    ];
    for (error, english, chinese) in cases {
        for (locale, expected) in [("en", english), ("zh-CN", chinese)] {
            rust_i18n::set_locale(locale);
            let message = formatter(&error);
            assert_eq!(message, error.localized_message());
            assert_eq!(message, navidog_core::store::error_message(&error));
            assert!(
                message.starts_with(expected),
                "{error:?} in {locale}: expected prefix {expected:?}, got {message:?}"
            );
        }
    }
    rust_i18n::set_locale(&previous_locale);
}
