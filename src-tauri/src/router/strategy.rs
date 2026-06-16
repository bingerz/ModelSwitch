use crate::channel::Channel;
use crate::proxy::rate_limiter::RateLimiter;
use crate::router::active_requests::ActiveRequests;
use crate::router::latency_tracker::LatencyTracker;
use rand::Rng;

/// Strategy for selecting a channel from a list of candidates.
pub trait RoutingStrategy: Send + Sync {
    fn select(&self, candidates: &[Channel]) -> Option<Channel>;
}

/// Weighted random selection (default strategy).
pub struct WeightedRandomStrategy;

impl RoutingStrategy for WeightedRandomStrategy {
    fn select(&self, candidates: &[Channel]) -> Option<Channel> {
        super::weighted::weighted_random(candidates)
    }
}

/// Latency-based selection: prefer channels with lower average latency.
/// Uses the sliding-window LatencyTracker for responsive decisions.
/// Falls back to weighted random if no latency data is available.
pub struct LatencyBasedStrategy {
    /// Maximum number of "fast" candidates to random-pick from (top N by latency)
    pub top_k: usize,
    latency_tracker: std::sync::Arc<LatencyTracker>,
}

impl LatencyBasedStrategy {
    pub fn new(latency_tracker: std::sync::Arc<LatencyTracker>) -> Self {
        Self {
            top_k: 2,
            latency_tracker,
        }
    }
}

impl RoutingStrategy for LatencyBasedStrategy {
    fn select(&self, candidates: &[Channel]) -> Option<Channel> {
        // Separate candidates into those with latency data and those without
        let mut with_latency: Vec<(&Channel, u64)> = Vec::new();
        let mut without_latency: Vec<&Channel> = Vec::new();

        for c in candidates {
            let lat = self.latency_tracker.avg_latency(c.id);
            if lat > 0 {
                with_latency.push((c, lat));
            } else {
                without_latency.push(c);
            }
        }

        // If we have latency data, sort by latency and pick from top K
        if !with_latency.is_empty() {
            with_latency.sort_by_key(|(_, lat)| *lat);
            let top = &with_latency[..self.top_k.min(with_latency.len())];
            let idx = rand::rng().random_range(0..top.len());
            return Some(top[idx].0.clone());
        }

        // No latency data — fall back to weighted random
        super::weighted::weighted_random(candidates)
    }
}

/// Least-busy selection: pick the channel with the fewest active requests.
/// Falls back to weighted random if all channels have equal active counts.
pub struct LeastBusyStrategy {
    active_requests: std::sync::Arc<ActiveRequests>,
}

impl LeastBusyStrategy {
    pub fn new(active_requests: std::sync::Arc<ActiveRequests>) -> Self {
        Self { active_requests }
    }
}

impl RoutingStrategy for LeastBusyStrategy {
    fn select(&self, candidates: &[Channel]) -> Option<Channel> {
        if candidates.is_empty() {
            return None;
        }

        let mut min_count = u32::MAX;
        let mut least_busy: Vec<&Channel> = Vec::new();

        for c in candidates {
            let count = self.active_requests.get(c.id);
            if count < min_count {
                min_count = count;
                least_busy.clear();
                least_busy.push(c);
            } else if count == min_count {
                least_busy.push(c);
            }
        }

        // All channels equally busy — fall back to weighted random
        if least_busy.len() == candidates.len() {
            return super::weighted::weighted_random(candidates);
        }

        let idx = rand::rng().random_range(0..least_busy.len());
        Some(least_busy[idx].clone())
    }
}

/// Usage-based selection: prefer channels with the lowest TPM utilization ratio.
/// Channels without a TPM limit are treated as fully available (ratio 0).
/// Picks randomly from the top 2 least-utilized to avoid thundering herd.
pub struct UsageBasedStrategy {
    rate_limiter: std::sync::Arc<RateLimiter>,
}

impl UsageBasedStrategy {
    pub fn new(rate_limiter: std::sync::Arc<RateLimiter>) -> Self {
        Self { rate_limiter }
    }
}

impl RoutingStrategy for UsageBasedStrategy {
    fn select(&self, candidates: &[Channel]) -> Option<Channel> {
        if candidates.is_empty() {
            return None;
        }

        // Score each candidate by TPM utilization ratio (current_tpm / tpm_limit)
        // Lower ratio = less utilized = preferred
        // Channels without TPM limit get ratio of 0 (always preferred)
        let mut scored: Vec<(&Channel, f64)> = candidates
            .iter()
            .map(|c| {
                let current = self.rate_limiter.current_tpm(c.id);
                let limit = self.rate_limiter.tpm_limit(c.id);
                let ratio = match limit {
                    Some(lim) if lim > 0 => current as f64 / lim as f64,
                    _ => 0.0, // No limit = treat as fully available
                };
                (c, ratio)
            })
            .collect();

        // Sort by utilization ratio ascending (lowest utilization first)
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Pick from top 2 (lowest utilization) randomly to avoid thundering herd
        let top_k = scored.len().min(2);
        let idx = rand::rng().random_range(0..top_k);
        Some(scored[idx].0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    fn make_channel(name: &str, priority: u8, weight: u32, avg_latency: u64) -> Channel {
        Channel {
            id: Uuid::new_v4(),
            name: name.to_string(),
            provider: Provider::OpenAI,
            priority,
            weight,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: format!("test-{}", name),
                api_key: None,
                expires_at: None,
            },
            enabled: true,
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            avg_latency_ms: avg_latency,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            failure_window_start: None,
            window_failure_count: 0,
            max_concurrent: None,
            api_keys: vec![],
        }
    }

    #[test]
    fn latency_strategy_prefers_fast_channels() {
        let tracker = Arc::new(LatencyTracker::new());
        let fast = make_channel("fast", 1, 100, 100);
        let slow = make_channel("slow", 1, 100, 5000);

        // Record latency data in the tracker
        tracker.record(fast.id, 100);
        tracker.record(slow.id, 5000);

        let candidates = vec![slow.clone(), fast.clone()];

        let strategy = LatencyBasedStrategy::new(Arc::clone(&tracker));
        // With only top_k=2, both are candidates. Run many times to check bias.
        let mut fast_count = 0;
        for _ in 0..100 {
            let selected = strategy.select(&candidates).unwrap();
            if selected.avg_latency_ms == 100 {
                fast_count += 1;
            }
        }
        // Should pick fast channel roughly 50%+ of the time (both in top_k)
        assert!(fast_count > 20);
    }

    #[test]
    fn latency_strategy_falls_back_without_data() {
        let tracker = Arc::new(LatencyTracker::new());
        let ch1 = make_channel("ch1", 1, 100, 0);
        let ch2 = make_channel("ch2", 1, 100, 0);
        let candidates = vec![ch1, ch2];

        let strategy = LatencyBasedStrategy::new(tracker);
        let selected = strategy.select(&candidates);
        assert!(selected.is_some());
    }

    #[test]
    fn least_busy_prefers_least_loaded() {
        let busy = make_channel("busy", 1, 100, 100);
        let idle = make_channel("idle", 1, 100, 200);
        let active = Arc::new(ActiveRequests::new());
        // Simulate active requests on busy channel
        active.increment(busy.id);
        active.increment(busy.id);
        active.increment(busy.id);

        let candidates = vec![busy.clone(), idle.clone()];
        let strategy = LeastBusyStrategy::new(active);
        // Should always pick idle (0 active vs 3)
        for _ in 0..10 {
            let selected = strategy.select(&candidates).unwrap();
            assert_eq!(selected.id, idle.id);
        }
    }

    #[test]
    fn least_busy_falls_back_on_equal_load() {
        let ch1 = make_channel("ch1", 1, 100, 0);
        let ch2 = make_channel("ch2", 1, 100, 0);
        let active = Arc::new(ActiveRequests::new());
        // Same load on both — should fall back to weighted random
        active.increment(ch1.id);
        active.increment(ch2.id);

        let candidates = vec![ch1, ch2];
        let strategy = LeastBusyStrategy::new(active);
        let mut ch1_count = 0;
        for _ in 0..100 {
            let selected = strategy.select(&candidates).unwrap();
            if selected.name == "ch1" {
                ch1_count += 1;
            }
        }
        // Should be roughly 50/50
        assert!(ch1_count > 20 && ch1_count < 80);
    }

    #[test]
    fn usage_based_prefers_lower_utilization() {
        let limiter = Arc::new(RateLimiter::new(None));
        let ch_high = make_channel("high", 1, 100, 0);
        let ch_mid = make_channel("mid", 1, 100, 0);
        let ch_low = make_channel("low", 1, 100, 0);

        // Set TPM limits
        limiter.set_channel_tpm_limit(ch_high.id, 10_000);
        limiter.set_channel_tpm_limit(ch_mid.id, 10_000);
        limiter.set_channel_tpm_limit(ch_low.id, 10_000);

        // Record usage — high is 80%, mid is 50%, low is 10%
        limiter.record(ch_high.id, 8_000);
        limiter.record(ch_mid.id, 5_000);
        limiter.record(ch_low.id, 1_000);

        let candidates = vec![ch_high.clone(), ch_mid.clone(), ch_low.clone()];
        let strategy = UsageBasedStrategy::new(limiter);

        // top_k = 2, so the highest (80%) should never be picked
        for _ in 0..50 {
            let selected = strategy.select(&candidates).unwrap();
            assert!(
                selected.id == ch_low.id || selected.id == ch_mid.id,
                "should never pick the highest-utilization channel"
            );
        }
    }

    #[test]
    fn usage_based_treats_no_limit_as_available() {
        let limiter = Arc::new(RateLimiter::new(None));
        let ch_with_limit = make_channel("limited", 1, 100, 0);
        let ch_with_limit_2 = make_channel("limited2", 1, 100, 0);
        let ch_no_limit = make_channel("unlimited", 1, 100, 0);

        // Set limits on two channels
        limiter.set_channel_tpm_limit(ch_with_limit.id, 1_000);
        limiter.record(ch_with_limit.id, 900); // 90% utilized
        limiter.set_channel_tpm_limit(ch_with_limit_2.id, 1_000);
        limiter.record(ch_with_limit_2.id, 800); // 80% utilized

        // No limit on ch_no_limit — ratio is 0 (preferred)
        let candidates = vec![
            ch_with_limit.clone(),
            ch_with_limit_2.clone(),
            ch_no_limit.clone(),
        ];
        let strategy = UsageBasedStrategy::new(limiter);

        // top_k = 2, so both the no-limit (ratio 0) and 80% channels are eligible.
        // The 90% channel should never be picked.
        for _ in 0..50 {
            let selected = strategy.select(&candidates).unwrap();
            assert!(
                selected.id == ch_no_limit.id || selected.id == ch_with_limit_2.id,
                "should never pick the 90%-utilized channel"
            );
        }
    }

    #[test]
    fn usage_based_returns_none_for_empty() {
        let limiter = Arc::new(RateLimiter::new(None));
        let strategy = UsageBasedStrategy::new(limiter);
        let candidates: Vec<Channel> = vec![];
        assert!(strategy.select(&candidates).is_none());
    }
}
