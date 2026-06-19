use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

const WINDOW_SIZE: usize = 10; // Track last 10 samples

/// A single latency sample: `(latency_ms, output_tokens)`.
/// `output_tokens` is `None` when token data is unavailable (e.g. TTFB-only
/// measurements from the streaming path).
type LatencySample = (u64, Option<u64>);

/// Tracks recent latency samples per channel using a sliding window.
/// Provides more responsive routing decisions than a static EMA.
pub struct LatencyTracker {
    samples: Mutex<HashMap<Uuid, Vec<LatencySample>>>,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(HashMap::new()),
        }
    }

    /// Record a latency sample for a channel.
    /// Backwards-compatible entrypoint that records the sample without
    /// output-token data (per-token normalization will not be available
    /// for samples recorded this way).
    pub fn record(&self, channel_id: Uuid, latency_ms: u64) {
        self.record_with_tokens(channel_id, latency_ms, None);
    }

    /// Record a latency sample with output token count for per-token normalization.
    pub fn record_with_tokens(
        &self,
        channel_id: Uuid,
        latency_ms: u64,
        output_tokens: Option<u64>,
    ) {
        let mut guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        let samples = guard
            .entry(channel_id)
            .or_insert_with(|| Vec::with_capacity(WINDOW_SIZE + 1));
        samples.push((latency_ms, output_tokens));
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
                    samples.iter().map(|(lat, _)| lat).sum::<u64>() / samples.len() as u64
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
                let mut sorted: Vec<u64> = samples.iter().map(|(lat, _)| *lat).collect();
                sorted.sort_unstable();
                let idx = (sorted.len() as f64 * 0.95).ceil() as usize;
                sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
            })
            .unwrap_or(0)
    }

    /// Average latency per output token (milliseconds per token).
    /// Returns None if no samples have token data.
    /// Computes total_latency / total_tokens across all samples that have tokens.
    pub fn avg_latency_per_token(&self, channel_id: Uuid) -> Option<u64> {
        let guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(&channel_id).and_then(|samples| {
            let with_tokens: Vec<&LatencySample> = samples
                .iter()
                .filter(|(_, tokens)| tokens.is_some())
                .collect();
            if with_tokens.is_empty() {
                return None;
            }
            let total_latency: u64 = with_tokens.iter().map(|(lat, _)| *lat).sum();
            let total_tokens: u64 = with_tokens.iter().filter_map(|(_, t)| *t).sum();
            if total_tokens == 0 {
                return None;
            }
            Some(total_latency / total_tokens)
        })
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

    #[test]
    fn avg_latency_per_token_returns_none_without_token_data() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        tracker.record(id, 1000);
        assert_eq!(tracker.avg_latency_per_token(id), None);
    }

    #[test]
    fn avg_latency_per_token_computes_throughput() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        // 5000ms for 1000 tokens = 5ms/token
        tracker.record_with_tokens(id, 5000, Some(1000));
        // 3000ms for 1000 tokens = 3ms/token
        tracker.record_with_tokens(id, 3000, Some(1000));
        // Total: 8000ms / 2000 tokens = 4ms/token
        assert_eq!(tracker.avg_latency_per_token(id), Some(4));
    }

    #[test]
    fn avg_latency_per_token_ignores_samples_without_tokens() {
        let tracker = LatencyTracker::new();
        let id = Uuid::new_v4();
        tracker.record(id, 99999); // no token data — should be ignored
        tracker.record_with_tokens(id, 5000, Some(1000));
        assert_eq!(tracker.avg_latency_per_token(id), Some(5));
    }
}
