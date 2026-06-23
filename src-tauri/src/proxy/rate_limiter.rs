use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

const WINDOW_MS: u64 = 60_000; // 1 minute

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
            let map = self.channels.read().unwrap_or_else(|e| e.into_inner());
            if let Some(arc) = map.get(&channel_id) {
                return Arc::clone(arc);
            }
        }
        // Slow path: write lock to create entry
        let mut map = self.channels.write().unwrap_or_else(|e| e.into_inner());
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
        let mut state = arc.lock().unwrap_or_else(|e| e.into_inner());
        state.limits.tpm = Some(limit);
    }

    pub fn set_channel_rpm_limit(&self, channel_id: Uuid, limit: u64) {
        let arc = self.get_or_create_channel(channel_id);
        let mut state = arc.lock().unwrap_or_else(|e| e.into_inner());
        state.limits.rpm = limit;
    }

    /// Check if a request with the given estimated token count is allowed.
    /// Returns (allowed, reason).
    pub fn check(&self, channel_id: Uuid, estimated_tokens: u64) -> (bool, &'static str) {
        // Check global TPM first (independent lock)
        if let Some(global_limit) = self.global_tpm_limit {
            let global = self.global_tpm.lock().unwrap_or_else(|e| e.into_inner());
            if !global.check_and_add(estimated_tokens, global_limit) {
                return (false, "global_tpm_exceeded");
            }
        }

        // Get channel Arc (brief read lock), then lock only this channel
        let arc = self.get_or_create_channel(channel_id);
        let state = arc.lock().unwrap_or_else(|e| e.into_inner());

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
            let mut state = arc.lock().unwrap_or_else(|e| e.into_inner());
            state.windows.tpm.add(tokens);
            state.windows.rpm.add(1);
        }

        // Global TPM recording (independent lock)
        if self.global_tpm_limit.is_some() {
            let mut global = self.global_tpm.lock().unwrap_or_else(|e| e.into_inner());
            global.add(tokens);
        }
    }

    /// Get the current TPM usage for a channel (0 if no data).
    pub fn current_tpm(&self, channel_id: Uuid) -> u64 {
        let map = self.channels.read().unwrap_or_else(|e| e.into_inner());
        if let Some(arc) = map.get(&channel_id) {
            let arc = Arc::clone(arc);
            drop(map);
            let state = arc.lock().unwrap_or_else(|e| e.into_inner());
            state.windows.tpm.current_total()
        } else {
            0
        }
    }

    /// Get the TPM limit for a channel (None if not configured).
    pub fn tpm_limit(&self, channel_id: Uuid) -> Option<u64> {
        let map = self.channels.read().unwrap_or_else(|e| e.into_inner());
        if let Some(arc) = map.get(&channel_id) {
            let arc = Arc::clone(arc);
            drop(map);
            let state = arc.lock().unwrap_or_else(|e| e.into_inner());
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
}
