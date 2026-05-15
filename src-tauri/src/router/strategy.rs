use crate::channel::Channel;
use crate::router::active_requests::ActiveRequests;
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
/// Falls back to weighted random if no latency data is available.
pub struct LatencyBasedStrategy {
    /// Maximum number of "fast" candidates to random-pick from (top N by latency)
    pub top_k: usize,
}

impl LatencyBasedStrategy {
    pub fn new() -> Self {
        Self { top_k: 2 }
    }
}

impl Default for LatencyBasedStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutingStrategy for LatencyBasedStrategy {
    fn select(&self, candidates: &[Channel]) -> Option<Channel> {
        // Separate candidates into those with latency data and those without
        let mut with_latency: Vec<&Channel> = Vec::new();
        let mut without_latency: Vec<&Channel> = Vec::new();

        for c in candidates {
            if c.avg_latency_ms > 0 {
                with_latency.push(c);
            } else {
                without_latency.push(c);
            }
        }

        // If we have latency data, sort by latency and pick from top K
        if !with_latency.is_empty() {
            with_latency.sort_by_key(|c| c.avg_latency_ms);
            let top = &with_latency[..self.top_k.min(with_latency.len())];
            let idx = rand::rng().random_range(0..top.len());
            return Some(top[idx].clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelStatus, Credential, Provider};
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
                cred_type: crate::channel::CredentialType::ApiKey,
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
        }
    }

    #[test]
    fn latency_strategy_prefers_fast_channels() {
        let fast = make_channel("fast", 1, 100, 100);
        let slow = make_channel("slow", 1, 100, 5000);
        let candidates = vec![slow.clone(), fast.clone()];

        let strategy = LatencyBasedStrategy::new();
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
        let ch1 = make_channel("ch1", 1, 100, 0);
        let ch2 = make_channel("ch2", 1, 100, 0);
        let candidates = vec![ch1, ch2];

        let strategy = LatencyBasedStrategy::new();
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
}
