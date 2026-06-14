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
    candidates.sort_by_key(|c| c.priority);

    let strategy: Box<dyn RoutingStrategy> = match routing_strategy {
        "latency" => Box::new(LatencyBasedStrategy::new()),
        "least_busy" => Box::new(LeastBusyStrategy::new(std::sync::Arc::new(
            active_requests.clone(),
        ))),
        _ => Box::new(WeightedRandomStrategy),
    };

    let mut current_priority = 0u8;
    let mut priority_candidates: Vec<Channel> = Vec::new();

    for ch in candidates {
        if ch.priority != current_priority {
            // Try selection from previous priority
            if !priority_candidates.is_empty() {
                if let Some(selected) = strategy.select(&priority_candidates) {
                    return Some(selected);
                }
            }
            current_priority = ch.priority;
            priority_candidates.clear();
        }
        priority_candidates.push(ch);
    }

    // Try last priority
    if !priority_candidates.is_empty() {
        strategy.select(&priority_candidates)
    } else {
        None
    }
}
