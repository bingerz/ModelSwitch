//! Admin audit logging — ring-buffer for administrative actions.
//!
//! All administrative operations (channel CRUD, virtual key CRUD, config
//! changes) are recorded as `AuditEntry` records in a bounded ring buffer.
//! The buffer evicts the oldest entries when capacity is reached, similar
//! to how `DispatchLogger` operates.
//!
//! When a `log_file` path is configured, every recorded entry is also
//! appended as NDJSON to disk so that audit history survives process
//! restarts. On startup, [`AuditLog::load_from_file`] reads the NDJSON
//! file back into the in-memory ring buffer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Default ring-buffer capacity.
const DEFAULT_MAX_ENTRIES: usize = 1000;

/// A single audit log entry recording an administrative action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When the action occurred.
    pub timestamp: DateTime<Utc>,
    /// Action namespace, e.g. `"channel.create"`, `"vkey.delete"`.
    pub action: String,
    /// Who performed the action — IP address or `"system"` for the config watcher.
    pub actor: String,
    /// Target resource identifier (e.g. channel UUID).
    pub target: String,
    /// Additional context (before/after diff, request fields, etc.).
    pub details: serde_json::Value,
}

/// Ring-buffer audit log for administrative actions.
///
/// Stores up to `max_entries` entries; oldest entries are evicted when
/// capacity is reached. Thread-safe via `Arc<RwLock<...>>`.
///
/// When `log_file` is `Some`, each recorded entry is appended to the NDJSON
/// file so that audit history survives restarts.
pub struct AuditLog {
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
    max_entries: usize,
    log_file: Option<PathBuf>,
}

impl AuditLog {
    /// Create a new `AuditLog` with the given capacity and no disk persistence.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: None,
        }
    }

    /// Create a new `AuditLog` with the default capacity (1000 entries) and
    /// no disk persistence.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }

    /// Create a new `AuditLog` that appends every entry as NDJSON to the
    /// given file path. The file is NOT read automatically; call
    /// [`load_from_file`] after construction to restore prior history.
    pub fn with_persistence(max_entries: usize, log_file: PathBuf) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: Some(log_file),
        }
    }

    /// Create a new `AuditLog` with the default capacity and disk persistence.
    pub fn with_default_capacity_and_persistence(log_file: PathBuf) -> Self {
        Self::with_persistence(DEFAULT_MAX_ENTRIES, log_file)
    }

    /// Read the NDJSON file (if configured) and populate the in-memory ring
    /// buffer with the most recent entries up to `max_entries`. Missing files
    /// are treated as an empty history. Malformed lines are skipped.
    pub async fn load_from_file(&self) {
        let path = match &self.log_file {
            Some(p) => p.clone(),
            None => return,
        };

        if let Err(e) = Self::ensure_parent_dir(&path).await {
            tracing::warn!(error = %e, "failed to create audit log directory");
            return;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("audit log file does not exist yet; starting empty");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to read audit log file");
                return;
            }
        };

        let mut loaded: Vec<AuditEntry> = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEntry>(trimmed) {
                Ok(entry) => loaded.push(entry),
                Err(e) => {
                    tracing::debug!(error = %e, "skipping malformed audit log line");
                }
            }
        }

        let mut entries = self.entries.write().await;
        entries.clear();
        // Keep only the most recent `max_entries` entries from the file.
        let start = loaded.len().saturating_sub(self.max_entries);
        for entry in loaded.into_iter().skip(start) {
            if entries.len() >= self.max_entries {
                break;
            }
            entries.push_back(entry);
        }

        tracing::info!(loaded = entries.len(), "audit log entries loaded from file");
    }

    /// Record a new audit entry. If the buffer is at capacity, the oldest
    /// entry is evicted (FIFO ring-buffer semantics). When a log file is
    /// configured, the entry is also appended as NDJSON to disk.
    pub async fn record(
        &self,
        action: &str,
        actor: &str,
        target: &str,
        details: serde_json::Value,
    ) {
        let entry = AuditEntry {
            timestamp: Utc::now(),
            action: action.to_string(),
            actor: actor.to_string(),
            target: target.to_string(),
            details,
        };

        if let Some(ref path) = self.log_file {
            let line = match serde_json::to_string(&entry) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize audit entry");
                    String::new()
                }
            };
            if !line.is_empty() {
                let file_path = path.clone();
                crate::spawn_bg(async move {
                    if let Err(e) = Self::append_line(&file_path, &line).await {
                        tracing::warn!(error = %e, "failed to append audit entry to file");
                    }
                });
            }
        }

        let mut entries = self.entries.write().await;
        if entries.len() >= self.max_entries {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    /// Return recent entries in newest-first order.
    ///
    /// If `limit` is `None`, returns all stored entries.
    pub async fn list(&self, limit: Option<usize>) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        let iter = entries.iter().rev();
        match limit {
            Some(n) => iter.take(n).cloned().collect(),
            None => iter.cloned().collect(),
        }
    }

    /// Total number of stored entries.
    pub async fn total(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Clear all audit entries.
    pub async fn clear(&self) {
        self.entries.write().await.clear();
    }
}

// ─── Private helpers for file persistence ──────────────
impl AuditLog {
    async fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(())
    }

    async fn append_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(())
    }
}

// ─── Admin Endpoint ─────────────────────────────────────

fn default_limit() -> Option<usize> {
    Some(100)
}

/// Query parameters for `GET /api/audit-log`.
#[derive(Debug, Deserialize)]
pub struct AuditLogParams {
    /// Maximum number of entries to return (default 100).
    #[serde(default = "default_limit")]
    pub limit: Option<usize>,
}

/// `GET /api/audit-log` — return recent audit entries, newest first.
pub async fn get_audit_log(
    axum::extract::State(state): axum::extract::State<Arc<crate::proxy::AppState>>,
    axum::extract::Query(params): axum::extract::Query<AuditLogParams>,
) -> axum::Json<super::ApiResponse<Vec<AuditEntry>>> {
    let entries = state.audit_log.list(params.limit).await;
    axum::Json(super::ApiResponse::ok(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn record_stores_entry() {
        let log = AuditLog::new(100);
        log.record(
            "channel.create",
            "127.0.0.1",
            "ch-001",
            json!({"name": "test"}),
        )
        .await;

        let entries = log.list(None).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "channel.create");
        assert_eq!(entries[0].actor, "127.0.0.1");
        assert_eq!(entries[0].target, "ch-001");
        assert_eq!(entries[0].details["name"], "test");
    }

    #[tokio::test]
    async fn list_returns_newest_first() {
        let log = AuditLog::new(100);
        log.record("channel.create", "a", "t1", json!({})).await;
        log.record("channel.update", "b", "t2", json!({})).await;
        log.record("channel.delete", "c", "t3", json!({})).await;

        let entries = log.list(None).await;
        assert_eq!(entries.len(), 3);
        // Newest first
        assert_eq!(entries[0].action, "channel.delete");
        assert_eq!(entries[1].action, "channel.update");
        assert_eq!(entries[2].action, "channel.create");
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let log = AuditLog::new(100);
        for i in 0..10 {
            log.record(
                "channel.create",
                "actor",
                &format!("target-{i}"),
                json!({"index": i}),
            )
            .await;
        }

        let limited = log.list(Some(3)).await;
        assert_eq!(limited.len(), 3);
        // Should be the 3 newest: indices 9, 8, 7
        assert_eq!(limited[0].details["index"], 9);
        assert_eq!(limited[1].details["index"], 8);
        assert_eq!(limited[2].details["index"], 7);
    }

    #[tokio::test]
    async fn list_none_limit_returns_all() {
        let log = AuditLog::new(100);
        for i in 0..5 {
            log.record("test.action", "a", &format!("t{i}"), json!({}))
                .await;
        }
        let all = log.list(None).await;
        assert_eq!(all.len(), 5);
    }

    #[tokio::test]
    async fn list_empty_returns_empty_vec() {
        let log = AuditLog::new(100);
        let entries = log.list(None).await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn capacity_evicts_oldest() {
        let log = AuditLog::new(3);
        log.record("a", "actor", "t1", json!({})).await;
        log.record("b", "actor", "t2", json!({})).await;
        log.record("c", "actor", "t3", json!({})).await;
        log.record("d", "actor", "t4", json!({})).await;

        let entries = log.list(None).await;
        // Only 3 entries — oldest ("a") evicted
        assert_eq!(entries.len(), 3);
        // Newest first: d, c, b
        assert_eq!(entries[0].action, "d");
        assert_eq!(entries[1].action, "c");
        assert_eq!(entries[2].action, "b");
    }

    #[tokio::test]
    async fn capacity_evicts_multiple() {
        let log = AuditLog::new(2);
        for i in 0..10 {
            log.record("action", "actor", &format!("t{i}"), json!({}))
                .await;
        }

        let entries = log.list(None).await;
        assert_eq!(entries.len(), 2);
        // Only the last two recorded entries survive
        assert_eq!(entries[0].target, "t9");
        assert_eq!(entries[1].target, "t8");
    }

    #[tokio::test]
    async fn clear_empties_all_entries() {
        let log = AuditLog::new(100);
        log.record("a", "actor", "t1", json!({})).await;
        log.record("b", "actor", "t2", json!({})).await;
        assert_eq!(log.total().await, 2);

        log.clear().await;
        assert_eq!(log.total().await, 0);
        assert!(log.list(None).await.is_empty());
    }

    #[tokio::test]
    async fn total_tracks_entry_count() {
        let log = AuditLog::new(100);
        assert_eq!(log.total().await, 0);

        log.record("a", "x", "t", json!({})).await;
        assert_eq!(log.total().await, 1);

        log.record("b", "x", "t", json!({})).await;
        assert_eq!(log.total().await, 2);
    }

    #[tokio::test]
    async fn with_default_capacity_creates_1000_capacity() {
        let log = AuditLog::with_default_capacity();
        for i in 0..1050 {
            log.record("action", "actor", &format!("t{i}"), json!({}))
                .await;
        }
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 1000);
        // First entry should be the 1050th (newest), last should be the 51st
        assert_eq!(entries[0].target, "t1049");
        assert_eq!(entries[999].target, "t50");
    }

    #[tokio::test]
    async fn record_after_clear_works() {
        let log = AuditLog::new(100);
        log.record("a", "actor", "t1", json!({})).await;
        log.clear().await;
        log.record("b", "actor", "t2", json!({})).await;

        let entries = log.list(None).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "b");
    }

    #[tokio::test]
    async fn timestamp_is_set_to_now() {
        let log = AuditLog::new(100);
        let before = Utc::now();
        log.record("test", "actor", "target", json!({})).await;
        let after = Utc::now();

        let entries = log.list(None).await;
        assert!(entries[0].timestamp >= before);
        assert!(entries[0].timestamp <= after);
    }

    #[tokio::test]
    async fn persistence_appends_to_file() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-test-{}.ndjson",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let log = AuditLog::with_persistence(100, path.clone());
        log.record("test.action", "actor", "target", json!({"k": "v"}))
            .await;
        // Background append — give it a moment to flush.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("test.action"));
        assert!(content.contains("\"k\":\"v\""));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn load_from_file_restores_history() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-load-{}.ndjson",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        // Write entries directly to the NDJSON file (bypassing spawn_bg timing).
        let mut content = String::new();
        for action in &["a", "b", "c"] {
            let entry = serde_json::json!({
                "timestamp": "2025-01-01T00:00:00Z",
                "action": action,
                "actor": "x",
                "target": format!("t-{}", action),
                "details": {}
            });
            content.push_str(&entry.to_string());
            content.push('\n');
        }
        tokio::fs::write(&path, &content).await.unwrap();

        // Fresh instance loads from disk.
        let log = AuditLog::with_persistence(100, path.clone());
        log.load_from_file().await;
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 3, "all 3 entries should be restored");
        // Newest first (last line is newest).
        assert_eq!(entries[0].action, "c");
        assert_eq!(entries[1].action, "b");
        assert_eq!(entries[2].action, "a");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn load_from_file_respects_capacity() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-cap-{}.ndjson",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        // Write 10 entries directly to the NDJSON file.
        let mut content = String::new();
        for i in 0..10 {
            let entry = serde_json::json!({
                "timestamp": "2025-01-01T00:00:00Z",
                "action": "action",
                "actor": "actor",
                "target": format!("t{i}"),
                "details": {}
            });
            content.push_str(&entry.to_string());
            content.push('\n');
        }
        tokio::fs::write(&path, &content).await.unwrap();

        // Fresh instance with smaller capacity — only the 3 newest survive.
        let log = AuditLog::with_persistence(3, path.clone());
        log.load_from_file().await;
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 3, "capacity should cap restored entries");
        assert_eq!(entries[0].target, "t9");
        assert_eq!(entries[1].target, "t8");
        assert_eq!(entries[2].target, "t7");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn load_from_file_missing_file_is_empty() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-missing-{}.ndjson",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let log = AuditLog::with_persistence(100, path.clone());
        log.load_from_file().await;
        let entries = log.list(None).await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn load_from_file_skips_malformed_lines() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-malformed-{}.ndjson",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        // Write one valid then one malformed line then a valid line.
        let valid = serde_json::json!({
            "timestamp": "2025-01-01T00:00:00Z",
            "action": "valid",
            "actor": "a",
            "target": "t",
            "details": {}
        })
        .to_string();
        let mut content = String::new();
        content.push_str(&valid);
        content.push('\n');
        content.push_str("this is not json\n");
        content.push_str(&valid);
        content.push('\n');
        tokio::fs::write(&path, &content).await.unwrap();

        let log = AuditLog::with_persistence(100, path.clone());
        log.load_from_file().await;
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 2, "malformed line should be skipped");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn without_persistence_does_not_panic() {
        // Default in-memory log should never touch disk.
        let log = AuditLog::with_default_capacity();
        log.record("action", "actor", "t", json!({})).await;
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 1);
    }
}
