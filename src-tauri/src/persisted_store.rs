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
    /// Set when a corrupt store file was detected during `load()`.
    /// When poisoned, `persist()` and `persist_sync()` refuse to write
    /// to prevent overwriting potentially recoverable data.
    poisoned: std::sync::atomic::AtomicBool,
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
            poisoned: std::sync::atomic::AtomicBool::new(false),
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
    ///
    /// A `schema_version: 1` key is injected alongside the HashMap entries so
    /// that the migration framework can detect the current version on the
    /// next startup.
    pub async fn persist(&self) {
        if self.poisoned.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::error!(
                path = %self.store_path.display(),
                "persist() refused — store is poisoned (corrupt data was detected on load). \
                 Restart the gateway after investigating the .corrupt.* backup file."
            );
            return;
        }
        let data = self.data.read().await;
        let json = match serde_json::to_string(&*data) {
            Ok(s) => inject_schema_version(&s),
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
        // Atomic write: serialize to a temp file in the same directory, fsync,
        // then rename over the real path. A crash mid-write leaves the temp
        // file (or the previous real file) in place rather than a truncated
        // store, which would cause `serde_json::from_str` to fail on the next
        // startup and silently wipe all virtual keys/budgets.
        let tmp_path = self.store_path.with_extension("json.tmp");
        #[cfg(unix)]
        {
            use tokio::io::AsyncWriteExt;
            let open_result = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp_path)
                .await;
            match open_result {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(json.as_bytes()).await {
                        tracing::error!("Failed to write store (temp): {e}");
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        return;
                    }
                    // Flush OS buffer so the bytes hit disk before rename.
                    if let Err(e) = f.sync_all().await {
                        tracing::error!("Failed to fsync store (temp): {e}");
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        return;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create store file (temp): {e}");
                    return;
                }
            }
            // Atomic on POSIX when src and dst are on the same filesystem
            // (they are — same directory).
            if let Err(e) = tokio::fs::rename(&tmp_path, &self.store_path).await {
                tracing::error!("Failed to rename temp store into place: {e}");
                let _ = tokio::fs::remove_file(&tmp_path).await;
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = tokio::fs::write(&tmp_path, &json).await {
                tracing::error!("Failed to write store (temp): {e}");
                return;
            }
            if let Err(e) = tokio::fs::rename(&tmp_path, &self.store_path).await {
                tracing::error!("Failed to rename temp store into place: {e}");
                let _ = tokio::fs::remove_file(&tmp_path).await;
            }
        }
    }

    /// Load data from disk. Merges into existing entries (does NOT overwrite).
    ///
    /// Strips a top-level `schema_version` key (added by the migration
    /// framework) before deserializing into `HashMap<K, V>` so that it does
    /// not interfere with key parsing.
    pub async fn load(&self) {
        let json = match tokio::fs::read_to_string(&self.store_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!("Failed to read store file: {e}");
                return;
            }
        };

        // Strip `schema_version` if present so it doesn't break HashMap<K, V>
        // deserialization (the key "schema_version" is not a valid K).
        let cleaned_json = strip_schema_version(&json);

        let loaded: HashMap<K, V> = match serde_json::from_str(&cleaned_json) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(
                    "CRITICAL: Failed to parse persisted store at {}: {e}. \
                     Backing up corrupt file and poisoning store to prevent overwrite.",
                    self.store_path.display()
                );
                // Back up the corrupt file so an operator can attempt manual recovery.
                let backup_path = {
                    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
                    self.store_path.with_extension(format!("json.corrupt.{ts}"))
                };
                if let Err(backup_err) = tokio::fs::rename(&self.store_path, &backup_path).await {
                    tracing::error!(
                        original = %self.store_path.display(),
                        backup = %backup_path.display(),
                        error = %backup_err,
                        "Failed to back up corrupt store file — data will be lost on next persist"
                    );
                }
                // Poison the store: persist() will refuse to write until the process restarts.
                self.poisoned
                    .store(true, std::sync::atomic::Ordering::SeqCst);
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
        if self.poisoned.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::error!(
                path = %self.store_path.display(),
                "persist_sync() refused — store is poisoned"
            );
            return;
        }
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
                    Ok(s) => inject_schema_version(&s),
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
        let tmp_path = self.store_path.with_extension("json.tmp");
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp_path)
            {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(json.as_bytes()) {
                        tracing::error!("Failed to write store (sync temp): {e}");
                        let _ = std::fs::remove_file(&tmp_path);
                        return;
                    }
                    // fsync before rename to ensure durability
                    if let Err(e) = f.sync_all() {
                        tracing::error!("Failed to fsync store (sync temp): {e}");
                        let _ = std::fs::remove_file(&tmp_path);
                        return;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create store file (sync temp): {e}");
                    return;
                }
            }
            // Atomic rename
            if let Err(e) = std::fs::rename(&tmp_path, &self.store_path) {
                tracing::error!("Failed to rename temp store into place (sync): {e}");
                let _ = std::fs::remove_file(&tmp_path);
                return;
            }
            tracing::info!("Store persisted on shutdown (atomic write)");
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = std::fs::write(&tmp_path, &json) {
                tracing::error!("Failed to write store (sync temp): {e}");
                return;
            }
            if let Err(e) = std::fs::rename(&tmp_path, &self.store_path) {
                tracing::error!("Failed to rename temp store into place (sync): {e}");
                let _ = std::fs::remove_file(&tmp_path);
                return;
            }
            tracing::info!("Store persisted on shutdown (atomic write)");
        }
    }

    /// Return the store path (for logging/debugging).
    pub fn path(&self) -> &std::path::Path {
        &self.store_path
    }
}

// ---------------------------------------------------------------------------
// Schema version helpers (used by the migration framework)
// ---------------------------------------------------------------------------

/// Current data-file schema version. Must match `migration::CURRENT_SCHEMA_VERSION`.
const DATA_SCHEMA_VERSION: u32 = 1;

/// Strip a top-level `schema_version` key from a JSON object string so that
/// it does not interfere with `HashMap<K, V>` deserialization. If the JSON is
/// not an object or does not contain the key, the original string is returned
/// unchanged.
fn strip_schema_version(json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        if obj.remove("schema_version").is_some() {
            return serde_json::to_string(&value).unwrap_or_else(|_| json.to_string());
        }
    }
    json.to_string()
}

/// Inject `schema_version: N` into a JSON object string. If the JSON is not
/// an object, the original string is returned unchanged.
fn inject_schema_version(json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "schema_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from(DATA_SCHEMA_VERSION)),
        );
        return serde_json::to_string(&value).unwrap_or_else(|_| json.to_string());
    }
    json.to_string()
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

#[cfg(test)]
mod async_tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[tokio::test]
    async fn async_persist_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-persist-test-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            w.insert(
                "key1".to_string(),
                TestData {
                    name: "test".to_string(),
                    value: 42,
                },
            );
        }
        store.persist().await;
        assert!(path.exists(), "persist file should exist");

        let store2: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        store2.load().await;
        let r = store2.read().await;
        assert_eq!(r.get("key1").unwrap().value, 42);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn async_load_corrupt_json_poisons_and_backs_up() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-corrupt-test-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&path, b"{ this is not valid json }").unwrap();

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        store.load().await; // should not panic
        let r = store.read().await;
        assert!(r.is_empty(), "corrupt JSON should result in empty map");
        drop(r);

        // Store should be poisoned
        assert!(
            store.poisoned.load(std::sync::atomic::Ordering::SeqCst),
            "store should be poisoned after corrupt load"
        );

        // Clean up backup
        let parent = path.parent().unwrap();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("modelswitch-corrupt-test-")
                    && name_str.contains(".corrupt.")
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    #[tokio::test]
    async fn load_corrupt_json_backs_up_and_poisons() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-corrupt-poison-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&path, b"{ this is not valid json }").unwrap();

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        store.load().await;

        // Store should be poisoned
        assert!(
            store.poisoned.load(std::sync::atomic::Ordering::SeqCst),
            "store should be poisoned after corrupt load"
        );

        // Persist should be refused
        {
            let mut w = store.write().await;
            w.insert(
                "k".to_string(),
                TestData {
                    name: "v".to_string(),
                    value: 1,
                },
            );
        }
        store.persist().await;

        // Original path should NOT have been overwritten (persist was refused)
        // The corrupt file was renamed to .corrupt.{timestamp}
        assert!(
            !path.exists(),
            "original path should not exist — corrupt file was backed up"
        );

        // Clean up backup file
        let parent = path.parent().unwrap();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("modelswitch-corrupt-poison-")
                    && name_str.contains(".corrupt.")
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    #[tokio::test]
    async fn persist_after_successful_load_is_not_poisoned() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-healthy-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            w.insert(
                "key1".to_string(),
                TestData {
                    name: "test".to_string(),
                    value: 42,
                },
            );
        }
        store.persist().await;

        // Load into a new store — should NOT be poisoned
        let store2: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        store2.load().await;
        assert!(
            !store2.poisoned.load(std::sync::atomic::Ordering::SeqCst),
            "store should NOT be poisoned after successful load"
        );

        // Persist should work
        {
            let mut w = store2.write().await;
            w.insert(
                "key2".to_string(),
                TestData {
                    name: "v2".to_string(),
                    value: 99,
                },
            );
        }
        store2.persist().await;
        assert!(
            path.exists(),
            "file should exist after non-poisoned persist"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn persist_sync_uses_atomic_write() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-sync-atomic-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            w.insert(
                "key1".to_string(),
                TestData {
                    name: "test".to_string(),
                    value: 42,
                },
            );
        }

        store.persist_sync();

        // Verify data was written
        assert!(path.exists(), "file should exist after sync persist");
        let data = std::fs::read_to_string(&path).unwrap();
        assert!(data.contains("key1"), "data should contain key1");

        // Temp file should not exist
        let tmp_path = path.with_extension("json.tmp");
        assert!(
            !tmp_path.exists(),
            "temp file should be cleaned up after atomic rename"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn async_persist_cleans_up_temp_file() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-temp-test-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            w.insert(
                "k".to_string(),
                TestData {
                    name: "v".to_string(),
                    value: 1,
                },
            );
        }
        store.persist().await;

        let temp_path = path.with_extension("json.tmp");
        assert!(
            !temp_path.exists(),
            "temp file should be cleaned up after successful persist"
        );
        assert!(path.exists(), "main file should exist");

        let _ = std::fs::remove_file(&path);
    }

    /// After `persist()`, the data file must exist, contain valid JSON, and
    /// no `.tmp` file should remain. Combines the atomic-rename completion
    /// check with a content-validity check in a single test.
    #[tokio::test]
    async fn async_persist_uses_atomic_write() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-atomic-write-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let tmp_path = path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&tmp_path);

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            w.insert(
                "alpha".to_string(),
                TestData {
                    name: "first".to_string(),
                    value: 7,
                },
            );
        }
        store.persist().await;

        // Atomic rename completed: real file exists, temp file gone.
        assert!(path.exists(), "data file should exist after persist");
        assert!(
            !tmp_path.exists(),
            "no .tmp file should remain after atomic rename"
        );

        // File content must be valid JSON parseable back into the map.
        let raw = std::fs::read_to_string(&path).expect("data file should be readable");
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("persisted data must be valid JSON");
        assert!(
            parsed.get("alpha").is_some(),
            "persisted JSON should contain the stored key"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// `load()` uses `entry(k).or_insert(v)` semantics — existing in-memory
    /// entries must NOT be overwritten by disk data. New keys from disk are
    /// merged in, but in-memory values win on conflict.
    #[tokio::test]
    async fn async_load_merges_without_overwriting() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-merge-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        // Disk file contains: loaded_only, shared_key (value=1)
        let disk_json = r#"{"loaded_only":{"name":"from_disk","value":1},"shared_key":{"name":"from_disk","value":1}}"#;
        std::fs::write(&path, disk_json).expect("write disk file");

        let store: PersistedStore<String, TestData> = PersistedStore::new(path.clone());
        {
            let mut w = store.write().await;
            // in-memory only entry
            w.insert(
                "memory_only".to_string(),
                TestData {
                    name: "from_memory".to_string(),
                    value: 99,
                },
            );
            // shared with disk — in-memory value must win
            w.insert(
                "shared_key".to_string(),
                TestData {
                    name: "from_memory".to_string(),
                    value: 100,
                },
            );
        }

        store.load().await;

        {
            let r = store.read().await;
            // Disk-only key should be merged in.
            let loaded = r
                .get("loaded_only")
                .expect("disk-only key should be merged in");
            assert_eq!(loaded.value, 1, "loaded_only should retain disk value");
            assert_eq!(loaded.name, "from_disk");

            // Memory-only key should still be present.
            let mem = r
                .get("memory_only")
                .expect("memory-only key should still be present");
            assert_eq!(mem.value, 99);

            // Shared key should retain the in-memory value (NOT overwritten).
            let shared = r
                .get("shared_key")
                .expect("shared_key must exist after merge");
            assert_eq!(
                shared.value, 100,
                "load() must not overwrite existing in-memory entries (or_insert semantics)"
            );
            assert_eq!(
                shared.name, "from_memory",
                "in-memory value should win on key collision"
            );
        }

        let _ = std::fs::remove_file(&path);
    }
}
