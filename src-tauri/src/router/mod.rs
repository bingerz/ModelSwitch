pub mod active_requests;
pub mod affinity;
pub mod cooldown;
pub mod fallback;
pub mod latency_tracker;
pub mod strategy;
pub mod weighted;

use crate::channel::{Channel, SharedChannels};
use crate::proxy::rate_limiter::RateLimiter;
use crate::router::cooldown::CooldownTracker;
use active_requests::ActiveRequests;
use latency_tracker::LatencyTracker;
use std::sync::Arc;
use strategy::{
    LatencyBasedStrategy, LeastBusyStrategy, LowestCostStrategy, RoutingStrategy,
    UsageBasedStrategy, WeightedRandomStrategy,
};

pub use strategy::RoutingStrategyType;

/// Context references needed by the routing layer.
/// Bundled into a struct to keep `select_channel` signatures manageable.
pub struct RoutingContext<'a> {
    pub active_requests: &'a std::sync::Arc<ActiveRequests>,
    pub rate_limiter: &'a Arc<RateLimiter>,
    pub latency_tracker: &'a Arc<LatencyTracker>,
    pub cooldown_tracker: &'a Arc<CooldownTracker>,
}

/// Select a healthy channel using the specified routing strategy.
///
/// When `account_group` is `Some(tag)`, only channels whose `account_group`
/// matches the tag or is `None` (universal) are considered.
pub async fn select_channel(
    channels: SharedChannels,
    requested_model: &str,
    routing_strategy: RoutingStrategyType,
    ctx: &RoutingContext<'_>,
    account_group: Option<&str>,
) -> Option<Channel> {
    let guard = channels.read().await;

    // Filter by availability, group, concurrency, and model — then clone + recover
    // only the surviving candidates. is_available() already accounts for expired
    // circuit breakers, so recover_if_expired() can safely run after filtering.
    //
    // Each inner channel is read-locked only briefly per iteration. The outer
    // HashMap read guard is held for the full scan so we get a consistent
    // snapshot, but it is an RwLock read guard — routing reads of OTHER
    // channels can proceed concurrently with any per-channel write (those
    // acquire the inner parking_lot::RwLock, not this outer one).
    let mut candidates: Vec<Channel> = guard
        .values()
        .filter_map(|ch_arc| {
            let c = ch_arc.read();
            if !c.is_available() {
                return None;
            }
            // Account group filter: match if channel group equals tag, or
            // channel has no group (universal).
            if let Some(tag) = account_group {
                if !(c.account_group.as_deref() == Some(tag) || c.account_group.is_none()) {
                    return None;
                }
            }
            // Check concurrent request limit
            if let Some(max) = c.max_concurrent {
                if ctx.active_requests.get(c.id) >= max {
                    return None;
                }
            }
            // Empty mapping = pass-through, supports all models
            // Non-empty mapping = only supports explicitly listed models
            if !c.model_mapping.is_empty() && !c.model_mapping.contains_key(requested_model) {
                return None;
            }
            // Check excluded-models patterns (glob match)
            if c.is_model_excluded(requested_model) {
                return None;
            }
            // Check per-model rate-limit cooldown
            if c.is_model_in_cooldown(requested_model) {
                return None;
            }
            // Check failure-rate-based cooldown
            if ctx.cooldown_tracker.is_in_cooldown(c.id) {
                return None;
            }
            let mut cloned = c.clone();
            cloned.recover_if_expired();
            Some(cloned)
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
        RoutingStrategyType::Latency => {
            Box::new(LatencyBasedStrategy::new(Arc::clone(ctx.latency_tracker)))
        }
        RoutingStrategyType::LeastBusy => Box::new(LeastBusyStrategy::new(std::sync::Arc::clone(
            ctx.active_requests,
        ))),
        RoutingStrategyType::Usage => {
            Box::new(UsageBasedStrategy::new(Arc::clone(ctx.rate_limiter)))
        }
        RoutingStrategyType::LowestCost => Box::new(LowestCostStrategy),
        RoutingStrategyType::WeightedRandom => Box::new(WeightedRandomStrategy),
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
