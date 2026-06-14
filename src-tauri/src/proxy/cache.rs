use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A simple in-memory request cache with bounded size.
/// Caches non-streaming requests by hash of model + body (excluding stream field).
pub struct RequestCache {
    entries: Mutex<HashMap<u64, CacheEntry>>,
    /// Insertion order for LRU eviction.
    order: Mutex<Vec<u64>>,
    ttl: Duration,
    max_entries: usize,
}

struct CacheEntry {
    response_body: String,
    cached_at: Instant,
}

impl RequestCache {
    /// Create a new cache with the given TTL and max entries.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            ttl,
            max_entries,
        }
    }

    /// Compute a cache key from the model and request body (excluding dynamic fields).
    pub fn cache_key(model: &str, body: &serde_json::Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut h = DefaultHasher::new();
        model.hash(&mut h);
        // Hash the body without the stream field for cache stability
        if let Some(mut map) = body.as_object().cloned() {
            map.remove("stream");
            let stable = serde_json::to_string(&map).unwrap_or_default();
            stable.hash(&mut h);
        }
        h.finish()
    }

    /// Try to get a cached response. Returns None if expired or not found.
    pub fn get(&self, key: u64) -> Option<String> {
        let mut guard = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        // Clean expired entries on every read
        let ttl = self.ttl;
        let before = guard.len();
        guard.retain(|k, entry| {
            let valid = entry.cached_at.elapsed() < ttl;
            if !valid {
                order.retain(|ok| *ok != *k);
            }
            valid
        });
        if guard.len() < before {
            tracing::debug!(evicted = before - guard.len(), "Cache TTL eviction");
        }
        guard.get(&key).map(|e| e.response_body.clone())
    }

    /// Insert a response into the cache.
    pub fn insert(&self, key: u64, response_body: String) {
        let mut guard = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();

        // If key already exists, remove old position
        if guard.contains_key(&key) {
            order.retain(|k| *k != key);
        }

        guard.insert(
            key,
            CacheEntry {
                response_body,
                cached_at: Instant::now(),
            },
        );
        order.push(key);

        // Evict oldest if over capacity
        while guard.len() > self.max_entries {
            if let Some(old_key) = order.first().copied() {
                order.remove(0);
                guard.remove(&old_key);
            } else {
                break;
            }
        }
    }

    /// Returns the current number of cached entries (for diagnostics).
    pub fn len(&self) -> usize {
        let guard = self.entries.lock().unwrap();
        guard.len()
    }

    /// Clear all cached entries.
    pub fn flush(&self) {
        let mut guard = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        guard.clear();
        order.clear();
    }
}

/// Default TTL: 5 minutes, max 1000 entries
impl Default for RequestCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(300), 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_key_excludes_stream() {
        let body1 = json!({ "model": "gpt-4", "messages": [], "stream": true });
        let body2 = json!({ "model": "gpt-4", "messages": [], "stream": false });
        assert_eq!(
            RequestCache::cache_key("gpt-4", &body1),
            RequestCache::cache_key("gpt-4", &body2)
        );
    }

    #[test]
    fn cache_key_differs_by_model() {
        let body = json!({ "model": "gpt-4", "messages": [] });
        let k1 = RequestCache::cache_key("gpt-4", &body);
        let k2 = RequestCache::cache_key("gpt-3.5", &body);
        assert_ne!(k1, k2);
    }

    #[test]
    fn cache_hit_and_miss() {
        let cache = RequestCache::default();
        let key = RequestCache::cache_key("gpt-4", &json!({"messages": []}));
        assert!(cache.get(key).is_none());
        cache.insert(key, "cached response".to_string());
        assert_eq!(cache.get(key), Some("cached response".to_string()));
    }

    #[test]
    fn cache_tracks_length() {
        let cache = RequestCache::default();
        assert_eq!(cache.len(), 0);
        cache.insert(1, "a".to_string());
        cache.insert(2, "b".to_string());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn evicts_oldest_when_over_capacity() {
        let cache = RequestCache::new(Duration::from_secs(300), 3);
        cache.insert(1, "a".to_string());
        cache.insert(2, "b".to_string());
        cache.insert(3, "c".to_string());
        assert_eq!(cache.len(), 3);
        // Adding 4th should evict key 1 (oldest)
        cache.insert(4, "d".to_string());
        assert_eq!(cache.len(), 3);
        assert!(cache.get(1).is_none());
        assert_eq!(cache.get(2), Some("b".to_string()));
        assert_eq!(cache.get(4), Some("d".to_string()));
    }

    #[test]
    fn update_existing_key_preserves_capacity() {
        let cache = RequestCache::new(Duration::from_secs(300), 2);
        cache.insert(1, "a".to_string());
        cache.insert(2, "b".to_string());
        cache.insert(1, "updated".to_string());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(1), Some("updated".to_string()));
        // Key 3 should evict key 2 (oldest insertion order, key 1 was just updated)
        cache.insert(3, "c".to_string());
        assert_eq!(cache.len(), 2);
        assert!(cache.get(2).is_none());
    }

    #[test]
    fn flush_clears_everything() {
        let cache = RequestCache::default();
        cache.insert(1, "a".to_string());
        cache.insert(2, "b".to_string());
        cache.flush();
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1).is_none());
    }
}

/// Tracks in-flight requests for coalescing duplicate concurrent requests.
/// When multiple requests with the same cache key arrive, only one is sent
/// upstream; the rest wait and reuse the cached result.
pub struct InFlightRequests {
    inflight: Mutex<HashMap<u64, Arc<tokio::sync::Notify>>>,
}

impl InFlightRequests {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Register an in-flight request. Returns `true` if this is the first
    /// request for this key (caller should proceed with the real request),
    /// or `false` if another request is already in flight (caller should wait).
    pub fn register(&self, key: u64) -> bool {
        let mut guard = self.inflight.lock().unwrap();
        if guard.contains_key(&key) {
            false
        } else {
            guard.insert(key, Arc::new(tokio::sync::Notify::new()));
            true
        }
    }

    /// Wait for an in-flight request with the given key to complete.
    pub async fn wait(&self, key: u64) {
        let notify = {
            let guard = self.inflight.lock().unwrap();
            guard.get(&key).cloned()
        };
        if let Some(n) = notify {
            n.notified().await;
        }
    }

    /// Complete an in-flight request, waking all waiters.
    pub fn complete(&self, key: u64) {
        let mut guard = self.inflight.lock().unwrap();
        if let Some(notify) = guard.remove(&key) {
            notify.notify_waiters();
        }
    }
}

impl Default for InFlightRequests {
    fn default() -> Self {
        Self::new()
    }
}
