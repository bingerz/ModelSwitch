//! AES-256-GCM encryption for credential values at rest.
//!
//! Values are encrypted with a key derived from the admin token. Encrypted
//! values are prefixed with `enc:v1:` followed by base64(nonce || ciphertext).
//! Plaintext values (no prefix) are loaded as-is for backward compatibility.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{bail, Context, Result};

const PREFIX: &str = "enc:v1:";

/// Derive a 32-byte AES key from a secret string (typically admin_token).
pub fn derive_key(secret: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"modelswitch-credential-encryption-v1");
    hasher.update(secret.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypt a plaintext string. Returns `enc:v1:{base64(nonce + ciphertext)}`.
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
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
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(&combined)
    ))
}

/// Decrypt an `enc:v1:` prefixed value back to plaintext.
/// Returns an error if the value is not encrypted or decryption fails.
pub fn decrypt(key: &[u8; 32], encoded: &str) -> Result<String> {
    let payload = encoded
        .strip_prefix(PREFIX)
        .context("value is not encrypted (missing enc:v1: prefix)")?;

    use base64::Engine;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .context("invalid base64 in encrypted value")?;

    if combined.len() < 12 {
        bail!("encrypted value too short (missing nonce)");
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("AES-GCM decryption failed: {e}"))?;

    String::from_utf8(plaintext).context("decrypted value is not valid UTF-8")
}

/// Check if a value is encrypted (has the `enc:v1:` prefix).
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = derive_key("my-secret-admin-token");
        let plaintext = "sk-abc123-secret-key";
        let encrypted = encrypt(&key, plaintext).unwrap();
        assert!(is_encrypted(&encrypted));
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_encryptions_produce_different_ciphertext() {
        let key = derive_key("secret");
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
        let key1 = derive_key("correct-secret");
        let key2 = derive_key("wrong-secret");
        let encrypted = encrypt(&key1, "sensitive").unwrap();
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn decrypt_plaintext_returns_error() {
        let key = derive_key("secret");
        assert!(decrypt(&key, "not-encrypted").is_err());
    }

    #[test]
    fn is_encrypted_detects_prefix() {
        assert!(is_encrypted("enc:v1:abc123"));
        assert!(!is_encrypted("sk-abc123"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn different_secrets_derive_different_keys() {
        let k1 = derive_key("secret1");
        let k2 = derive_key("secret2");
        assert_ne!(k1, k2);
    }
}
