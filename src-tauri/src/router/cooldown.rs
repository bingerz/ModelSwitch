use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Default failure rate threshold (30%)
const DEFAULT_FAILURE_THRESHOLD: f64 = 0.30;
/// Minimum attempts before failure rate is evaluated
const MIN_ATTEMPTS: usize = 5;
/// Default cooldown duration
const DEFAULT_COOLDOWN_SECS: u64 = 60;
/// Rolling window size (number of recent attempts to track)
const WINDOW_SIZE: usize = 20;

#[derive(Debug, Clone)]
struct AttemptRecord {
    success: bool,
    at: Instant,
}

/// Tracks recent attempts per channel and computes failure rates.
/// When failure rate exceeds threshold with sufficient samples,
/// the channel enters cooldown.
pub struct CooldownTracker {
    records: Mutex<HashMap<Uuid, VecDeque<AttemptRecord>>>,
    cooldowns: Mutex<HashMap<Uuid, Instant>>,
    threshold: f64,
    min_attempts: usize,
    cooldown_duration: Duration,
}

impl CooldownTracker {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            threshold: DEFAULT_FAILURE_THRESHOLD,
            min_attempts: MIN_ATTEMPTS,
            cooldown_duration: Duration::from_secs(DEFAULT_COOLDOWN_SECS),
        }
    }

    /// Record an attempt result for a channel.
    /// `success` = true for 2xx, false for 5xx/429/connection errors.
    /// 4xx client errors (except 429) should NOT be recorded (caller decides).
    pub fn record_attempt(&self, channel_id: Uuid, success: bool) {
        let mut records = self.records.lock().unwrap();
        let queue = records.entry(channel_id).or_default();
        queue.push_back(AttemptRecord {
            success,
            at: Instant::now(),
        });
        if queue.len() > WINDOW_SIZE {
            queue.pop_front();
        }
        drop(records);

        if !success {
            self.evaluate_cooldown(channel_id);
        } else {
            // Clear cooldown on success
            let mut cooldowns = self.cooldowns.lock().unwrap();
            cooldowns.remove(&channel_id);
        }
    }

    /// Check if a channel is currently in cooldown.
    pub fn is_in_cooldown(&self, channel_id: Uuid) -> bool {
        let cooldowns = self.cooldowns.lock().unwrap();
        if let Some(until) = cooldowns.get(&channel_id) {
            if *until > Instant::now() {
                return true;
            }
        }
        false
    }

    /// Get remaining cooldown seconds (0 if not in cooldown).
    pub fn cooldown_remaining_secs(&self, channel_id: Uuid) -> u64 {
        let cooldowns = self.cooldowns.lock().unwrap();
        if let Some(until) = cooldowns.get(&channel_id) {
            let remaining = until.saturating_duration_since(Instant::now());
            return remaining.as_secs();
        }
        0
    }

    /// Evaluate whether a channel should enter cooldown based on failure rate.
    fn evaluate_cooldown(&self, channel_id: Uuid) {
        let records = self.records.lock().unwrap();
        if let Some(queue) = records.get(&channel_id) {
            if queue.len() < self.min_attempts {
                return;
            }
            let failures = queue.iter().filter(|r| !r.success).count();
            let failure_rate = failures as f64 / queue.len() as f64;
            if failure_rate >= self.threshold {
                drop(records);
                let mut cooldowns = self.cooldowns.lock().unwrap();
                cooldowns.insert(channel_id, Instant::now() + self.cooldown_duration);
                tracing::warn!(
                    channel_id = %channel_id,
                    failure_rate = failure_rate,
                    threshold = self.threshold,
                    "Channel entered cooldown due to high failure rate"
                );
            }
        }
    }

    /// Remove a channel from tracking (e.g., when channel is deleted).
    pub fn remove(&self, channel_id: Uuid) {
        self.records.lock().unwrap().remove(&channel_id);
        self.cooldowns.lock().unwrap().remove(&channel_id);
    }
}

impl Default for CooldownTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cooldown_below_min_attempts() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        for _ in 0..4 {
            tracker.record_attempt(id, false);
        }
        assert!(
            !tracker.is_in_cooldown(id),
            "Should not cooldown with < min_attempts"
        );
    }

    #[test]
    fn cooldown_after_threshold_failures() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        // Record 5 failures out of 5 = 100% failure rate > 30%
        for _ in 0..5 {
            tracker.record_attempt(id, false);
        }
        assert!(
            tracker.is_in_cooldown(id),
            "Should cooldown at 100% failure rate"
        );
    }

    #[test]
    fn no_cooldown_with_low_failure_rate() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        // 1 failure out of 5 = 20% < 30%
        tracker.record_attempt(id, false);
        for _ in 0..4 {
            tracker.record_attempt(id, true);
        }
        assert!(
            !tracker.is_in_cooldown(id),
            "Should not cooldown at 20% failure rate"
        );
    }

    #[test]
    fn success_clears_cooldown() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        for _ in 0..5 {
            tracker.record_attempt(id, false);
        }
        assert!(tracker.is_in_cooldown(id));
        tracker.record_attempt(id, true);
        assert!(!tracker.is_in_cooldown(id), "Success should clear cooldown");
    }

    #[test]
    fn rolling_window_evicts_old_entries() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        // Fill with 20 failures (100% rate), then add 20 successes
        for _ in 0..20 {
            tracker.record_attempt(id, false);
        }
        assert!(tracker.is_in_cooldown(id));
        for _ in 0..20 {
            tracker.record_attempt(id, true);
        }
        // All failures evicted from window, 100% success rate
        assert!(!tracker.is_in_cooldown(id));
    }

    #[test]
    fn remove_clears_tracking() {
        let tracker = CooldownTracker::new();
        let id = Uuid::new_v4();
        for _ in 0..5 {
            tracker.record_attempt(id, false);
        }
        tracker.remove(id);
        assert!(!tracker.is_in_cooldown(id));
    }
}
