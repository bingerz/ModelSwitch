use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Session affinity map: session_id -> (channel_id, last_used)
struct AffinityState {
    map: HashMap<String, (Uuid, Instant)>,
    ttl: Duration,
}

impl AffinityState {
    fn get(&self, session_id: &str) -> Option<Uuid> {
        self.map.get(session_id).and_then(|(ch_id, last_used)| {
            if last_used.elapsed() < self.ttl {
                Some(*ch_id)
            } else {
                None
            }
        })
    }

    fn set(&mut self, session_id: String, channel_id: Uuid) {
        self.map.insert(session_id, (channel_id, Instant::now()));
    }

    fn cleanup_expired(&mut self) {
        self.map.retain(|_, (_, last)| last.elapsed() < self.ttl);
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
        Self {
            state: Arc::new(RwLock::new(AffinityState {
                map: HashMap::new(),
                ttl: Duration::from_secs(ttl_secs),
            })),
        }
    }

    pub async fn get_channel(&self, session_id: &str) -> Option<Uuid> {
        self.state.read().await.get(session_id)
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
}
