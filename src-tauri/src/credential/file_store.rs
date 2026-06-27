use super::CredentialStore;
use crate::config::app_config_dir;
use crate::credential::crypto;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::RwLock;

/// Credential store backed by a TOML file at `{config_dir}/modelswitch/credentials.toml`.
/// File permissions are set to 0600 (owner read/write only).
///
/// When constructed with [`Self::with_encryption`], values are encrypted with
/// AES-256-GCM before being written to disk. The in-memory cache always holds
/// plaintext — encryption only applies to the on-disk file.
pub struct FileCredentialStore {
    path: std::path::PathBuf,
    cache: RwLock<HashMap<String, String>>,
    encryption_key: Option<[u8; 32]>,
}

impl Default for FileCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileCredentialStore {
    /// Create a store without encryption (legacy/plaintext mode).
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Create a store that encrypts values at rest using a key derived from
    /// `admin_token`. Existing plaintext credentials on disk are loaded
    /// as-is and re-encrypted on the next persist.
    pub fn with_encryption(admin_token: &str) -> Self {
        Self::build(Some(crypto::derive_key(admin_token)))
    }

    fn build(encryption_key: Option<[u8; 32]>) -> Self {
        let path = app_config_dir().join("credentials.toml");

        let store = Self {
            path,
            cache: RwLock::new(HashMap::new()),
            encryption_key,
        };

        // Load existing credentials on startup
        if let Ok(content) = fs::read_to_string(&store.path) {
            if let Ok(data) = toml::from_str::<toml::Value>(&content) {
                if let Some(table) = data.as_table() {
                    let mut cache = store.cache.write().unwrap_or_else(|e| e.into_inner());
                    for (k, v) in table {
                        if let Some(s) = v.as_str() {
                            // Decrypt if encrypted; plaintext values load as-is
                            let value = if crypto::is_encrypted(s) {
                                if let Some(key) = &store.encryption_key {
                                    match crypto::decrypt(key, s) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            tracing::error!(
                                                key = %k,
                                                error = %e,
                                                "failed to decrypt credential — skipping entry"
                                            );
                                            continue;
                                        }
                                    }
                                } else {
                                    tracing::warn!(
                                        key = %k,
                                        "encrypted credential found but no encryption key configured — skipping"
                                    );
                                    continue;
                                }
                            } else {
                                // Plaintext value — backward compatible
                                if store.encryption_key.is_some() {
                                    tracing::info!(
                                        key = %k,
                                        "migrating plaintext credential to encrypted at next persist"
                                    );
                                }
                                s.to_string()
                            };
                            cache.insert(k.to_string(), value);
                        }
                    }
                }
            }
        }

        store
    }

    fn key(service: &str, username: &str) -> String {
        format!("{service}:{username}")
    }

    fn persist(&self) -> Result<()> {
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let mut table = toml::map::Map::new();
        for (k, v) in cache.iter() {
            let stored_value = if let Some(key) = &self.encryption_key {
                crypto::encrypt(key, v)
                    .with_context(|| format!("encryption failed for credential key '{k}'"))?
            } else {
                v.clone()
            };
            table.insert(k.clone(), toml::Value::String(stored_value));
        }
        drop(cache);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(&table)?;
        fs::write(&self.path, &content)?;

        // Set file permissions to 0600 (owner read/write only)
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&self.path, perms)?;

        Ok(())
    }
}

impl CredentialStore for FileCredentialStore {
    fn get(&self, service: &str, username: &str) -> Result<Option<String>> {
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        let key = Self::key(service, username);
        Ok(cache.get(&key).cloned())
    }

    fn set(&self, service: &str, username: &str, password: &str) -> Result<()> {
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            cache.insert(Self::key(service, username), password.to_string());
        }
        self.persist()
            .with_context(|| "failed to persist credentials file")
    }

    fn delete(&self, service: &str, username: &str) -> Result<()> {
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            cache.remove(&Self::key(service, username));
        }
        self.persist()
            .with_context(|| "failed to persist credentials file after delete")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serialise all file_store tests — they share a single credentials.toml file.
    static FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn lock() -> &'static Mutex<()> {
        FILE_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn credentials_path() -> std::path::PathBuf {
        crate::config::app_config_dir().join("credentials.toml")
    }

    fn cleanup_file() {
        let _ = std::fs::remove_file(credentials_path());
    }

    #[test]
    fn file_store_set_and_get() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("test_service", "user1", "secret123").unwrap();
        let result = store.get("test_service", "user1").unwrap();
        assert_eq!(result.as_deref(), Some("secret123"));
        let _ = store.delete("test_service", "user1");
        cleanup_file();
    }

    #[test]
    fn file_store_get_returns_none_for_missing() {
        let store = FileCredentialStore::new();
        let result = store.get("nonexistent_service", "nobody").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn file_store_delete_removes_credential() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("svc_del_test", "user_del", "pass_del").unwrap();
        assert_eq!(
            store.get("svc_del_test", "user_del").unwrap().as_deref(),
            Some("pass_del")
        );
        store.delete("svc_del_test", "user_del").unwrap();
        assert_eq!(store.get("svc_del_test", "user_del").unwrap(), None);
        cleanup_file();
    }

    #[test]
    fn file_store_overwrite_on_set() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("svc_overwrite_test", "user", "old").unwrap();
        store.set("svc_overwrite_test", "user", "new").unwrap();
        let result = store.get("svc_overwrite_test", "user").unwrap();
        assert_eq!(result.as_deref(), Some("new"));
        let _ = store.delete("svc_overwrite_test", "user");
        cleanup_file();
    }

    #[test]
    fn file_store_key_isolation() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("svc1_iso_test", "user", "a").unwrap();
        store.set("svc2_iso_test", "user", "b").unwrap();
        assert_eq!(
            store.get("svc1_iso_test", "user").unwrap().as_deref(),
            Some("a")
        );
        assert_eq!(
            store.get("svc2_iso_test", "user").unwrap().as_deref(),
            Some("b")
        );
        let _ = store.delete("svc1_iso_test", "user");
        let _ = store.delete("svc2_iso_test", "user");
        cleanup_file();
    }

    #[test]
    fn file_store_empty_username() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("svc_empty_user_test", "", "pass").unwrap();
        let result = store.get("svc_empty_user_test", "").unwrap();
        assert_eq!(result.as_deref(), Some("pass"));
        let _ = store.delete("svc_empty_user_test", "");
        cleanup_file();
    }

    #[test]
    fn encrypted_store_roundtrip() {
        let _guard = lock().lock().unwrap();
        cleanup_file();
        let store = FileCredentialStore::with_encryption("test-admin-token");
        store
            .set("svc_enc_test", "user", "super-secret-key")
            .unwrap();

        // Verify the on-disk file contains encrypted values
        let content = std::fs::read_to_string(credentials_path()).unwrap();
        assert!(
            content.contains("enc:v1:"),
            "on-disk file should contain encrypted values"
        );
        assert!(
            !content.contains("super-secret-key"),
            "plaintext should NOT appear on disk"
        );

        // Verify get() returns plaintext
        let result = store.get("svc_enc_test", "user").unwrap();
        assert_eq!(result.as_deref(), Some("super-secret-key"));

        let _ = store.delete("svc_enc_test", "user");
        cleanup_file();
    }

    #[test]
    fn encrypted_store_loads_plaintext_and_migrates() {
        let _guard = lock().lock().unwrap();
        cleanup_file();

        // Write plaintext credentials first (quote the key — colons require
        // quoting in TOML and the persist() path emits quoted keys).
        std::fs::write(
            credentials_path(),
            "\"plaintext_svc:user\" = \"plaintext-key\"\n",
        )
        .unwrap();

        // Load with encryption — plaintext values should be readable
        let store = FileCredentialStore::with_encryption("test-admin-token");
        let result = store.get("plaintext_svc", "user").unwrap();
        assert_eq!(result.as_deref(), Some("plaintext-key"));

        // After persist, the value should be encrypted on disk
        store.set("new_svc", "new_user", "new-secret").unwrap();
        let content = std::fs::read_to_string(credentials_path()).unwrap();
        assert!(
            content.contains("enc:v1:"),
            "migrated file should have encrypted values"
        );
        assert!(
            !content.contains("plaintext-key"),
            "old plaintext should be encrypted now"
        );

        cleanup_file();
    }
}
