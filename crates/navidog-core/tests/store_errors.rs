//! The store owns its translated diagnostics independently of connection UI initialization.

use navidog_core::crypto::CryptoError;
use navidog_core::store::StoreError;

#[test]
fn store_errors_have_english_and_chinese_messages() {
    let previous_locale = rust_i18n::locale().to_string();
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
        rust_i18n::set_locale("en");
        let english_message = error.localized_message();
        assert!(
            english_message.starts_with(english),
            "{error:?} in en: expected prefix {english:?}, got {english_message:?}"
        );
        let diagnostic = error.to_string();
        rust_i18n::set_locale("zh-CN");
        let chinese_message = error.localized_message();
        assert!(
            chinese_message.starts_with(chinese),
            "{error:?} in zh-CN: expected prefix {chinese:?}, got {chinese_message:?}"
        );
        assert_eq!(
            error.to_string(),
            diagnostic,
            "logs do not change with locale"
        );
    }
    rust_i18n::set_locale(&previous_locale);
}
