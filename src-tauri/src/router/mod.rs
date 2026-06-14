pub mod active_requests;
pub mod affinity;
pub mod circuit;
pub mod fallback;
pub mod priority;
pub mod strategy;
pub mod weighted;

use crate::channel::{Channel, SharedChannels};
use active_requests::ActiveRequests;
use strategy::{LatencyBasedStrategy, LeastBusyStrategy, RoutingStrategy, WeightedRandomStrategy};

/// Select a healthy channel using the specified routing strategy.
/// Falls back to weighted_random for unknown strategy names.
pub async fn select_channel(
    channels: SharedChannels,
    requested_model: &str,
    routing_strategy: &str,
    active_requests: &ActiveRequests,
) -> Option<Channel> {
    let guard = channels.read().await;

    // Recover any expired circuit-open channels and filter by availability + model support
    let mut candidates: Vec<Channel> = guard
        .iter()
        .map(|c| {
            let mut c = c.clone();
            c.recover_if_expired();
            c
        })
        .filter(|c| c.is_available())
        .filter(|c| {
            // Empty mapping = pass-through, supports all models
            // Non-empty mapping = only supports explicitly listed models
            c.model_mapping.is_empty() || c.model_mapping.contains_key(requested_model)
        })
        .collect();

    drop(guard);

    if candidates.is_empty() {
        return None;
    }

    // Group by priority, sorted ascending (priority 1 = highest priority)
    // Within the same priority, prefer Healthy over HalfOpen channels
    // to limit probe traffic to recovering channels.
    candidates.sort_by(|a, b| {
        a.priority.cmp(&b.priority).then_with(|| {
            let a_healthy = a.status == crate::channel::ChannelStatus::Healthy;
            let b_healthy = b.status == crate::channel::ChannelStatus::Healthy;
            b_healthy.cmp(&a_healthy) // Healthy first
        })
    });

    let strategy: Box<dyn RoutingStrategy> = match routing_strategy {
        "latency" => Box::new(LatencyBasedStrategy::new()),
        "least_busy" => Box::new(LeastBusyStrategy::new(std::sync::Arc::new(
            active_requests.clone(),
        ))),
        _ => Box::new(WeightedRandomStrategy),
    };

    let mut current_priority = 0u8;
    let mut healthy_candidates: Vec<Channel> = Vec::new();
    let mut halfopen_candidates: Vec<Channel> = Vec::new();

    let flush = |healthy: &mut Vec<Channel>,
                 halfopen: &mut Vec<Channel>,
                 strategy: &Box<dyn RoutingStrategy>|
     -> Option<Channel> {
        // Try Healthy candidates first, then fall back to HalfOpen probes
        if !healthy.is_empty() {
            if let Some(selected) = strategy.select(healthy) {
                return Some(selected);
            }
        }
        if !halfopen.is_empty() {
            if let Some(selected) = strategy.select(halfopen) {
                return Some(selected);
            }
        }
        None
    };

    for ch in candidates {
        if ch.priority != current_priority {
            // Try selection from previous priority
            if let Some(selected) =
                flush(&mut healthy_candidates, &mut halfopen_candidates, &strategy)
            {
                return Some(selected);
            }
            healthy_candidates.clear();
            halfopen_candidates.clear();
            current_priority = ch.priority;
        }
        if ch.status == crate::channel::ChannelStatus::HalfOpen {
            halfopen_candidates.push(ch);
        } else {
            healthy_candidates.push(ch);
        }
    }

    // Try last priority
    flush(&mut healthy_candidates, &mut halfopen_candidates, &strategy)
}
