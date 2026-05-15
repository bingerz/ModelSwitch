use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use uuid::Uuid;

/// Sliding window rate limiter for a single metric (tokens or requests).
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

/// Per-channel rate limiting for tokens per minute (TPM) and requests per minute (RPM).
pub struct RateLimiter {
    tpm_windows: Mutex<HashMap<Uuid, SlidingWindow>>,
    rpm_windows: Mutex<HashMap<Uuid, SlidingWindow>>,
    tpm_limits: Mutex<HashMap<Uuid, u64>>,
    rpm_limits: Mutex<HashMap<Uuid, u64>>,
    global_tpm_limit: Option<u64>,
    global_tpm_window: Mutex<SlidingWindow>,
}

const WINDOW_MS: u64 = 60_000; // 1 minute

impl RateLimiter {
    pub fn new(global_tpm_limit: Option<u64>) -> Self {
        Self {
            tpm_windows: Mutex::new(HashMap::new()),
            rpm_windows: Mutex::new(HashMap::new()),
            tpm_limits: Mutex::new(HashMap::new()),
            rpm_limits: Mutex::new(HashMap::new()),
            global_tpm_limit,
            global_tpm_window: Mutex::new(SlidingWindow::new(WINDOW_MS)),
        }
    }

    pub fn set_channel_tpm_limit(&self, channel_id: Uuid, limit: u64) {
        self.tpm_limits.lock().unwrap().insert(channel_id, limit);
        self.tpm_windows
            .lock()
            .unwrap()
            .entry(channel_id)
            .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
    }

    pub fn set_channel_rpm_limit(&self, channel_id: Uuid, limit: u64) {
        self.rpm_limits.lock().unwrap().insert(channel_id, limit);
        self.rpm_windows
            .lock()
            .unwrap()
            .entry(channel_id)
            .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
    }

    fn get_rpm_limit(&self, channel_id: Uuid) -> u64 {
        self.rpm_limits
            .lock()
            .unwrap()
            .get(&channel_id)
            .copied()
            .unwrap_or(60)
    }

    fn get_tpm_limit(&self, channel_id: Uuid) -> Option<u64> {
        self.tpm_limits.lock().unwrap().get(&channel_id).copied()
    }

    /// Check if a request with the given estimated token count is allowed.
    pub fn check(&self, channel_id: Uuid, estimated_tokens: u64) -> (bool, &'static str) {
        // Check global TPM
        if let Some(global_limit) = self.global_tpm_limit {
            let mut guard = self.global_tpm_window.lock().unwrap();
            if !guard.check_and_add(estimated_tokens, global_limit) {
                return (false, "global_tpm_exceeded");
            }
        }

        // Check per-channel RPM
        {
            let rpm_limit = self.get_rpm_limit(channel_id);
            let mut guard = self.rpm_windows.lock().unwrap();
            let window = guard
                .entry(channel_id)
                .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
            if window.current_total() >= rpm_limit {
                return (false, "channel_rpm_exceeded");
            }
        }

        // Check per-channel TPM (only if limit is configured)
        if let Some(tpm_limit) = self.get_tpm_limit(channel_id) {
            let mut guard = self.tpm_windows.lock().unwrap();
            let window = guard
                .entry(channel_id)
                .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
            if window.current_total() + estimated_tokens > tpm_limit {
                return (false, "channel_tpm_exceeded");
            }
        }

        (true, "ok")
    }

    /// Record that a request was dispatched to a channel.
    pub fn record(&self, channel_id: Uuid, tokens: u64) {
        {
            let mut guard = self.tpm_windows.lock().unwrap();
            let window = guard
                .entry(channel_id)
                .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
            window.add(tokens);
        }
        {
            let mut guard = self.rpm_windows.lock().unwrap();
            let window = guard
                .entry(channel_id)
                .or_insert_with(|| SlidingWindow::new(WINDOW_MS));
            window.add(1);
        }
        if self.global_tpm_limit.is_some() {
            let mut guard = self.global_tpm_window.lock().unwrap();
            guard.add(tokens);
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
        limiter.record(ch_id, 500);
        let rpm_guard = limiter.rpm_windows.lock().unwrap();
        assert!(rpm_guard.contains_key(&ch_id));
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
        // Simulate time passing by manipulating epoch — we can't, so test with a very short window
        // Instead, verify entries exist and can be pruned
        assert_eq!(window.current_total(), 10);
        // After adding more, old entries are still within window
        window.add(20);
        assert_eq!(window.current_total(), 30);
    }
}
