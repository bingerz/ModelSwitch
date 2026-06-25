//! Admin audit logging — ring-buffer for administrative actions.
//!
//! All administrative operations (channel CRUD, virtual key CRUD, config
//! changes) are recorded as `AuditEntry` records in a bounded ring buffer.
//! The buffer evicts the oldest entries when capacity is reached, similar
//! to how `DispatchLogger` operates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Default ring-buffer capacity.
const DEFAULT_MAX_ENTRIES: usize = 1000;

/// A single audit log entry recording an administrative action.
#[derive(Debug, Clone, Serialize)]
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
pub struct AuditLog {
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
    max_entries: usize,
}

impl AuditLog {
    /// Create a new `AuditLog` with the given capacity.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
        }
    }

    /// Create a new `AuditLog` with the default capacity (1000 entries).
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }

    /// Record a new audit entry. If the buffer is at capacity, the oldest
    /// entry is evicted (FIFO ring-buffer semantics).
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
}
