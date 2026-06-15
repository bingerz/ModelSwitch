use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

const WINDOW_SIZE: usize = 10; // Track last 10 samples

/// Tracks recent latency samples per channel using a sliding window.
/// Provides more responsive routing decisions than a static EMA.
pub struct LatencyTracker {
    samples: Mutex<HashMap<Uuid, Vec<u64>>>,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(HashMap::new()),
        }
    }

    /// Record a latency sample for a channel.
    pub fn record(&self, channel_id: Uuid, latency_ms: u64) {
        let mut guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        let samples = guard
            .entry(channel_id)
            .or_insert_with(|| Vec::with_capacity(WINDOW_SIZE + 1));
        samples.push(latency_ms);
        if samples.len() > WINDOW_SIZE {
            samples.remove(0); // Remove oldest
        }
    }

    /// Get the average latency from recent samples for a channel.
    /// Returns 0 if no samples exist.
    pub fn avg_latency(&self, channel_id: Uuid) -> u64 {
        let guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(&channel_id)
            .map(|samples| {
                if samples.is_empty() {
                    0
                } else {
                    samples.iter().sum::<u64>() / samples.len() as u64
                }
            })
            .unwrap_or(0)
    }

    /// Get the p95 latency from recent samples for a channel.
    pub fn p95_latency(&self, channel_id: Uuid) -> u64 {
        let guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(&channel_id)
            .map(|samples| {
                if samples.is_empty() {
                    return 0;
                }
                let mut sorted = samples.clone();
                sorted.sort_unstable();
                let idx = (sorted.len() as f64 * 0.95).ceil() as usize;
                sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
            })
            .unwrap_or(0)
    }
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avg_latency_returns_zero_without_samples() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        assert_eq!(tracker.avg_latency(id), 0);
    }

    #[test]
    fn avg_latency_computes_average() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        tracker.record(id, 100);
        tracker.record(id, 200);
        tracker.record(id, 300);
        assert_eq!(tracker.avg_latency(id), 200);
    }

    #[test]
    fn sliding_window_caps_samples() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        for i in 1..=15 {
            tracker.record(id, i * 10);
        }
        // Only last 10 samples (60..150) should be kept: avg = (60+70+...+150)/10 = 105
        assert_eq!(tracker.avg_latency(id), 105);
    }

    #[test]
    fn p95_latency_picks_percentile() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        for i in 1..=10 {
            tracker.record(id, i * 100); // 100, 200, ..., 1000
        }
        // 10 samples, p95 index = ceil(10 * 0.95) = 10, idx-1 = 9 -> 1000
        assert_eq!(tracker.p95_latency(id), 1000);
    }

    #[test]
    fn multiple_channels_are_independent() {
        let tracker = LatencyTracker::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        tracker.record(id_a, 100);
        tracker.record(id_b, 500);
        assert_eq!(tracker.avg_latency(id_a), 100);
        assert_eq!(tracker.avg_latency(id_b), 500);
    }
}
