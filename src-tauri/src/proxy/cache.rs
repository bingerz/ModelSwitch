use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Controls cache read/write behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheMode {
    /// Cache is fully enabled (read + write).
    #[default]
    On,
    /// Cache is disabled entirely.
    Off,
    /// Read from cache but do not write new entries.
    ReadOnly,
    /// Write to cache but do not serve cached responses.
    WriteOnly,
}

impl CacheMode {
    /// Whether cached responses should be served.
    pub fn can_read(&self) -> bool {
        matches!(self, Self::On | Self::ReadOnly)
    }

    /// Whether new responses should be cached.
    pub fn can_write(&self) -> bool {
        matches!(self, Self::On | Self::WriteOnly)
    }

    /// Parse a cache mode from a configuration string.
    pub fn parse_mode(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "off" | "disabled" | "false" => Self::Off,
            "readonly" | "read-only" | "ro" => Self::ReadOnly,
            "writeonly" | "write-only" | "wo" => Self::WriteOnly,
            _ => Self::On,
        }
    }
}

/// A simple in-memory request cache with bounded size.
/// Caches non-streaming requests by hash of model + body (excluding stream field).
pub struct RequestCache {
    state: RwLock<CacheState>,
    ttl: Duration,
    max_entries: usize,
    mode: CacheMode,
}

struct CacheState {
    entries: HashMap<u128, CacheEntry>,
    /// Insertion order for LRU eviction.
    order: VecDeque<u128>,
}

struct CacheEntry {
    response_body: String,
    cached_at: Instant,
    /// The canonical string (model + body without stream) used to verify equality.
    key_material: String,
}

impl RequestCache {
    /// Create a new cache with the given TTL, max entries, and cache mode.
    pub fn new(ttl: Duration, max_entries: usize, mode: CacheMode) -> Self {
        Self {
            state: RwLock::new(CacheState {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
            ttl,
            max_entries,
            mode,
        }
    }

    /// Compute a cache key from the model and request body (excluding dynamic fields).
    pub fn cache_key(model: &str, body: &serde_json::Value) -> u128 {
        let material = canonical_key_material(model, body);
        let hash = blake3::hash(material.as_bytes());
        u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .expect("valid cache key length"),
        )
    }

    /// Compute both the hash key and the canonical key material.
    /// Callers should prefer this over `cache_key` + `canonical_key_material` separately
    /// to avoid recomputing the canonical string.
    pub fn compute_key(model: &str, body: &serde_json::Value) -> (u128, String) {
        let material = canonical_key_material(model, body);
        let hash = blake3::hash(material.as_bytes());
        let key = u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .expect("valid cache key length"),
        );
        (key, material)
    }

    /// Try to get a cached response. Returns None if expired, not found, or hash collision detected.
    /// The `key_material` is compared against the stored material to eliminate false positives
    /// from `u128` hash collisions.
    ///
    /// Read-only: expired entries are reported as misses but left in place.
    /// Actual eviction is handled by the periodic `sweep_expired()` task.
    pub fn get(&self, key: u128, key_material: &str) -> Option<String> {
        if !self.mode.can_read() {
            return None;
        }
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        // Read-only TTL check — expired entries are left in place and removed
        // later by the periodic sweep_expired() task. This keeps get() truly
        // read-only so concurrent reads never block each other.
        let entry = state.entries.get(&key)?;
        if entry.cached_at.elapsed() >= self.ttl {
            return None;
        }
        if entry.key_material == key_material {
            Some(entry.response_body.clone())
        } else {
            tracing::warn!("Cache hash collision detected for key {}", key);
            None
        }
    }

    /// Bulk-evict all expired entries. Returns the number of entries removed.
    ///
    /// Intended to be called periodically by a background task rather than on
    /// every cache read.
    pub fn sweep_expired(&self) -> usize {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        let before = state.entries.len();
        let ttl = self.ttl;
        // Collect expired keys first to avoid double mutable borrow of `state`.
        let expired: Vec<u128> = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.cached_at.elapsed() >= ttl)
            .map(|(k, _)| *k)
            .collect();
        for k in &expired {
            state.entries.remove(k);
            state.order.retain(|ok| ok != k);
        }
        let evicted = before - state.entries.len();
        if evicted > 0 {
            crate::metrics::cache_evictions().inc_by(evicted as u64);
            tracing::debug!(evicted, "Periodic cache sweep");
        }
        evicted
    }

    /// Insert a response into the cache.
    pub fn insert(&self, key: u128, key_material: String, response_body: String) {
        if !self.mode.can_write() {
            return;
        }
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());

        // If key already exists, remove old position from order
        if state.entries.contains_key(&key) {
            state.order.retain(|k| *k != key);
        }

        state.entries.insert(
            key,
            CacheEntry {
                response_body,
                key_material,
                cached_at: Instant::now(),
            },
        );
        state.order.push_back(key);

        // Evict oldest if over capacity — O(1) pop_front
        while state.entries.len() > self.max_entries {
            if let Some(old_key) = state.order.pop_front() {
                state.entries.remove(&old_key);
            } else {
                break;
            }
        }
    }

    /// Returns the current number of cached entries (for diagnostics).
    pub fn len(&self) -> usize {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        state.entries.len()
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all cached entries.
    pub fn flush(&self) {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.entries.clear();
        state.order.clear();
    }

    /// Returns the current cache mode.
    pub fn mode(&self) -> CacheMode {
        self.mode
    }
}

/// Compute the canonical string used for cache keying.
/// Excludes `stream` and `stream_options` fields from the body, since
/// `attempt.rs` injects `stream_options: {include_usage: true}` into streaming
/// requests. Without excluding `stream_options`, a streaming and non-streaming
/// request with otherwise identical bodies would share a cache key but receive
/// different upstream responses.
fn canonical_key_material(model: &str, body: &serde_json::Value) -> String {
    let mut buf = String::with_capacity(256);
    buf.push_str(model);
    buf.push('\0');

    if let Some(map) = body.as_object() {
        // Build canonical JSON without cloning the entire map.
        // Iterates by reference — only individual values are serialized.
        buf.push('{');
        let mut first = true;
        for (key, value) in map {
            if key == "stream" || key == "stream_options" {
                continue;
            }
            if !first {
                buf.push(',');
            }
            first = false;
            // Serialize key as JSON string (handles escaping)
            buf.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into()));
            buf.push(':');
            // Serialize value individually — much cheaper than cloning the whole map
            buf.push_str(&serde_json::to_string(value).unwrap_or_else(|_| "null".into()));
        }
        buf.push('}');
    } else {
        // Non-object body (array, string, etc.) — serialize as-is
        buf.push_str(&serde_json::to_string(body).unwrap_or_default());
    }

    buf
}

/// Default TTL: 5 minutes, max 1000 entries
impl Default for RequestCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(300), 1000, CacheMode::default())
    }
}

/// Tracks in-flight requests for coalescing duplicate concurrent requests.
/// When multiple requests with the same cache key arrive, only one is sent
/// upstream; the rest wait and reuse the cached result.
pub struct InFlightRequests {
    inflight: Mutex<HashMap<u128, Arc<tokio::sync::Notify>>>,
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
    pub fn register(&self, key: u128) -> bool {
        let mut guard = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if let std::collections::hash_map::Entry::Vacant(e) = guard.entry(key) {
            e.insert(Arc::new(tokio::sync::Notify::new()));
            true
        } else {
            false
        }
    }

    /// Wait for an in-flight request with the given key to complete.
    pub async fn wait(&self, key: u128) {
        let notify = {
            let guard = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(&key).cloned()
        };
        if let Some(n) = notify {
            n.notified().await;
        }
    }

    /// Complete an in-flight request, waking all waiters.
    pub fn complete(&self, key: u128) {
        let mut guard = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
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
    fn cache_key_excludes_stream_options() {
        // A streaming request with stream_options injected by attempt.rs
        let body_streaming = json!({
            "model": "gpt-4",
            "messages": [],
            "stream": true,
            "stream_options": { "include_usage": true }
        });
        // A non-streaming request without stream_options
        let body_non_streaming = json!({
            "model": "gpt-4",
            "messages": [],
            "stream": false
        });
        // They must produce the same cache key
        assert_eq!(
            RequestCache::cache_key("gpt-4", &body_streaming),
            RequestCache::cache_key("gpt-4", &body_non_streaming)
        );
    }

    #[test]
    fn cache_key_excludes_stream_options_only() {
        // Two requests that differ only in stream_options should share a key
        let body_with_options = json!({
            "messages": [],
            "stream_options": { "include_usage": true }
        });
        let body_without_options = json!({
            "messages": []
        });
        assert_eq!(
            RequestCache::cache_key("gpt-4", &body_with_options),
            RequestCache::cache_key("gpt-4", &body_without_options)
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
        let body = json!({"messages": []});
        let (key, material) = RequestCache::compute_key("gpt-4", &body);
        assert!(cache.get(key, &material).is_none());
        cache.insert(key, material.clone(), "cached response".to_string());
        assert_eq!(
            cache.get(key, &material),
            Some("cached response".to_string())
        );
    }

    #[test]
    fn cache_tracks_length() {
        let cache = RequestCache::default();
        assert_eq!(cache.len(), 0);
        cache.insert(1u128, "mat_1".to_string(), "a".to_string());
        cache.insert(2u128, "mat_2".to_string(), "b".to_string());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn evicts_oldest_when_over_capacity() {
        let cache = RequestCache::new(Duration::from_secs(300), 3, CacheMode::On);
        cache.insert(1u128, "mat_1".to_string(), "a".to_string());
        cache.insert(2u128, "mat_2".to_string(), "b".to_string());
        cache.insert(3u128, "mat_3".to_string(), "c".to_string());
        assert_eq!(cache.len(), 3);
        // Adding 4th should evict key 1 (oldest)
        cache.insert(4u128, "mat_4".to_string(), "d".to_string());
        assert_eq!(cache.len(), 3);
        assert!(cache.get(1u128, "mat_1").is_none());
        assert_eq!(cache.get(2u128, "mat_2"), Some("b".to_string()));
        assert_eq!(cache.get(4u128, "mat_4"), Some("d".to_string()));
    }

    #[test]
    fn update_existing_key_preserves_capacity() {
        let cache = RequestCache::new(Duration::from_secs(300), 2, CacheMode::On);
        cache.insert(1u128, "mat_1".to_string(), "a".to_string());
        cache.insert(2u128, "mat_2".to_string(), "b".to_string());
        cache.insert(1u128, "mat_1".to_string(), "updated".to_string());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(1u128, "mat_1"), Some("updated".to_string()));
        // Key 3 should evict key 2 (oldest insertion order, key 1 was just updated)
        cache.insert(3u128, "mat_3".to_string(), "c".to_string());
        assert_eq!(cache.len(), 2);
        assert!(cache.get(2u128, "mat_2").is_none());
    }

    #[test]
    fn flush_clears_everything() {
        let cache = RequestCache::default();
        cache.insert(1u128, "mat_1".to_string(), "a".to_string());
        cache.insert(2u128, "mat_2".to_string(), "b".to_string());
        cache.flush();
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1u128, "mat_1").is_none());
    }

    #[test]
    fn detects_hash_collision() {
        let cache = RequestCache::default();
        // Insert with one key_material
        cache.insert(
            42u128,
            "request_A_material".to_string(),
            "response_A".to_string(),
        );
        // Lookup with same hash but different material -> should miss
        let result = cache.get(42u128, "request_B_material");
        assert!(
            result.is_none(),
            "Should not return response for different key material"
        );
        // Lookup with same material -> should hit
        let result = cache.get(42u128, "request_A_material");
        assert_eq!(result, Some("response_A".to_string()));
    }

    #[test]
    fn cache_mode_read_only_serves_but_doesnt_write() {
        let cache = RequestCache::new(Duration::from_secs(300), 10, CacheMode::ReadOnly);
        // Can't write in ReadOnly mode
        cache.insert(1u128, "mat".to_string(), "response".to_string());
        assert_eq!(cache.len(), 0);
        // Can't read either since nothing was written
        assert!(cache.get(1u128, "mat").is_none());
    }

    #[test]
    fn cache_mode_write_only_writes_but_doesnt_serve() {
        let cache = RequestCache::new(Duration::from_secs(300), 10, CacheMode::WriteOnly);
        // Can write
        cache.insert(1u128, "mat".to_string(), "response".to_string());
        assert_eq!(cache.len(), 1);
        // Can't read in WriteOnly mode
        assert!(cache.get(1u128, "mat").is_none());
    }

    #[test]
    fn cache_mode_off_disables_everything() {
        let cache = RequestCache::new(Duration::from_secs(300), 10, CacheMode::Off);
        cache.insert(1u128, "mat".to_string(), "response".to_string());
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1u128, "mat").is_none());
    }

    #[test]
    fn cache_mode_from_str_parses_correctly() {
        assert_eq!(CacheMode::parse_mode("on"), CacheMode::On);
        assert_eq!(CacheMode::parse_mode("off"), CacheMode::Off);
        assert_eq!(CacheMode::parse_mode("readonly"), CacheMode::ReadOnly);
        assert_eq!(CacheMode::parse_mode("read-only"), CacheMode::ReadOnly);
        assert_eq!(CacheMode::parse_mode("ro"), CacheMode::ReadOnly);
        assert_eq!(CacheMode::parse_mode("writeonly"), CacheMode::WriteOnly);
        assert_eq!(CacheMode::parse_mode("write-only"), CacheMode::WriteOnly);
        assert_eq!(CacheMode::parse_mode("wo"), CacheMode::WriteOnly);
        assert_eq!(CacheMode::parse_mode("disabled"), CacheMode::Off);
        assert_eq!(CacheMode::parse_mode("false"), CacheMode::Off);
        assert_eq!(CacheMode::parse_mode("invalid"), CacheMode::On);
    }

    #[test]
    fn cache_mode_getter_returns_correct_mode() {
        let on = RequestCache::new(Duration::from_secs(300), 10, CacheMode::On);
        assert_eq!(on.mode(), CacheMode::On);

        let off = RequestCache::new(Duration::from_secs(300), 10, CacheMode::Off);
        assert_eq!(off.mode(), CacheMode::Off);

        let ro = RequestCache::new(Duration::from_secs(300), 10, CacheMode::ReadOnly);
        assert_eq!(ro.mode(), CacheMode::ReadOnly);

        let wo = RequestCache::new(Duration::from_secs(300), 10, CacheMode::WriteOnly);
        assert_eq!(wo.mode(), CacheMode::WriteOnly);
    }
}
