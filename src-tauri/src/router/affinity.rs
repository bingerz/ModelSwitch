use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Maximum number of session entries before LRU eviction kicks in.
const MAX_ENTRIES: usize = 10_000;

/// Every N-th `set_channel` call triggers a cleanup of expired entries,
/// amortising the cost of scanning for stale rows.
const CLEANUP_INTERVAL: u32 = 64;

/// Session affinity map: session_id -> (channel_id, last_used)
struct AffinityState {
    map: HashMap<String, (Uuid, Instant)>,
    ttl: Duration,
    max_entries: usize,
    /// Monotonic counter used to probabilistically trigger cleanup.
    set_counter: u32,
}

impl AffinityState {
    /// Look up a session and update its last-access timestamp (LRU semantics).
    ///
    /// Expired entries are removed lazily during lookup.
    fn get(&mut self, session_id: &str) -> Option<Uuid> {
        let is_expired = self
            .map
            .get(session_id)
            .map(|(_, last)| last.elapsed() >= self.ttl)
            .unwrap_or(true);

        if is_expired {
            self.map.remove(session_id);
            return None;
        }

        // Update last access time so recently-used entries survive LRU eviction.
        if let Some((ch_id, last)) = self.map.get_mut(session_id) {
            *last = Instant::now();
            Some(*ch_id)
        } else {
            None
        }
    }

    fn set(&mut self, session_id: String, channel_id: Uuid) {
        self.map.insert(session_id, (channel_id, Instant::now()));

        // Probabilistic cleanup of expired entries — runs roughly every
        // CLEANUP_INTERVAL calls so the cost is amortised across many inserts.
        self.set_counter = self.set_counter.wrapping_add(1);
        if self.set_counter % CLEANUP_INTERVAL == 0 {
            self.cleanup_expired();
        }

        // Evict least-recently-used entries when the map exceeds capacity.
        self.evict_lru();
    }

    fn cleanup_expired(&mut self) {
        self.map.retain(|_, (_, last)| last.elapsed() < self.ttl);
    }

    /// Evict least-recently-used entries until the map fits within `max_entries`.
    fn evict_lru(&mut self) {
        if self.map.len() <= self.max_entries {
            return;
        }

        // Collect (session_id, last_used) pairs and sort oldest-first.
        let mut entries: Vec<(String, Instant)> = self
            .map
            .iter()
            .map(|(k, (_, t))| (k.clone(), *t))
            .collect();
        entries.sort_by_key(|(_, t)| *t);

        let excess = self.map.len() - self.max_entries;
        for (key, _) in entries.iter().take(excess) {
            self.map.remove(key);
        }
    }
}

pub struct SessionAffinity {
    state: Arc<RwLock<AffinityState>>,
}

impl Clone for SessionAffinity {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl SessionAffinity {
    pub fn new(ttl_secs: u64) -> Self {
        Self::with_options(Duration::from_secs(ttl_secs), MAX_ENTRIES)
    }

    /// Create with a custom maximum entry count (primarily for testing).
    pub fn new_with_capacity(ttl_secs: u64, max_entries: usize) -> Self {
        Self::with_options(Duration::from_secs(ttl_secs), max_entries)
    }

    fn with_options(ttl: Duration, max_entries: usize) -> Self {
        Self {
            state: Arc::new(RwLock::new(AffinityState {
                map: HashMap::new(),
                ttl,
                max_entries,
                set_counter: 0,
            })),
        }
    }

    pub async fn get_channel(&self, session_id: &str) -> Option<Uuid> {
        self.state.write().await.get(session_id)
    }

    pub async fn set_channel(&self, session_id: &str, channel_id: Uuid) {
        self.state
            .write()
            .await
            .set(session_id.to_string(), channel_id);
    }

    /// Periodic cleanup of expired entries.
    pub async fn cleanup(&self) {
        self.state.write().await.cleanup_expired();
    }

    /// Current number of tracked sessions (diagnostics / testing).
    pub async fn len(&self) -> usize {
        self.state.read().await.map.len()
    }

    /// Extract a session identifier from the request, checking multiple sources.
    ///
    /// Sources are checked in priority order:
    /// 1. `metadata.user_id` in the request body (Claude Code session format)
    /// 2. `X-Session-ID` header
    /// 3. `Session_id` header (Codex format)
    /// 4. `X-Amp-Thread-Id` header
    /// 5. `X-Client-Request-Id` header
    /// 6. `conversation_id` field in the request body
    /// 7. Hash of the first 2 messages' content (fallback)
    ///
    /// Returns `None` if no session identifier can be determined.
    pub fn extract_session_id(headers: &HeaderMap, body: &serde_json::Value) -> Option<String> {
        // 1. Check metadata.user_id in body (Claude Code format)
        if let Some(uid) = body
            .get("metadata")
            .and_then(|m| m.get("user_id"))
            .and_then(|u| u.as_str())
        {
            if !uid.is_empty() {
                return Some(format!("uid:{}", uid));
            }
        }

        // 2-5. Check headers in priority order
        for header_name in &[
            "x-session-id",
            "session_id",
            "x-amp-thread-id",
            "x-client-request-id",
        ] {
            if let Some(val) = headers.get(*header_name).and_then(|h| h.to_str().ok()) {
                if !val.is_empty() {
                    return Some(format!("hdr:{}", val));
                }
            }
        }

        // 6. Check conversation_id in body
        if let Some(cid) = body.get("conversation_id").and_then(|c| c.as_str()) {
            if !cid.is_empty() {
                return Some(format!("cid:{}", cid));
            }
        }

        // 7. Fallback: hash first 2 messages
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let prefix: Vec<&serde_json::Value> = messages.iter().take(2).collect();
            if !prefix.is_empty() {
                let serialized = serde_json::to_string(&prefix).unwrap_or_default();
                let hash = sha256_short(&serialized);
                return Some(format!("msg:{}", hash));
            }
        }

        None
    }
}

/// Produce a short hex fingerprint (first 8 bytes of SHA-256 = 16 hex chars).
fn sha256_short(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(16);
    for byte in bytes.iter().take(8) {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Default TTL: 30 minutes
impl Default for SessionAffinity {
    fn default() -> Self {
        Self::new(1800)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_affinity() {
        let affinity = SessionAffinity::new(1800);
        let ch_id = Uuid::new_v4();
        affinity.set_channel("session-1", ch_id).await;
        assert_eq!(affinity.get_channel("session-1").await, Some(ch_id));
        assert_eq!(affinity.get_channel("unknown").await, None);
    }

    #[test]
    fn extract_session_id_from_metadata_user_id() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({
            "model": "claude-sonnet-4",
            "messages": [{"role": "user", "content": "hi"}],
            "metadata": {"user_id": "user-abc-123"}
        });
        assert_eq!(
            SessionAffinity::extract_session_id(&headers, &body),
            Some("uid:user-abc-123".to_string())
        );
    }

    #[test]
    fn extract_session_id_from_x_session_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", "sess-xyz".parse().unwrap());
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(
            SessionAffinity::extract_session_id(&headers, &body),
            Some("hdr:sess-xyz".to_string())
        );
    }

    #[test]
    fn extract_session_id_from_session_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("session_id", "codex-123".parse().unwrap());
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(
            SessionAffinity::extract_session_id(&headers, &body),
            Some("hdr:codex-123".to_string())
        );
    }

    #[test]
    fn extract_session_id_from_conversation_id() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "conversation_id": "conv-456"
        });
        assert_eq!(
            SessionAffinity::extract_session_id(&headers, &body),
            Some("cid:conv-456".to_string())
        );
    }

    #[test]
    fn extract_session_id_from_message_hash_fallback() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "Hello world"},
                {"role": "assistant", "content": "Hi there"}
            ]
        });
        let sid = SessionAffinity::extract_session_id(&headers, &body);
        assert!(sid.is_some(), "should produce a message-hash fallback");
        let sid = sid.unwrap();
        assert!(
            sid.starts_with("msg:"),
            "fallback should be prefixed with msg:"
        );
        // Same input should produce the same hash deterministically
        let sid2 = SessionAffinity::extract_session_id(&headers, &body);
        assert_eq!(sid2.as_deref(), Some(sid.as_str()));
    }

    #[test]
    fn extract_session_id_returns_none_for_empty_request() {
        let headers = HeaderMap::new();
        let body = serde_json::json!({
            "model": "gpt-4o"
        });
        assert_eq!(SessionAffinity::extract_session_id(&headers, &body), None);
    }

    #[test]
    fn metadata_user_id_takes_priority_over_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", "from-header".parse().unwrap());
        let body = serde_json::json!({
            "metadata": {"user_id": "from-body"}
        });
        assert_eq!(
            SessionAffinity::extract_session_id(&headers, &body),
            Some("uid:from-body".to_string())
        );
    }

    // ------------------------------------------------------------------
    // LRU eviction tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn evicts_oldest_when_capacity_exceeded() {
        // Capacity of 3 — inserting a 4th should evict the oldest.
        let affinity = SessionAffinity::new_with_capacity(1800, 3);
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();
        let ch3 = Uuid::new_v4();
        let ch4 = Uuid::new_v4();

        affinity.set_channel("s1", ch1).await;
        affinity.set_channel("s2", ch2).await;
        affinity.set_channel("s3", ch3).await;
        assert_eq!(affinity.len().await, 3);

        // Inserting a 4th unique session should evict s1 (oldest).
        affinity.set_channel("s4", ch4).await;
        assert_eq!(affinity.len().await, 3);

        // s1 should be gone; s2, s3, s4 should remain.
        assert_eq!(affinity.get_channel("s1").await, None);
        assert_eq!(affinity.get_channel("s2").await, Some(ch2));
        assert_eq!(affinity.get_channel("s3").await, Some(ch3));
        assert_eq!(affinity.get_channel("s4").await, Some(ch4));
    }

    #[tokio::test]
    async fn cleanup_removes_expired_entries() {
        // 50 ms TTL — entries expire almost immediately.
        let affinity = SessionAffinity::with_options(Duration::from_millis(50), 100);
        let ch = Uuid::new_v4();

        affinity.set_channel("s1", ch).await;
        affinity.set_channel("s2", ch).await;
        assert_eq!(affinity.len().await, 2);

        // Wait for entries to expire.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Explicit cleanup should purge expired rows.
        affinity.cleanup().await;
        assert_eq!(affinity.len().await, 0);
        assert_eq!(affinity.get_channel("s1").await, None);
    }

    #[tokio::test]
    async fn lru_updates_access_time() {
        let affinity = SessionAffinity::new_with_capacity(1800, 3);
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();
        let ch3 = Uuid::new_v4();
        let ch4 = Uuid::new_v4();

        // Fill to capacity.
        affinity.set_channel("s1", ch1).await;
        affinity.set_channel("s2", ch2).await;
        affinity.set_channel("s3", ch3).await;

        // Touch s1 so its last-access timestamp is newer than s2 and s3.
        let _ = affinity.get_channel("s1").await;
        // Small sleep guarantees the subsequent insert has a later timestamp.
        tokio::time::sleep(Duration::from_millis(2)).await;

        // Insert a 4th — s2 (not s1) should be evicted because s1 was
        // accessed more recently.
        affinity.set_channel("s4", ch4).await;

        assert_eq!(
            affinity.get_channel("s1").await,
            Some(ch1),
            "recently-accessed s1 should survive eviction"
        );
        assert_eq!(
            affinity.get_channel("s2").await,
            None,
            "oldest untouched s2 should be evicted"
        );
    }
}
