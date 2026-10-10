//! Secret-field encryption (11-connection.md §7): XChaCha20-Poly1305.
//! Associated data is "connection id + field name", so a ciphertext cannot be
//! swapped onto another field.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload, rand_core::RngCore};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "ci.nn.navidog";
const KEYCHAIN_ACCOUNT: &str = "config-key";
const CHECK_PLAINTEXT: &[u8] = b"navidog-master-check";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub nonce: String,
    pub ct: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("wrong key or corrupted data")]
    Decrypt,
    #[error("invalid ciphertext format")]
    Format,
    #[error("cannot access the keychain: {0}")]
    Keychain(String),
    #[error("master password derivation failed: {0}")]
    Kdf(String),
}

/// 32-byte symmetric key.
#[derive(Clone)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

impl Key {
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }

    pub fn encrypt(&self, aad: &str, plaintext: &str) -> Secret {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = self
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .expect("XChaCha20-Poly1305 encryption does not fail");
        Secret {
            nonce: B64.encode(nonce),
            ct: B64.encode(ct),
        }
    }

    pub fn decrypt(&self, aad: &str, secret: &Secret) -> Result<String, CryptoError> {
        let nonce = B64.decode(&secret.nonce).map_err(|_| CryptoError::Format)?;
        let ct = B64.decode(&secret.ct).map_err(|_| CryptoError::Format)?;
        if nonce.len() != 24 {
            return Err(CryptoError::Format);
        }
        let plain = self
            .cipher()
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ct,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Decrypt)?;
        String::from_utf8(plain).map_err(|_| CryptoError::Format)
    }
}

/// How secrets are keyed. Serialized as the `crypto` field of connections.json.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CryptoMode {
    /// Random per-machine key stored in the keychain.
    #[default]
    Machine,
    /// Key derived from a master password with Argon2id.
    Master {
        salt: String,
        m_kib: u32,
        t: u32,
        p: u32,
        check: Secret,
    },
}

/// OWASP Argon2id parameters: 19 MiB, 2 iterations, 1 lane.
pub const ARGON2_M_KIB: u32 = 19 * 1024;
pub const ARGON2_T: u32 = 2;
pub const ARGON2_P: u32 = 1;

pub fn derive_master_key(
    password: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<Key, CryptoError> {
    let params =
        argon2::Params::new(m_kib, t, p, Some(32)).map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(Key(out))
}

/// Builds a master-password mode and the key it derives.
pub fn new_master(password: &str) -> Result<(CryptoMode, Key), CryptoError> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let key = derive_master_key(password, &salt, ARGON2_M_KIB, ARGON2_T, ARGON2_P)?;
    let check = key.encrypt("check", std::str::from_utf8(CHECK_PLAINTEXT).unwrap());
    let mode = CryptoMode::Master {
        salt: B64.encode(salt),
        m_kib: ARGON2_M_KIB,
        t: ARGON2_T,
        p: ARGON2_P,
        check,
    };
    Ok((mode, key))
}

/// Unlocks a master-password mode. A wrong password is `CryptoError::Decrypt`.
pub fn unlock_master(mode: &CryptoMode, password: &str) -> Result<Key, CryptoError> {
    let CryptoMode::Master {
        salt,
        m_kib,
        t,
        p,
        check,
    } = mode
    else {
        return Err(CryptoError::Format);
    };
    let salt = B64.decode(salt).map_err(|_| CryptoError::Format)?;
    let key = derive_master_key(password, &salt, *m_kib, *t, *p)?;
    let plain = key.decrypt("check", check)?;
    if plain.as_bytes() != CHECK_PLAINTEXT {
        return Err(CryptoError::Decrypt);
    }
    Ok(key)
}

/// Reads the machine key from the OS credential store, creating it on first use.
///
/// On macOS one read can still show two keychain dialogs. Asking only for the
/// password (and not the keychain item) was tried and did not stop the second
/// dialog, so this stays on `keyring`. See `docs/design/12-secrets.md`.
pub fn machine_key() -> Result<Key, CryptoError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| CryptoError::Keychain(e.to_string()))?;
    match entry.get_password() {
        Ok(encoded) => {
            let bytes = B64.decode(encoded).map_err(|_| CryptoError::Format)?;
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| CryptoError::Format)?;
            Ok(Key(bytes))
        }
        Err(keyring::Error::NoEntry) => {
            let key = Key::random();
            entry
                .set_password(&B64.encode(key.0))
                .map_err(|e| CryptoError::Keychain(e.to_string()))?;
            Ok(key)
        }
        Err(e) => Err(CryptoError::Keychain(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_aad_binding() {
        let key = Key::random();
        let secret = key.encrypt("c-1/general.password", "s3cret");
        assert_eq!(
            key.decrypt("c-1/general.password", &secret).unwrap(),
            "s3cret"
        );
        // A different field or connection does not decrypt.
        assert!(key.decrypt("c-1/ssh.password", &secret).is_err());
        assert!(key.decrypt("c-2/general.password", &secret).is_err());
        assert!(
            Key::random()
                .decrypt("c-1/general.password", &secret)
                .is_err()
        );
    }

    #[test]
    fn master_password_unlock() {
        let (mode, key) = new_master("hunter2").unwrap();
        let secret = key.encrypt("a", "x");
        let unlocked = unlock_master(&mode, "hunter2").unwrap();
        assert_eq!(unlocked.decrypt("a", &secret).unwrap(), "x");
        assert!(matches!(
            unlock_master(&mode, "wrong"),
            Err(CryptoError::Decrypt)
        ));
        let json = serde_json::to_string(&mode).unwrap();
        assert!(json.contains("\"kind\":\"master\""));
    }
}
