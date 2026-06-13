use crate::config::app_config_dir;
use super::CredentialStore;
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
                    let mut cache = store.cache.write().unwrap();
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
        let cache = self.cache.read().unwrap();
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
        let cache = self.cache.read().unwrap();
        let key = Self::key(service, username);
        Ok(cache.get(&key).cloned())
    }

    fn set(&self, service: &str, username: &str, password: &str) -> Result<()> {
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(Self::key(service, username), password.to_string());
        }
        self.persist()
            .with_context(|| "failed to persist credentials file")
    }

    fn delete(&self, service: &str, username: &str) -> Result<()> {
        {
            let mut cache = self.cache.write().unwrap();
            cache.remove(&Self::key(service, username));
        }
        self.persist()
            .with_context(|| "failed to persist credentials file after delete")
    }
}
