pub mod active_requests;
pub mod affinity;
pub mod fallback;
pub mod latency_tracker;
pub mod strategy;
pub mod weighted;

use crate::channel::{Channel, SharedChannels};
use crate::proxy::rate_limiter::RateLimiter;
use active_requests::ActiveRequests;
use latency_tracker::LatencyTracker;
use std::sync::Arc;
use strategy::{
    LatencyBasedStrategy, LeastBusyStrategy, LowestCostStrategy, RoutingStrategy,
    UsageBasedStrategy, WeightedRandomStrategy,
};

/// Context references needed by the routing layer.
/// Bundled into a struct to keep `select_channel` signatures manageable.
pub struct RoutingContext<'a> {
    pub active_requests: &'a std::sync::Arc<ActiveRequests>,
    pub rate_limiter: &'a Arc<RateLimiter>,
    pub latency_tracker: &'a Arc<LatencyTracker>,
}

/// Select a healthy channel using the specified routing strategy.
/// Falls back to weighted_random for unknown strategy names.
pub async fn select_channel(
    channels: SharedChannels,
    requested_model: &str,
    routing_strategy: &str,
    ctx: &RoutingContext<'_>,
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
            // Check concurrent request limit
            if let Some(max) = c.max_concurrent {
                ctx.active_requests.get(c.id) < max
            } else {
                true // No limit configured
            }
        })
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
        "latency" => Box::new(LatencyBasedStrategy::new(Arc::clone(ctx.latency_tracker))),
        "least_busy" => Box::new(LeastBusyStrategy::new(std::sync::Arc::clone(
            ctx.active_requests,
        ))),
        "usage" => Box::new(UsageBasedStrategy::new(Arc::clone(ctx.rate_limiter))),
        "lowest_cost" => Box::new(LowestCostStrategy),
        _ => Box::new(WeightedRandomStrategy),
    };

    let mut current_priority = 0u8;
    let mut healthy_candidates: Vec<Channel> = Vec::new();
    let mut halfopen_candidates: Vec<Channel> = Vec::new();

    let flush = |healthy: &mut Vec<Channel>,
                 halfopen: &mut Vec<Channel>,
                 strategy: &dyn RoutingStrategy|
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
            if let Some(selected) = flush(
                &mut healthy_candidates,
                &mut halfopen_candidates,
                &*strategy,
            ) {
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
    flush(
        &mut healthy_candidates,
        &mut halfopen_candidates,
        &*strategy,
    )
}
