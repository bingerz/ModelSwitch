use std::collections::HashMap;
use std::sync::Mutex;
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

/// Internal state behind a single Mutex.
struct RateLimiterState {
    channels: HashMap<Uuid, (ChannelWindows, ChannelLimits)>,
    global_tpm_window: BucketedWindow,
}

/// Per-channel rate limiting for tokens per minute (TPM) and requests per minute (RPM).
/// All state is behind a single Mutex to avoid multi-lock overhead.
pub struct RateLimiter {
    state: Mutex<RateLimiterState>,
    global_tpm_limit: Option<u64>,
}

impl RateLimiter {
    pub fn new(global_tpm_limit: Option<u64>) -> Self {
        Self {
            state: Mutex::new(RateLimiterState {
                channels: HashMap::new(),
                global_tpm_window: BucketedWindow::new(WINDOW_MS),
            }),
            global_tpm_limit,
        }
    }

    pub fn set_channel_tpm_limit(&self, channel_id: Uuid, limit: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .channels
            .entry(channel_id)
            .or_insert_with(|| (ChannelWindows::new(), ChannelLimits::default()));
        entry.1.tpm = Some(limit);
    }

    pub fn set_channel_rpm_limit(&self, channel_id: Uuid, limit: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .channels
            .entry(channel_id)
            .or_insert_with(|| (ChannelWindows::new(), ChannelLimits::default()));
        entry.1.rpm = limit;
    }

    /// Check if a request with the given estimated token count is allowed.
    /// Returns (allowed, reason).
    pub fn check(&self, channel_id: Uuid, estimated_tokens: u64) -> (bool, &'static str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Check global TPM
        if let Some(global_limit) = self.global_tpm_limit {
            if !state
                .global_tpm_window
                .check_and_add(estimated_tokens, global_limit)
            {
                return (false, "global_tpm_exceeded");
            }
        }

        // Get or create channel entry
        let entry = state
            .channels
            .entry(channel_id)
            .or_insert_with(|| (ChannelWindows::new(), ChannelLimits::default()));

        // Check per-channel RPM
        if entry.0.rpm.current_total() >= entry.1.rpm {
            return (false, "channel_rpm_exceeded");
        }

        // Check per-channel TPM (only if limit is configured)
        if let Some(tpm_limit) = entry.1.tpm {
            if entry.0.tpm.current_total() + estimated_tokens > tpm_limit {
                return (false, "channel_tpm_exceeded");
            }
        }

        (true, "ok")
    }

    /// Record that a request was dispatched to a channel.
    pub fn record(&self, channel_id: Uuid, tokens: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        let entry = state
            .channels
            .entry(channel_id)
            .or_insert_with(|| (ChannelWindows::new(), ChannelLimits::default()));
        entry.0.tpm.add(tokens);
        entry.0.rpm.add(1);

        if self.global_tpm_limit.is_some() {
            state.global_tpm_window.add(tokens);
        }
    }

    /// Get the current TPM usage for a channel (0 if no data).
    pub fn current_tpm(&self, channel_id: Uuid) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .channels
            .get(&channel_id)
            .map(|(windows, _)| windows.tpm.current_total())
            .unwrap_or(0)
    }

    /// Get the TPM limit for a channel (None if not configured).
    pub fn tpm_limit(&self, channel_id: Uuid) -> Option<u64> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .channels
            .get(&channel_id)
            .and_then(|(_, limits)| limits.tpm)
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
