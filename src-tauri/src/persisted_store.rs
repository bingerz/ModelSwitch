use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;
use tokio::sync::RwLock;
use serde::{Serialize, de::DeserializeOwned};

/// Generic key-value store with JSON persistence.
/// Wraps a `RwLock<HashMap<K, V>>` with save/load helpers.
#[derive(Debug)]
pub struct PersistedStore<K, V>
where
    K: Hash + Eq + Clone + Serialize + DeserializeOwned,
    V: Clone + Serialize + DeserializeOwned,
{
    data: RwLock<HashMap<K, V>>,
    store_path: PathBuf,
}

impl<K, V> PersistedStore<K, V>
where
    K: Hash + Eq + Clone + Serialize + DeserializeOwned,
    V: Clone + Serialize + DeserializeOwned,
{
    pub fn new(store_path: PathBuf) -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            store_path,
        }
    }

    /// Get a read lock on the inner HashMap.
    /// Callers can use this for all read operations.
    pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, HashMap<K, V>> {
        self.data.read().await
    }

    /// Get a write lock on the inner HashMap.
    /// Callers can use this for all write operations.
    pub async fn write(&self) -> tokio::sync::RwLockWriteGuard<'_, HashMap<K, V>> {
        self.data.write().await
    }

    /// Persist all data to disk as JSON. Best-effort — errors are logged.
    pub async fn persist(&self) {
        let data = self.data.read().await;
        let json = match serde_json::to_string(&*data) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to serialize store: {e}");
                return;
            }
        };
        drop(data);

        if let Some(parent) = self.store_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                tracing::error!("Failed to create store dir: {e}");
                return;
            }
        }
        #[cfg(unix)]
        {
            use tokio::io::AsyncWriteExt;
            match tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.store_path)
                .await
            {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(json.as_bytes()).await {
                        tracing::error!("Failed to write store: {e}");
                    }
                }
                Err(e) => tracing::error!("Failed to create store file: {e}"),
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = tokio::fs::write(&self.store_path, &json).await {
                tracing::error!("Failed to write store: {e}");
            }
        }
    }

    /// Load data from disk. Merges into existing entries (does NOT overwrite).
    pub async fn load(&self) {
        let json = match tokio::fs::read_to_string(&self.store_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!("Failed to read store file: {e}");
                return;
            }
        };

        let loaded: HashMap<K, V> = match serde_json::from_str(&json) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Failed to parse store: {e}");
                return;
            }
        };

        let mut data = self.data.write().await;
        for (k, v) in loaded {
            data.entry(k).or_insert(v);
        }
        tracing::info!(count = data.len(), "Loaded persisted data from disk");
    }

    /// Synchronous persist for shutdown path.
    /// Uses `try_read()` with retries to handle lock contention.
    pub fn persist_sync(&self) {
        let json = {
            let mut guard = None;
            for _ in 0..10 {
                match self.data.try_read() {
                    Ok(g) => {
                        guard = Some(g);
                        break;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
            match guard {
                Some(g) => match serde_json::to_string(&*g) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Failed to serialize store (sync): {e}");
                        return;
                    }
                },
                None => {
                    tracing::warn!("Could not acquire read lock for sync persist");
                    return;
                }
            }
        };

        if let Some(parent) = self.store_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.store_path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    if let Err(e) = f.write_all(json.as_bytes()) {
                        tracing::error!("Failed to write store (sync): {e}");
                        return;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create store file (sync): {e}");
                    return;
                }
            }
            tracing::info!("Store persisted on shutdown");
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = std::fs::write(&self.store_path, &json) {
                tracing::error!("Failed to write store (sync): {e}");
            } else {
                tracing::info!("Store persisted on shutdown");
            }
        }
    }

    /// Return the store path (for logging/debugging).
    pub fn path(&self) -> &std::path::Path {
        &self.store_path
    }
}
