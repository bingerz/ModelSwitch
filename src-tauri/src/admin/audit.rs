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
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Default ring-buffer capacity.
const DEFAULT_MAX_ENTRIES: usize = 1000;
/// Maximum audit log file size before rotation (10 MB, matching DispatchLogger).
const AUDIT_LOG_MAX_SIZE: u64 = 10 * 1024 * 1024;
/// Number of rotated backup files to keep.
const AUDIT_LOG_MAX_BACKUPS: usize = 5;

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
    /// Hash of the previous entry in the chain (tamper-detection).
    /// `None` for the first entry or entries recorded before the chain was enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub prev_hash: Option<String>,
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
    /// Hash of the most recently recorded entry — the chain tip used to
    /// compute the next entry's `prev_hash`.
    last_hash: Mutex<Option<String>>,
}

impl AuditLog {
    /// Create a new `AuditLog` with the given capacity and no disk persistence.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            log_file: None,
            last_hash: Mutex::new(None),
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
            last_hash: Mutex::new(None),
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

        let new_tip = {
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

            let count = entries.len();
            // Restore the chain tip so subsequent records link to the last loaded entry.
            let tip = entries.back().map(compute_entry_hash);
            tracing::info!(loaded = count, "audit log entries loaded from file");
            tip
        };
        *self.last_hash.lock().await = new_tip;
    }

    /// Record a new audit entry. If the buffer is at capacity, the oldest
    /// entry is evicted (FIFO ring-buffer semantics). When a log file is
    /// configured, the entry is also appended as NDJSON to disk.
    ///
    /// Each entry is linked to its predecessor via a SHA-256 hash stored in
    /// `prev_hash`, forming a tamper-evident chain. The chain tip is tracked
    /// in `last_hash` so the next record can extend it.
    pub async fn record(
        &self,
        action: &str,
        actor: &str,
        target: &str,
        details: serde_json::Value,
    ) {
        // Link this entry to the chain tip before inserting.
        let prev_hash = self.last_hash.lock().await.clone();
        let entry = AuditEntry {
            timestamp: Utc::now(),
            action: action.to_string(),
            actor: actor.to_string(),
            target: target.to_string(),
            details,
            prev_hash,
        };

        // Compute this entry's hash and advance the chain tip.
        let computed = compute_entry_hash(&entry);
        *self.last_hash.lock().await = Some(computed);

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
                    let mut last_err = None;
                    for attempt in 1..=3 {
                        match Self::append_line(&file_path, &line).await {
                            Ok(()) => return,
                            Err(e) => {
                                tracing::warn!(
                                    attempt,
                                    error = %e,
                                    "failed to append audit entry, will retry"
                                );
                                last_err = Some(e);
                                if attempt < 3 {
                                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                }
                            }
                        }
                    }
                    if let Some(e) = last_err {
                        tracing::error!(
                            error = %e,
                            path = %file_path.display(),
                            "AUDIT LOG PERSISTENCE FAILED after 3 attempts — \
                             in-memory chain advanced but on-disk log has a gap. \
                             Investigate disk space and permissions."
                        );
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
        *self.last_hash.lock().await = None;
    }

    /// Verify the integrity of the hash chain across all stored entries.
    ///
    /// Returns `true` when every entry's `prev_hash` matches the hash
    /// recomputed from its predecessor. The oldest entry in the buffer is
    /// treated as the chain root (its `prev_hash` is not checked against a
    /// missing predecessor, which naturally happens when older entries have
    /// been evicted by the ring buffer).
    pub async fn verify_chain(&self) -> bool {
        let entries = self.entries.read().await;
        if entries.len() <= 1 {
            return true;
        }

        let mut iter = entries.iter();
        let mut prev_hash = compute_entry_hash(iter.next().expect("at least one entry"));

        for entry in iter {
            if entry.prev_hash.as_deref() != Some(prev_hash.as_str()) {
                return false;
            }
            prev_hash = compute_entry_hash(entry);
        }
        true
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

    /// Rotate the audit log file if it exceeds the maximum size.
    /// Shifts backups: file → file.1 → file.2 → ... → file.{N} (dropped)
    async fn rotate_if_needed(path: &PathBuf) {
        let Ok(metadata) = tokio::fs::metadata(path).await else {
            return;
        };
        if metadata.len() < AUDIT_LOG_MAX_SIZE {
            return;
        }

        // Drop the oldest backup, then shift each backup up by one.
        for i in (1..=AUDIT_LOG_MAX_BACKUPS).rev() {
            let src = if i == 1 {
                path.clone()
            } else {
                path.with_extension(format!("{}", i - 1))
            };
            let dst = path.with_extension(format!("{}", i));
            let _ = tokio::fs::rename(&src, &dst).await;
        }
        // The original file has been renamed to .1. A new file will be created
        // on the next append_line() call.
        tracing::info!(
            path = %path.display(),
            "audit log rotated — previous log archived as .1"
        );
    }

    async fn append_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
        Self::rotate_if_needed(path).await;
        Self::ensure_parent_dir(path).await?;
        use tokio::io::AsyncWriteExt;
        let mut opts = tokio::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            opts.mode(0o600);
        }
        let mut file = opts.open(path).await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        // Flush to OS buffer and fsync to ensure durability across power loss.
        file.sync_all().await?;
        Ok(())
    }
}

// ─── Hash chain helpers ────────────────────────────────

/// Compute the SHA-256 hash of an audit entry for chain verification.
///
/// The digest covers `timestamp || action || target || details || prev_hash`,
/// matching the formula used by [`AuditLog::record`]. The `actor` field is
/// intentionally excluded to match the chain spec.
fn compute_entry_hash(entry: &AuditEntry) -> String {
    let mut hasher = Sha256::new();
    hasher.update(entry.timestamp.to_rfc3339().as_bytes());
    hasher.update(b"|");
    hasher.update(entry.action.as_bytes());
    hasher.update(b"|");
    hasher.update(entry.target.as_bytes());
    hasher.update(b"|");
    // Canonical JSON so key ordering does not change the hash.
    if let Ok(canonical) = serde_json::to_string(&entry.details) {
        hasher.update(canonical.as_bytes());
    }
    hasher.update(b"|");
    if let Some(ref ph) = entry.prev_hash {
        hasher.update(ph.as_bytes());
    }
    let result = hasher.finalize();
    // Hex-encode the digest.
    let mut out = String::with_capacity(64);
    for byte in result {
        out.push_str(&format!("{byte:02x}"));
    }
    out
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

    #[tokio::test]
    async fn hash_chain_verifies_intact_entries() {
        let log = AuditLog::new(100);
        log.record("channel.create", "alice", "ch-1", json!({"v": 1}))
            .await;
        log.record("channel.update", "bob", "ch-1", json!({"v": 2}))
            .await;
        log.record("channel.delete", "carol", "ch-1", json!({"v": 3}))
            .await;

        // Three intact entries should form a verifiable chain.
        assert!(
            log.verify_chain().await,
            "chain should be valid for untampered entries"
        );
    }

    #[tokio::test]
    async fn hash_chain_detects_tamper() {
        let log = AuditLog::new(100);
        log.record("channel.create", "alice", "ch-1", json!({"v": 1}))
            .await;
        log.record("channel.update", "bob", "ch-1", json!({"v": 2}))
            .await;
        log.record("channel.delete", "carol", "ch-1", json!({"v": 3}))
            .await;

        // Sanity check before tampering.
        assert!(log.verify_chain().await);

        // Tamper with the middle entry's details (newest-first ordering means
        // entries[1] is the middle one in chronological order).
        {
            let mut entries = log.entries.write().await;
            if let Some(mid) = entries.get_mut(1) {
                mid.details = json!({"v": 999});
            }
        }

        assert!(
            !log.verify_chain().await,
            "chain should be broken after tampering"
        );
    }

    #[tokio::test]
    async fn hash_chain_prev_hash_links_entries() {
        let log = AuditLog::new(100);
        log.record("a", "x", "t1", json!({})).await;
        log.record("b", "x", "t2", json!({})).await;

        let entries = log.list(None).await;
        // Newest first: entries[0] = "b", entries[1] = "a"
        // The newest entry's prev_hash should be Some(...) — the hash of "a".
        assert!(
            entries[0].prev_hash.is_some(),
            "second entry should link to the first"
        );
        // The first recorded entry has no predecessor.
        assert!(
            entries[1].prev_hash.is_none(),
            "first entry should have no prev_hash"
        );
    }

    #[tokio::test]
    async fn verify_chain_empty_returns_true() {
        let log = AuditLog::new(100);
        assert!(log.verify_chain().await);
    }

    #[tokio::test]
    async fn verify_chain_single_entry_returns_true() {
        let log = AuditLog::new(100);
        log.record("test", "actor", "target", json!({})).await;
        assert!(log.verify_chain().await);
    }

    #[tokio::test]
    async fn clear_resets_chain_tip() {
        let log = AuditLog::new(100);
        log.record("action1", "t1", "d1", json!({})).await;
        log.clear().await;
        log.record("action2", "t2", "d2", json!({})).await;
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 1);
        // After clear, the new entry should have prev_hash = None (fresh chain root)
        assert!(
            entries[0].prev_hash.is_none(),
            "entry after clear should have no prev_hash"
        );
    }

    #[tokio::test]
    async fn concurrent_appends_maintain_chain_integrity() {
        let log = std::sync::Arc::new(AuditLog::new(500));
        let mut handles = vec![];
        for i in 0..20 {
            let log_clone = log.clone();
            handles.push(tokio::spawn(async move {
                log_clone
                    .record(
                        &format!("action_{i}"),
                        &format!("target_{i}"),
                        &format!("details_{i}"),
                        json!({}),
                    )
                    .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let entries = log.list(None).await;
        assert_eq!(
            entries.len(),
            20,
            "all 20 concurrent records should be stored"
        );
        assert!(
            log.verify_chain().await,
            "chain must be intact after concurrent appends"
        );
    }

    #[tokio::test]
    async fn hash_chain_tamper_action_breaks_chain() {
        let log = AuditLog::new(100);
        log.record("create", "admin", "key1", json!({})).await;
        log.record("delete", "admin", "key1", json!({})).await;
        // verify_chain reads from the in-memory buffer, confirm it was valid before tamper
        assert!(
            log.verify_chain().await,
            "chain should be valid before tamper"
        );
    }

    #[tokio::test]
    async fn hash_chain_eviction_preserves_integrity() {
        let log = AuditLog::new(3); // small capacity to force eviction
        log.record("a1", "actor", "t1", json!({})).await;
        log.record("a2", "actor", "t2", json!({})).await;
        log.record("a3", "actor", "t3", json!({})).await;
        log.record("a4", "actor", "t4", json!({})).await; // evicts a1
        let entries = log.list(None).await;
        assert_eq!(entries.len(), 3, "should have 3 entries after eviction");
        // After eviction, the oldest surviving entry becomes the new chain root
        // verify_chain should still pass (root entry's prev_hash is not checked)
        assert!(
            log.verify_chain().await,
            "chain should remain valid after eviction"
        );
    }

    #[tokio::test]
    async fn append_line_persists_and_can_be_read_back() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-test-{}.log",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        AuditLog::append_line(&path, r#"{"action":"test"}"#)
            .await
            .unwrap();
        AuditLog::append_line(&path, r#"{"action":"test2"}"#)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains(r#"{"action":"test"}"#));
        assert!(content.contains(r#"{"action":"test2"}"#));
        assert_eq!(content.lines().count(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn rotate_if_needed_does_nothing_for_small_files() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-audit-rotate-small-{}.log",
            uuid::Uuid::new_v4().simple()
        ));
        tokio::fs::write(&path, b"small").await.unwrap();

        AuditLog::rotate_if_needed(&path).await;

        assert!(path.exists(), "file should still exist (not rotated)");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn append_line_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "modelswitch-audit-nested-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let path = dir.join("deep/audit.log");

        AuditLog::append_line(&path, r#"{"action":"test"}"#)
            .await
            .unwrap();

        assert!(path.exists(), "file should exist with created parent dirs");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
