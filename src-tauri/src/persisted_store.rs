use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;
use tokio::sync::RwLock;

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

// ---------------------------------------------------------------------------
// PersistenceBackend trait + FileBackend
// ---------------------------------------------------------------------------

/// Backend storage for persisted data.
/// The default implementation is file-based JSON.
/// Future implementations can use databases (SQLite, PostgreSQL) instead.
pub trait PersistenceBackend: Send + Sync {
    /// Load all key-value pairs from storage.
    fn load_all(&self) -> anyhow::Result<HashMap<String, serde_json::Value>>;

    /// Save all key-value pairs to storage.
    fn save_all(&self, data: &HashMap<String, serde_json::Value>) -> anyhow::Result<()>;
}

/// File-based JSON persistence backend (existing behavior).
pub struct FileBackend {
    path: std::path::PathBuf,
}

impl FileBackend {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl PersistenceBackend for FileBackend {
    fn load_all(&self) -> anyhow::Result<HashMap<String, serde_json::Value>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read_to_string(&self.path)?;
        if data.trim().is_empty() {
            return Ok(HashMap::new());
        }
        let map: HashMap<String, serde_json::Value> = serde_json::from_str(&data)?;
        Ok(map)
    }

    fn save_all(&self, data: &HashMap<String, serde_json::Value>) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(data)?;

        // Atomic write: write to temp file, then rename
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &self.path)?;

        // Set file permissions to 0600 (owner read/write only) for security
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_backend_save_and_load_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modelswitch_test_backend_roundtrip_{}.json",
            uuid::Uuid::new_v4()
        ));

        let backend = FileBackend::new(&path);

        let mut data = HashMap::new();
        data.insert("key1".to_string(), json!("value1"));
        data.insert("key2".to_string(), json!({"nested": 42}));
        data.insert("key3".to_string(), json!([1, 2, 3]));

        backend.save_all(&data).expect("save should succeed");

        let loaded = backend.load_all().expect("load should succeed");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded["key1"], json!("value1"));
        assert_eq!(loaded["key2"], json!({"nested": 42}));
        assert_eq!(loaded["key3"], json!([1, 2, 3]));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_backend_load_non_existent_returns_empty() {
        let path = std::env::temp_dir().join("modelswitch_test_nonexistent_definitely.json");

        // Ensure file does not exist
        let _ = std::fs::remove_file(&path);

        let backend = FileBackend::new(&path);
        let loaded = backend
            .load_all()
            .expect("should not error on missing file");
        assert!(
            loaded.is_empty(),
            "non-existent file should return empty map"
        );
    }

    #[test]
    fn file_backend_load_empty_file_returns_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modelswitch_test_backend_empty_{}.json",
            uuid::Uuid::new_v4()
        ));

        // Write an empty file
        std::fs::write(&path, "").expect("should write empty file");

        let backend = FileBackend::new(&path);
        let loaded = backend.load_all().expect("should not error on empty file");
        assert!(
            loaded.is_empty(),
            "empty file should return empty map, not error"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_backend_load_whitespace_file_returns_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modelswitch_test_backend_whitespace_{}.json",
            uuid::Uuid::new_v4()
        ));

        // Write a whitespace-only file
        std::fs::write(&path, "   \n\n  ").expect("should write whitespace file");

        let backend = FileBackend::new(&path);
        let loaded = backend
            .load_all()
            .expect("should not error on whitespace file");
        assert!(
            loaded.is_empty(),
            "whitespace-only file should return empty map"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_backend_atomic_write_cleans_up_temp() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modelswitch_test_backend_atomic_{}.json",
            uuid::Uuid::new_v4()
        ));
        let tmp_path = path.with_extension("tmp");

        let backend = FileBackend::new(&path);
        let mut data = HashMap::new();
        data.insert("key".to_string(), json!("value"));

        backend.save_all(&data).expect("save should succeed");

        // Temp file should not exist after atomic rename
        assert!(
            !tmp_path.exists(),
            "temp file should be cleaned up after rename"
        );
        // Main file should exist with correct content
        assert!(path.exists(), "main file should exist after save");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_backend_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "modelswitch_test_backend_subdir_{}",
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("nested/deep/store.json");

        let backend = FileBackend::new(&path);
        let mut data = HashMap::new();
        data.insert("key".to_string(), json!("value"));

        backend
            .save_all(&data)
            .expect("save should create parent dirs");

        assert!(path.exists(), "file should exist with created parent dirs");

        let loaded = backend.load_all().expect("load should succeed");
        assert_eq!(loaded["key"], json!("value"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_backend_overwrites_existing_data() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modelswitch_test_backend_overwrite_{}.json",
            uuid::Uuid::new_v4()
        ));

        let backend = FileBackend::new(&path);

        // Save initial data
        let mut data1 = HashMap::new();
        data1.insert("key1".to_string(), json!("value1"));
        backend.save_all(&data1).expect("first save should succeed");

        // Overwrite with different data
        let mut data2 = HashMap::new();
        data2.insert("key2".to_string(), json!("value2"));
        backend
            .save_all(&data2)
            .expect("second save should succeed");

        let loaded = backend.load_all().expect("load should succeed");
        assert_eq!(loaded.len(), 1, "old data should be replaced");
        assert_eq!(loaded["key2"], json!("value2"));
        assert!(
            !loaded.contains_key("key1"),
            "old key should not exist after overwrite"
        );

        let _ = std::fs::remove_file(&path);
    }
}
