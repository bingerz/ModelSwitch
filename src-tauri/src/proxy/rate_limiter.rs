use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

const WINDOW_MS: u64 = 60_000; // 1 minute

/// Selectable rate-limiting algorithm.
///
/// `SlidingWindow` is the default and matches the historic behavior of the
/// gateway (per-channel TPM/RPM tracked over a rolling 60-second window).
/// `TokenBucket` provides burst-capable limiting — a large request can be
/// admitted as long as the bucket has accumulated enough tokens, and idle
/// periods refill the bucket up to its capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitAlgorithm {
    /// Rolling 60-second window with per-channel TPM/RPM counters (default).
    SlidingWindow,
    /// Burst-capable token bucket. Tokens refill continuously at `refill_rate`
    /// per second up to `capacity`, allowing short bursts above the average
    /// rate.
    TokenBucket,
}

impl Default for RateLimitAlgorithm {
    fn default() -> Self {
        Self::SlidingWindow
    }
}

/// A leaky-bucket-style token bucket rate limiter.
///
/// Tokens accumulate at a fixed `refill_rate` (tokens per second) up to
/// `capacity`. Each `try_consume` call deducts the requested amount; if the
/// bucket has fewer tokens than requested, the call returns `false` and the
/// bucket is left unchanged.
///
/// All operations are thread-safe — the bucket stores its mutable state behind
/// `parking_lot::Mutex`. The mutex is held only for the duration of each
/// method call, so contention is minimal.
pub struct TokenBucket {
    /// Tokens currently available (mutated under lock during refill/consume).
    tokens: Mutex<f64>,
    /// Maximum tokens (burst capacity).
    capacity: f64,
    /// Tokens added per second (refill rate).
    refill_rate: f64,
    /// Last refill timestamp (mutated under lock during refill).
    last_refill: Mutex<Instant>,
}

impl TokenBucket {
    /// Create a new bucket that starts completely full.
    ///
    /// # Panics
    /// Panics if `capacity` or `refill_rate` is not finite and positive.
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        assert!(
            capacity.is_finite() && capacity > 0.0,
            "capacity must be a positive finite number"
        );
        assert!(
            refill_rate.is_finite() && refill_rate > 0.0,
            "refill_rate must be a positive finite number"
        );
        Self {
            tokens: Mutex::new(capacity),
            capacity,
            refill_rate,
            last_refill: Mutex::new(Instant::now()),
        }
    }

    /// Refill the bucket based on elapsed wall-clock time since the last
    /// refill. Tokens are added proportionally to the elapsed duration and
    /// capped at `capacity`. Returns the number of tokens after refill.
    pub fn refill(&self) -> f64 {
        let now = Instant::now();
        let mut last = self.last_refill.lock();
        let elapsed = now.duration_since(*last);
        let added = elapsed.as_secs_f64() * self.refill_rate;
        let mut tokens = self.tokens.lock();
        *tokens = (*tokens + added).min(self.capacity);
        *last = now;
        *tokens
    }

    /// Attempt to consume `tokens` tokens from the bucket.
    ///
    /// Refills the bucket first (so time-elapsed credit is applied), then
    /// checks whether enough tokens are available. If yes, deducts and
    /// returns `true`; otherwise leaves the bucket unchanged and returns
    /// `false`.
    pub fn try_consume(&self, tokens: f64) -> bool {
        let available = self.refill();
        let mut current = self.tokens.lock();
        if available >= tokens {
            *current = available - tokens;
            true
        } else {
            *current = available;
            false
        }
    }

    /// Current token count after applying any pending refill.
    pub fn available_tokens(&self) -> f64 {
        self.refill()
    }

    /// Maximum tokens (burst capacity). Read-only accessor.
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// Refill rate in tokens per second. Read-only accessor.
    pub fn refill_rate(&self) -> f64 {
        self.refill_rate
    }
}

/// Fixed-size time-bucketed counter for sliding-window rate limiting.
/// Uses O(bucket_count) memory and per-operation time, independent of request volume.
struct BucketedWindow {
    /// Each bucket: (bucket_start_ms, count). Index = (timestamp_ms / bucket_ms) % bucket_count.
    buckets: Vec<(u64, u64)>,
    bucket_ms: u64,
    bucket_count: u64,
    window_ms: u64,
    epoch: Instant,
}

impl BucketedWindow {
    fn new(window_ms: u64) -> Self {
        let bucket_ms = 1000.min(window_ms);
        let bucket_count = (window_ms / bucket_ms).max(1);
        Self {
            buckets: vec![(0u64, 0u64); bucket_count as usize],
            bucket_ms,
            bucket_count,
            window_ms,
            epoch: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn add(&mut self, count: u64) {
        let now = self.now_ms();
        let bucket_ts = (now / self.bucket_ms) * self.bucket_ms;
        let idx = ((now / self.bucket_ms) % self.bucket_count) as usize;
        if self.buckets[idx].0 == bucket_ts {
            // Same bucket — accumulate.
            self.buckets[idx].1 += count;
        } else {
            // Stale bucket — overwrite.
            self.buckets[idx] = (bucket_ts, count);
        }
    }

    /// Sum counts from buckets whose start time falls within the active window.
    /// O(bucket_count) — independent of request volume.
    fn current_total(&self) -> u64 {
        let now = self.now_ms();
        let cutoff = now.saturating_sub(self.window_ms);
        self.buckets
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .map(|(_, c)| c)
            .sum()
    }

    fn check_and_add(&self, count: u64, limit: u64) -> bool {
        self.current_total() + count <= limit
    }
}

/// Per-channel rate limit configuration.
struct ChannelLimits {
    tpm: Option<u64>,
    rpm: u64,
}

impl Default for ChannelLimits {
    fn default() -> Self {
        Self { tpm: None, rpm: 60 }
    }
}

/// Per-channel bucketed windows for TPM and RPM.
struct ChannelWindows {
    tpm: BucketedWindow,
    rpm: BucketedWindow,
}

impl ChannelWindows {
    fn new() -> Self {
        Self {
            tpm: BucketedWindow::new(WINDOW_MS),
            rpm: BucketedWindow::new(WINDOW_MS),
        }
    }
}

/// Per-channel state behind its own independent Mutex.
/// Each channel gets an `Arc<Mutex<PerChannelState>>` so that operations
/// on one channel never block operations on another.
struct PerChannelState {
    windows: ChannelWindows,
    limits: ChannelLimits,
}

/// Per-channel rate limiting for tokens per minute (TPM) and requests per minute (RPM).
///
/// Each channel's state is behind an independent `Mutex` to eliminate contention
/// between unrelated channels. The outer `RwLock` on the `HashMap` is only held
/// briefly to look up or create a channel's `Arc`, then dropped before the
/// per-channel mutex is acquired. Global TPM tracking has its own separate lock.
pub struct RateLimiter {
    /// Channel lookup/creation only — briefly held under read lock.
    channels: RwLock<HashMap<Uuid, Arc<Mutex<PerChannelState>>>>,
    /// Global TPM window — independent lock, never blocks per-channel ops.
    global_tpm: Mutex<BucketedWindow>,
    global_tpm_limit: Option<u64>,
}

impl RateLimiter {
    pub fn new(global_tpm_limit: Option<u64>) -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            global_tpm: Mutex::new(BucketedWindow::new(WINDOW_MS)),
            global_tpm_limit,
        }
    }

    /// Get or create the `Arc<Mutex<PerChannelState>>` for a channel.
    /// Acquires a write lock on the channels map only if creation is needed.
    fn get_or_create_channel(&self, channel_id: Uuid) -> Arc<Mutex<PerChannelState>> {
        // Fast path: read lock only
        {
            let map = self.channels.read();
            if let Some(arc) = map.get(&channel_id) {
                return Arc::clone(arc);
            }
        }
        // Slow path: write lock to create entry
        let mut map = self.channels.write();
        // Double-check after acquiring write lock (another thread may have created it)
        map.entry(channel_id)
            .or_insert_with(|| {
                Arc::new(Mutex::new(PerChannelState {
                    windows: ChannelWindows::new(),
                    limits: ChannelLimits::default(),
                }))
            })
            .clone()
    }

    pub fn set_channel_tpm_limit(&self, channel_id: Uuid, limit: u64) {
        let arc = self.get_or_create_channel(channel_id);
        let mut state = arc.lock();
        state.limits.tpm = Some(limit);
    }

    pub fn set_channel_rpm_limit(&self, channel_id: Uuid, limit: u64) {
        let arc = self.get_or_create_channel(channel_id);
        let mut state = arc.lock();
        state.limits.rpm = limit;
    }

    /// Check if a request with the given estimated token count is allowed.
    /// Returns (allowed, reason).
    pub fn check(&self, channel_id: Uuid, estimated_tokens: u64) -> (bool, &'static str) {
        // Check global TPM first (independent lock)
        if let Some(global_limit) = self.global_tpm_limit {
            let global = self.global_tpm.lock();
            if !global.check_and_add(estimated_tokens, global_limit) {
                return (false, "global_tpm_exceeded");
            }
        }

        // Get channel Arc (brief read lock), then lock only this channel
        let arc = self.get_or_create_channel(channel_id);
        let state = arc.lock();

        // Check per-channel RPM
        if state.windows.rpm.current_total() >= state.limits.rpm {
            return (false, "channel_rpm_exceeded");
        }

        // Check per-channel TPM (only if limit is configured)
        if let Some(tpm_limit) = state.limits.tpm {
            if state.windows.tpm.current_total() + estimated_tokens > tpm_limit {
                return (false, "channel_tpm_exceeded");
            }
        }

        (true, "ok")
    }

    /// Record that a request was dispatched to a channel.
    pub fn record(&self, channel_id: Uuid, tokens: u64) {
        // Per-channel recording (independent lock)
        let arc = self.get_or_create_channel(channel_id);
        {
            let mut state = arc.lock();
            state.windows.tpm.add(tokens);
            state.windows.rpm.add(1);
        }

        // Global TPM recording (independent lock)
        if self.global_tpm_limit.is_some() {
            let mut global = self.global_tpm.lock();
            global.add(tokens);
        }
    }

    /// Get the current TPM usage for a channel (0 if no data).
    pub fn current_tpm(&self, channel_id: Uuid) -> u64 {
        let map = self.channels.read();
        if let Some(arc) = map.get(&channel_id) {
            let arc = Arc::clone(arc);
            drop(map);
            let state = arc.lock();
            state.windows.tpm.current_total()
        } else {
            0
        }
    }

    /// Get the TPM limit for a channel (None if not configured).
    pub fn tpm_limit(&self, channel_id: Uuid) -> Option<u64> {
        let map = self.channels.read();
        if let Some(arc) = map.get(&channel_id) {
            let arc = Arc::clone(arc);
            drop(map);
            let state = arc.lock();
            state.limits.tpm
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_allows_first_request() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        let (allowed, _) = limiter.check(ch_id, 1000);
        assert!(allowed);
    }

    #[test]
    fn record_tracks_requests() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        limiter.set_channel_rpm_limit(ch_id, 2);
        limiter.record(ch_id, 100);
        assert!(limiter.check(ch_id, 10).0);
        limiter.record(ch_id, 100);
        // Now at RPM limit (2), next check should fail
        assert!(!limiter.check(ch_id, 10).0);
    }

    #[test]
    fn global_tpm_rejects_when_exceeded() {
        let limiter = RateLimiter::new(Some(100));
        let ch_id = Uuid::new_v4();
        limiter.record(ch_id, 90);
        let (allowed, reason) = limiter.check(ch_id, 20);
        assert!(!allowed);
        assert_eq!(reason, "global_tpm_exceeded");
    }

    #[test]
    fn channel_rpm_uses_configured_limit() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        limiter.set_channel_rpm_limit(ch_id, 2);
        limiter.record(ch_id, 100);
        limiter.record(ch_id, 100);
        let (allowed, reason) = limiter.check(ch_id, 10);
        assert!(!allowed);
        assert_eq!(reason, "channel_rpm_exceeded");
    }

    #[test]
    fn channel_tpm_enforced_when_configured() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        limiter.set_channel_tpm_limit(ch_id, 1000);
        limiter.record(ch_id, 900);
        let (allowed, reason) = limiter.check(ch_id, 200);
        assert!(!allowed);
        assert_eq!(reason, "channel_tpm_exceeded");
    }

    #[test]
    fn window_prunes_old_entries() {
        let mut window = BucketedWindow::new(100); // 100ms window
        window.add(10);
        assert_eq!(window.current_total(), 10);
        window.add(20);
        assert_eq!(window.current_total(), 30);
    }

    #[test]
    fn current_tpm_reflects_recorded_usage() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        assert_eq!(limiter.current_tpm(ch_id), 0);
        limiter.record(ch_id, 500);
        limiter.record(ch_id, 300);
        assert_eq!(limiter.current_tpm(ch_id), 800);
    }

    #[test]
    fn tpm_limit_returns_configured_value() {
        let limiter = RateLimiter::new(None);
        let ch_id = Uuid::new_v4();
        assert_eq!(limiter.tpm_limit(ch_id), None);
        limiter.set_channel_tpm_limit(ch_id, 10_000);
        assert_eq!(limiter.tpm_limit(ch_id), Some(10_000));
    }

    #[test]
    fn bucketed_window_accumulates_within_same_bucket() {
        let mut window = BucketedWindow::new(60_000);
        window.add(10);
        window.add(20);
        window.add(30);
        assert_eq!(window.current_total(), 60);
    }

    #[test]
    fn bucketed_window_excludes_expired_data() {
        // Use a tiny window so data expires quickly
        let mut window = BucketedWindow::new(50); // 50ms window, bucket_ms=50, 1 bucket
        window.add(100);
        assert_eq!(window.current_total(), 100);
        std::thread::sleep(std::time::Duration::from_millis(80));
        // After window expires, old data should be excluded
        assert_eq!(window.current_total(), 0);
        // New add should work
        window.add(50);
        assert_eq!(window.current_total(), 50);
    }

    // ---------------------------------------------------------------------------
    // TokenBucket tests
    // ---------------------------------------------------------------------------

    #[test]
    fn token_bucket_allows_burst_within_capacity() {
        // capacity = 10, refill = 1/s. A burst of 5 should be allowed immediately.
        let bucket = TokenBucket::new(10.0, 1.0);
        assert!(bucket.try_consume(5.0));
        // Remaining tokens should be ~5
        let remaining = bucket.available_tokens();
        assert!(
            remaining >= 4.9 && remaining <= 5.1,
            "expected ~5 tokens remaining, got {remaining}"
        );
    }

    #[test]
    fn token_bucket_denies_when_empty() {
        // capacity = 3, refill very slow. Drain the bucket fully.
        let bucket = TokenBucket::new(3.0, 0.01);
        assert!(bucket.try_consume(3.0));
        // Bucket is now empty — next consume should be denied.
        assert!(!bucket.try_consume(1.0));
    }

    #[test]
    fn token_bucket_refills_over_time() {
        // capacity = 10, refill = 1000/s (fast for testing).
        let bucket = TokenBucket::new(10.0, 1000.0);
        // Drain the bucket
        assert!(bucket.try_consume(10.0));
        assert!(!bucket.try_consume(1.0), "bucket should be empty");
        // Wait 20ms → ~20 tokens worth of refill, but capped at capacity
        std::thread::sleep(std::time::Duration::from_millis(20));
        // Should have refilled enough to admit a request
        assert!(
            bucket.try_consume(5.0),
            "bucket should have refilled enough after sleeping"
        );
    }

    #[test]
    fn token_bucket_caps_at_capacity() {
        // capacity = 5, refill = 100/s. Even after a long idle period, the
        // bucket should never exceed capacity.
        let bucket = TokenBucket::new(5.0, 100.0);
        // Wait well beyond what's needed to fill
        std::thread::sleep(std::time::Duration::from_millis(50));
        let available = bucket.available_tokens();
        assert!(
            available <= 5.0 + 1e-9,
            "tokens should not exceed capacity, got {available}"
        );
        // Consume partial and verify the cap still holds after another wait
        assert!(bucket.try_consume(2.0));
        std::thread::sleep(std::time::Duration::from_millis(50));
        let after = bucket.available_tokens();
        assert!(
            after <= 5.0 + 1e-9,
            "tokens should still not exceed capacity after refill, got {after}"
        );
    }

    #[test]
    fn rate_limit_algorithm_serde_roundtrip() {
        // Verify serde mapping: "sliding_window" / "token_bucket"
        let sw: RateLimitAlgorithm =
            serde_json::from_str("\"sliding_window\"").expect("deserialize sliding_window");
        assert_eq!(sw, RateLimitAlgorithm::SlidingWindow);

        let tb: RateLimitAlgorithm =
            serde_json::from_str("\"token_bucket\"").expect("deserialize token_bucket");
        assert_eq!(tb, RateLimitAlgorithm::TokenBucket);

        // Serialize back
        assert_eq!(
            serde_json::to_string(&RateLimitAlgorithm::SlidingWindow).unwrap(),
            "\"sliding_window\""
        );
        assert_eq!(
            serde_json::to_string(&RateLimitAlgorithm::TokenBucket).unwrap(),
            "\"token_bucket\""
        );
    }

    #[test]
    fn rate_limit_algorithm_default_is_sliding_window() {
        assert_eq!(
            RateLimitAlgorithm::default(),
            RateLimitAlgorithm::SlidingWindow
        );
    }
}
