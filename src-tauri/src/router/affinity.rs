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
        self.state.write().await.set(session_id.to_string(), channel_id);
    }

    /// Periodic cleanup of expired entries.
    pub async fn cleanup(&self) {
        self.state.write().await.cleanup_expired();
    }

    /// Extract session ID from a request body.
    /// Checks metadata.session_id, user field, or hashes the first message content.
    pub fn extract_session_id(body: &serde_json::Value) -> Option<String> {
        // Check metadata.session_id (OpenAI format)
        if let Some(meta) = body.get("metadata").and_then(|m| m.as_object()) {
            if let Some(sid) = meta.get("session_id").and_then(|s| s.as_str()) {
                return Some(sid.to_string());
            }
        }
        // Check user field
        if let Some(user) = body.get("user").and_then(|u| u.as_str()) {
            if !user.is_empty() {
                return Some(format!("user:{}", user));
            }
        }
        None
    }
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
    fn extract_session_id_from_metadata() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "metadata": {"session_id": "abc-123"}
        });
        assert_eq!(SessionAffinity::extract_session_id(&body), Some("abc-123".to_string()));
    }

    #[test]
    fn extract_session_id_from_user() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "user": "user@example.com"
        });
        assert_eq!(SessionAffinity::extract_session_id(&body), Some("user:user@example.com".to_string()));
    }

    #[test]
    fn no_session_id() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        });
        assert_eq!(SessionAffinity::extract_session_id(&body), None);
    }
}
