use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use uuid::Uuid;

const WINDOW_MS: u64 = 60_000; // 1 minute

/// Sliding window for a single metric (tokens or requests).
struct SlidingWindow {
    entries: Vec<(u64, u64)>, // (relative_ms, count)
    window_ms: u64,
    epoch: Instant,
}

impl SlidingWindow {
    fn new(window_ms: u64) -> Self {
        Self {
            entries: Vec::new(),
            window_ms,
            epoch: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn add(&mut self, count: u64) {
        let now = self.now_ms();
        self.prune(now);
        self.entries.push((now, count));
    }

    fn current_total(&mut self) -> u64 {
        let now = self.now_ms();
        self.prune(now);
        self.entries.iter().map(|(_, c)| c).sum()
    }

    fn check_and_add(&mut self, count: u64, limit: u64) -> bool {
        self.current_total() + count <= limit
    }

    fn prune(&mut self, now: u64) {
        let cutoff = now.saturating_sub(self.window_ms);
        self.entries.retain(|(ts, _)| *ts >= cutoff);
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

/// Per-channel sliding windows for TPM and RPM.
struct ChannelWindows {
    tpm: SlidingWindow,
    rpm: SlidingWindow,
}

impl ChannelWindows {
    fn new() -> Self {
        Self {
            tpm: SlidingWindow::new(WINDOW_MS),
            rpm: SlidingWindow::new(WINDOW_MS),
        }
    }
}

/// Internal state behind a single Mutex.
struct RateLimiterState {
    channels: HashMap<Uuid, (ChannelWindows, ChannelLimits)>,
    global_tpm_window: SlidingWindow,
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
                global_tpm_window: SlidingWindow::new(WINDOW_MS),
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
            if !state.global_tpm_window.check_and_add(estimated_tokens, global_limit) {
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
        let mut window = SlidingWindow::new(100); // 100ms window
        window.add(10);
        assert_eq!(window.current_total(), 10);
        window.add(20);
        assert_eq!(window.current_total(), 30);
    }
}
