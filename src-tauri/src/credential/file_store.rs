use super::CredentialStore;
use crate::config::app_config_dir;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::RwLock;

/// Credential store backed by a TOML file at `{config_dir}/modelswitch/credentials.toml`.
/// File permissions are set to 0600 (owner read/write only).
pub struct FileCredentialStore {
    path: std::path::PathBuf,
    cache: RwLock<HashMap<String, String>>,
}

impl Default for FileCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileCredentialStore {
    pub fn new() -> Self {
        let path = app_config_dir().join("credentials.toml");

        let store = Self {
            path,
            cache: RwLock::new(HashMap::new()),
        };

        // Load existing credentials on startup
        if let Ok(content) = fs::read_to_string(&store.path) {
            if let Ok(data) = toml::from_str::<toml::Value>(&content) {
                if let Some(table) = data.as_table() {
                    let mut cache = store.cache.write().unwrap_or_else(|e| e.into_inner());
                    for (k, v) in table {
                        if let Some(s) = v.as_str() {
                            cache.insert(k.to_string(), s.to_string());
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
            table.insert(k.clone(), toml::Value::String(v.clone()));
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

    fn credentials_path() -> std::path::PathBuf {
        crate::config::app_config_dir().join("credentials.toml")
    }

    fn cleanup_file() {
        let _ = std::fs::remove_file(credentials_path());
    }

    #[test]
    fn file_store_set_and_get() {
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
        cleanup_file();
        let store = FileCredentialStore::new();
        store.set("svc_empty_user_test", "", "pass").unwrap();
        let result = store.get("svc_empty_user_test", "").unwrap();
        assert_eq!(result.as_deref(), Some("pass"));
        let _ = store.delete("svc_empty_user_test", "");
        cleanup_file();
    }
}
