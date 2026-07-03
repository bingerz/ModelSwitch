//! AES-256-GCM encryption for credential values at rest.
//!
//! Values are encrypted with a key derived from the admin token. Encrypted
//! values are prefixed with `enc:v2:` (current) or `enc:v1:` (legacy) followed
//! by base64(nonce || ciphertext). Plaintext values (no prefix) are loaded
//! as-is for backward compatibility.
//!
//! v2 uses HKDF-SHA256 for key derivation; v1 used a single SHA-256 hash.
//! Both keys are held so legacy `enc:v1:` values on disk can still be decrypted
//! and migrated to v2 on the next persist.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{bail, Context, Result};

const PREFIX_V1: &str = "enc:v1:";
const PREFIX_V2: &str = "enc:v2:";

/// Holds both v1 (legacy single SHA-256) and v2 (HKDF) keys so that
/// `decrypt` can read old on-disk values while `encrypt` writes v2.
pub struct EncryptionKey {
    v1: [u8; 32],
    v2: [u8; 32],
}

impl EncryptionKey {
    /// Derive both v1 and v2 keys from a secret (typically admin_token).
    pub fn derive(secret: &str) -> Self {
        Self {
            v1: derive_key_v1(secret),
            v2: derive_key_v2(secret),
        }
    }
}

/// Legacy v1 key derivation — single SHA-256 hash. Kept only so old
/// `enc:v1:` values on disk can be decrypted and migrated.
fn derive_key_v1(secret: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"modelswitch-credential-encryption-v1");
    hasher.update(secret.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// v2 key derivation — HKDF-SHA256. The admin token is already high-entropy,
/// so HKDF's single-pass extract+expand is the correct KDF (not PBKDF2).
fn derive_key_v2(secret: &str) -> [u8; 32] {
    use sha2::Sha256;
    use hkdf::Hkdf;

    let salt = b"modelswitch-credential-encryption-v2";
    let info = b"aes-256-gcm-key";
    let hk = Hkdf::<Sha256>::new(Some(salt), secret.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("32 bytes is valid length");
    okm
}

/// Encrypt a plaintext string. Returns `enc:v2:{base64(nonce + ciphertext)}`.
pub fn encrypt(key: &EncryptionKey, plaintext: &str) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.v2));
    // 96-bit nonce — GCM standard. Random per encryption.
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("AES-GCM encryption failed: {e}"))?;

    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    use base64::Engine;
    Ok(format!(
        "{PREFIX_V2}{}",
        base64::engine::general_purpose::STANDARD.encode(&combined)
    ))
}

/// Decrypt an `enc:v1:` or `enc:v2:` prefixed value back to plaintext.
/// Returns an error if the value is not encrypted or decryption fails.
pub fn decrypt(key: &EncryptionKey, encoded: &str) -> Result<String> {
    let (aes_key, payload) = if let Some(rest) = encoded.strip_prefix(PREFIX_V2) {
        (&key.v2, rest)
    } else if let Some(rest) = encoded.strip_prefix(PREFIX_V1) {
        (&key.v1, rest)
    } else {
        bail!("value is not encrypted (missing enc:v1:/enc:v2: prefix)");
    };

    use base64::Engine;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .context("invalid base64 in encrypted value")?;

    if combined.len() < 12 {
        bail!("encrypted value too short (missing nonce)");
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aes_key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {e}"))?;

    String::from_utf8(plaintext).context("decrypted value is not valid UTF-8")
}

/// Check if a value is encrypted (has an `enc:v1:` or `enc:v2:` prefix).
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX_V1) || value.starts_with(PREFIX_V2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = EncryptionKey::derive("my-secret-admin-token");
        let plaintext = "sk-abc123-secret-key";
        let encrypted = encrypt(&key, plaintext).unwrap();
        assert!(is_encrypted(&encrypted));
        assert!(encrypted.starts_with("enc:v2:"));
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_encryptions_produce_different_ciphertext() {
        let key = EncryptionKey::derive("secret");
        let plaintext = "same-value";
        let e1 = encrypt(&key, plaintext).unwrap();
        let e2 = encrypt(&key, plaintext).unwrap();
        // Random nonce ensures different ciphertext each time
        assert_ne!(e1, e2);
        // Both decrypt to the same plaintext
        assert_eq!(decrypt(&key, &e1).unwrap(), plaintext);
        assert_eq!(decrypt(&key, &e2).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let key1 = EncryptionKey::derive("correct-secret");
        let key2 = EncryptionKey::derive("wrong-secret");
        let encrypted = encrypt(&key1, "sensitive").unwrap();
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn decrypt_plaintext_returns_error() {
        let key = EncryptionKey::derive("secret");
        assert!(decrypt(&key, "not-encrypted").is_err());
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("enc:v1:abc123"));
        assert!(is_encrypted("enc:v2:abc123"));
        assert!(!is_encrypted("sk-abc123"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn different_secrets_derive_different_keys() {
        let k1 = EncryptionKey::derive("secret1");
        let k2 = EncryptionKey::derive("secret2");
        assert_ne!(k1.v2, k2.v2);
    }

    #[test]
    fn decrypt_legacy_v1_value() {
        // A v1-encrypted value uses the old single-SHA-256 key.
        // Simulate by deriving a v1 key and encrypting with v1 prefix.
        let secret = "legacy-admin-token";
        let v1_key = derive_key_v1(secret);

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&v1_key));
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = "legacy-secret-value";
        let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes()).unwrap();

        let mut combined = Vec::with_capacity(12 + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        use base64::Engine;
        let encoded = format!(
            "{PREFIX_V1}{}",
            base64::engine::general_purpose::STANDARD.encode(&combined)
        );

        // The modern key (with both v1 and v2) should decrypt the v1 value.
        let key = EncryptionKey::derive(secret);
        assert_eq!(decrypt(&key, &encoded).unwrap(), plaintext);
    }

    #[test]
    fn v1_and_v2_keys_differ() {
        // The same secret must produce different v1 and v2 keys — otherwise
        // the KDF upgrade is meaningless.
        let key = EncryptionKey::derive("some-secret");
        assert_ne!(key.v1, key.v2);
    }
}
