use axum::http::HeaderMap;
use axum::response::Response;
use rand::Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::cache::RequestCache;
use crate::proxy::stream::{
    all_channels_exhausted_response, json_response, sse_stream_response_with_cached,
};
use crate::router;
use crate::router::RoutingContext;
use crate::guardrails::GuardrailAction;
use crate::virtual_key::ReserveResult;

use super::attempt::{try_channel_attempt, AttemptOutcome, DispatchContext};
use super::provider::ProviderAdaptor;
use super::request_meta::{extract_request_meta, extract_virtual_key_id, RequestMeta};
use super::{
    error_response, estimate_tokens, make_log, DispatchLogInput, FailureReason, RequestFormat,
};

/// Check if a model is allowed for a virtual key, with model group awareness.
/// A model is allowed if:
/// 1. It passes the existing `is_model_allowed` check (glob match or None = all)
/// 2. OR it is a member of a group whose name is in `allowed_models`
fn is_model_allowed_with_groups(
    virtual_key: &crate::virtual_key::VirtualKey,
    model: &str,
    model_groups: &HashMap<String, Vec<String>>,
) -> bool {
    // Direct check first (handles None = all allowed, and glob matching)
    if virtual_key.is_model_allowed(model) {
        return true;
    }

    // Check if any group in allowed_models contains this model
    if let Some(ref allowed) = virtual_key.allowed_models {
        for allowed_entry in allowed {
            if let Some(group_models) = model_groups.get(allowed_entry) {
                if group_models
                    .iter()
                    .any(|gm| crate::channel::matches_glob(gm, model))
                {
                    return true;
                }
            }
        }
    }

    false
}

/// RAII guard that increments `active_requests` on creation and decrements on drop.
/// Ensures the gauge is always balanced regardless of which return path dispatch takes.
struct ActiveRequestGuard;

impl ActiveRequestGuard {
    fn new() -> Self {
        crate::metrics::active_requests().inc();
        Self
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        crate::metrics::active_requests().dec();
    }
}

/// Log the all-channels-exhausted outcome, wake coalesced waiters, and return 429.
async fn log_all_exhausted(
    state: &Arc<crate::proxy::AppState>,
    original_model: &str,
    cache_key: u128,
    total_attempts: u32,
    start: std::time::Instant,
    request_id: Option<&str>,
    virtual_key_id: Option<String>,
) -> Response {
    // Complete in-flight entry (no-op if not registered) so coalesced waiters
    // can proceed and re-check the cache.
    state.cache.in_flight.complete(cache_key);
    let reason_str = FailureReason::AllExhausted.log_str();
    let log_input = DispatchLogInput {
        model: original_model,
        channel_id: Uuid::nil(),
        channel_name: "none",
        channel_priority: 0,
        retry_count: total_attempts,
        reason: Some(&reason_str),
        latency_ms: start.elapsed().as_millis() as u64,
        success: false,
        estimated_cost: None,
        input_tokens: None,
        output_tokens: None,
        cache_hit_tokens: None,
        cache_miss_tokens: None,
        request_id,
        virtual_key_id,
    };
    state.logger.log(make_log(&log_input)).await;
    crate::metrics::requests_total()
        .with_label_values(&["none", original_model, "error"])
        .inc();
    all_channels_exhausted_response()
}

/// Check the request cache and coalesce in-flight requests.
/// Returns `Some(response)` on cache hit (caller should return immediately),
/// or `None` to continue dispatch.
/// For streaming requests, the cached SSE text is returned as an event-stream response.
async fn check_request_cache(
    state: &Arc<crate::proxy::AppState>,
    cache_key: u128,
    key_material: &str,
    is_stream: bool,
) -> Option<Response> {
    if let Some(cached) = state.cache.request_cache.get(cache_key, key_material) {
        tracing::info!("Cache hit for request");
        crate::metrics::cache_hits().inc();
        if is_stream {
            return Some(sse_stream_response_with_cached(&cached));
        }
        return Some(json_response(reqwest::StatusCode::OK, cached));
    }
    if !state.cache.in_flight.register(cache_key) {
        // Another request is in flight — wait for it, then check cache
        state.cache.in_flight.wait(cache_key).await;
        if let Some(cached) = state.cache.request_cache.get(cache_key, key_material) {
            tracing::info!("Coalesced request served from cache");
            crate::metrics::cache_hits().inc();
            if is_stream {
                return Some(sse_stream_response_with_cached(&cached));
            }
            return Some(json_response(reqwest::StatusCode::OK, cached));
        }
        // Cache miss after wait — proceed normally as second attempt
        let _ = state.cache.in_flight.register(cache_key);
    }
    crate::metrics::cache_misses().inc();
    None
}

/// Select a channel for the current attempt, preferring session affinity.
/// When `account_group` is `Some(tag)`, only channels with a matching
/// `account_group` or no group at all are considered.
/// Returns `None` if no channel is available for the model.
async fn select_channel_for_attempt(
    affinity_channel: Option<Uuid>,
    channels: &crate::channel::SharedChannels,
    current_model: &str,
    routing_strategy: crate::router::RoutingStrategyType,
    ctx: &RoutingContext<'_>,
    account_group: Option<&str>,
) -> Option<Channel> {
    // Try affinity channel first if still valid.
    // Check against the original channel map with inline account_group
    // filtering so we avoid cloning the entire channel list into a new
    // Vec + Arc<RwLock> on every request.
    if let Some(aff_id) = affinity_channel {
        // Clone the Arc under the read guard, then drop the guard before
        // locking the inner channel. This keeps outer-lock hold time minimal.
        let ch_arc = {
            let guard = channels.read().await;
            guard.get(&aff_id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let ch = ch_arc.read();
            let group_ok = account_group.is_none_or(|tag| {
                ch.account_group.as_deref() == Some(tag) || ch.account_group.is_none()
            });
            let model_ok =
                ch.model_mapping.is_empty() || ch.model_mapping.contains_key(current_model);
            if group_ok
                && model_ok
                && ch.is_available()
                && !ctx.cooldown_tracker.is_in_cooldown(ch.id)
            {
                let mut c = ch.clone();
                c.recover_if_expired();
                return Some(c);
            }
        }
    }

    // Normal routing — pass account_group to select_channel for inline
    // filtering inside the candidate-building filter chain. No more
    // effective_channels clone.
    router::select_channel(
        channels.clone(),
        current_model,
        routing_strategy,
        ctx,
        account_group,
    )
    .await
}

/// Build the fallback model chain: group expansion (if group request),
/// normal fallback chain, then context-window fallbacks appended.
///
/// Deduplicates context-window fallbacks that are already present in the
/// chain (whether from the group expansion or the regular fallback list).
fn build_fallback_chain(
    original_model: &str,
    is_group_request: bool,
    model_groups: &HashMap<String, Vec<String>>,
    model_fallbacks: &HashMap<String, Vec<String>>,
    context_window_fallbacks: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut chain = if is_group_request {
        model_groups
            .get(original_model)
            .cloned()
            .unwrap_or_else(|| vec![original_model.to_string()])
    } else {
        router::fallback::resolve_fallback_chain(original_model, model_fallbacks)
    };

    // Append context-window fallbacks that aren't already in the chain.
    // When a request fails due to context length exceeded, the dispatch loop
    // moves to the next model in the chain — appending context fallbacks here
    // means a context overflow on any model naturally proceeds to larger-context
    // models without a separate retry loop.
    if let Some(ctx_fallbacks) = context_window_fallbacks.get(original_model) {
        for fb in ctx_fallbacks {
            if !chain.contains(fb) {
                chain.push(fb.clone());
            }
        }
    }

    chain
}

/// Compute per-model retry counts from the fallback chain, applying
/// per-model overrides (with wildcard support) and falling back to the
/// global default when no override matches.
fn resolve_model_retry_counts(
    fallback_chain: &[String],
    overrides: &HashMap<String, crate::config::ModelRetryConfig>,
    default_max_retries: u32,
) -> Vec<u32> {
    fallback_chain
        .iter()
        .map(|m| {
            resolve_model_retry_config(m, overrides)
                .max_retries
                .unwrap_or(default_max_retries)
        })
        .collect()
}

/// Pre-flight checks that determine whether a model should be skipped before
/// entering the retry loop. Returns `true` if the model cannot serve the
/// request, either because:
///   - the request body is too large for the model's context window, or
///   - no enabled, available, non-cooldown channel can serve this model.
///
/// The context check is a fast heuristic (chars / 4 ≈ tokens); the per-attempt
/// check in `attempt.rs` provides a more accurate check after body mutation.
async fn pre_flight_skip_model(
    state: &Arc<crate::proxy::AppState>,
    current_model: &str,
    body_str_len: usize,
    channels: &crate::channel::SharedChannels,
) -> bool {
    // P2-8: Pre-flight context validation — skip models whose context window
    // is definitely too small for the request body. This avoids wasting retry
    // budget on a model that cannot possibly fit the request.
    {
        let registry = state.model_registry.read();
        let caps = registry.get(current_model);
        if let Some(max_ctx) = caps.max_context_tokens {
            let estimated_tokens = (body_str_len / 4) as u64;
            if estimated_tokens > max_ctx {
                tracing::warn!(
                    model = %current_model,
                    estimated_tokens,
                    max_context = max_ctx,
                    "Pre-flight: request likely exceeds model context window, skipping model"
                );
                return true;
            }
        }
    }

    // P2-9: Fallback negative caching — skip models with no available channel.
    // If no enabled, available channel can serve this model, there is no point
    // entering the retry loop (channel selection will fail immediately anyway).
    {
        let guard = channels.read().await;
        let has_channel = guard.values().any(|ch_arc| {
            let ch = ch_arc.read();
            ch.enabled
                && ch.is_available()
                && !ch.is_model_excluded(current_model)
                && (ch.model_mapping.is_empty() || ch.model_mapping.contains_key(current_model))
                && !state.router.cooldown_tracker.is_in_cooldown(ch.id)
        });
        if !has_channel {
            tracing::debug!(
                model = %current_model,
                "Skipping fallback model — no available channel"
            );
            return true;
        }
    }

    false
}

/// Shared dispatch logic for both OpenAI and Anthropic proxy handlers.
/// Implements priority-based channel selection, hot retry, circuit breaking,
/// model fallback chains, SSE streaming, and header passthrough.
pub(crate) async fn dispatch(
    state: &Arc<crate::proxy::AppState>,
    original_headers: &HeaderMap,
    body: &Value,
    provider: &dyn ProviderAdaptor,
    request_format: RequestFormat,
) -> Response {
    let _guard = ActiveRequestGuard::new();

    let meta = extract_request_meta(body, original_headers, state, provider.default_model()).await;
    let RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
        account_group,
    } = meta;

    // Resolve gateway-level model alias. The alias (client-facing name) is
    // replaced by the canonical model before any downstream logic so that:
    //   1. Cache keys use the canonical model name (consistent across alias
    //      and canonical requests).
    //   2. Fallback chains match against the real model name.
    //   3. Per-model retry overrides match the canonical name.
    //   4. Virtual-key model whitelists operate on the canonical name.
    // This is different from per-channel model_mapping, which translates at
    // the upstream level after channel selection.
    let original_model = state
        .gateway
        .model_aliases
        .get(&original_model)
        .cloned()
        .unwrap_or(original_model);

    // Guardrails content moderation — fail fast on blocked content.
    if let GuardrailAction::Block(reason) = state.guardrails.check_request(body) {
        return error_response(
            reqwest::StatusCode::FORBIDDEN,
            &reason,
            "content_blocked",
            "guardrails_blocked",
        );
    }

    // Check if the requested model is a group name. If so, the fallback chain
    // will be expanded to the group's member models so the dispatch loop tries
    // each until one has an available channel.
    let is_group_request = state.gateway.model_groups.contains_key(&original_model);
    if is_group_request {
        tracing::info!(
            model = %original_model,
            "Model group requested — will expand to group members in fallback chain"
        );
    }

    let vk_id = extract_virtual_key_id(original_headers);

    // Check virtual key model whitelist (with model group awareness) and denylist
    if let Some(vk) = vk_id {
        if let Some(virtual_key) = state.billing.virtual_key_store.get(vk).await {
            if !is_model_allowed_with_groups(
                &virtual_key,
                &original_model,
                &state.gateway.model_groups,
            ) {
                return error_response(
                    reqwest::StatusCode::FORBIDDEN,
                    &format!(
                        "Model '{}' is not allowed for this virtual key",
                        original_model
                    ),
                    "model_not_allowed",
                    "virtual_key_model_not_allowed",
                );
            }
            if virtual_key.is_model_denied(&original_model) {
                return error_response(
                    reqwest::StatusCode::FORBIDDEN,
                    &format!("Model '{}' is denied for this virtual key", original_model),
                    "model_denied",
                    "virtual_key_model_denied",
                );
            }
        }
    }

    // Pre-charge estimated cost for virtual key budget enforcement.
    // This prevents concurrent requests from all passing the budget check
    // before any spend is recorded. The reservation is reconciled on failure
    // (all channels exhausted) or absorbed into actual spend on success.
    let mut reserved_cents: u64 = 0;
    if let Some(vk) = vk_id {
        let est_tokens = estimate_tokens(body, is_stream);
        // Rough estimate: ~$0.01 per 1000 tokens as a conservative default
        let estimated_cost_cents = (est_tokens as f64 / 1000.0 * 0.01 * 100.0) as u64 + 1;
        match state
            .billing
            .virtual_key_store
            .reserve_spend(vk, estimated_cost_cents)
            .await
        {
            ReserveResult::Exceeded => {
                return error_response(
                    reqwest::StatusCode::PAYMENT_REQUIRED,
                    "Virtual key budget exceeded. Please increase your budget limit or try again later.",
                    "budget_exceeded",
                    "virtual_key_budget_exceeded",
                );
            }
            ReserveResult::NoBudget => {}
            ReserveResult::Reserved(n) => reserved_cents = n,
        }
    }

    let max_retries = state.gateway.max_retries;
    let channels = state.channel_mgr.channels();
    let start = std::time::Instant::now();

    // Compute cache key once — reuse throughout the dispatch chain to avoid
    // recomputing the BLAKE3 hash + canonical serialization 3-4 times.
    let (cache_key, cache_key_material) = RequestCache::compute_key(&original_model, body);

    // Check request cache
    if let Some(cached) =
        check_request_cache(state, cache_key, &cache_key_material, is_stream).await
    {
        return cached;
    }

    // Resolve fallback chain. If the requested model is a group name, expand
    // to the group's member models — the dispatch loop tries each in order
    // until one has an available channel. Otherwise use the normal chain:
    // [original_model, fallback1, fallback2, ...]
    let fallback_chain = build_fallback_chain(
        &original_model,
        is_group_request,
        &state.gateway.model_groups,
        &state.gateway.model_fallbacks,
        &state.gateway.context_window_fallbacks,
    );

    // Compute per-model retry counts, falling back to the global default
    let model_retry_counts: Vec<u32> = resolve_model_retry_counts(
        &fallback_chain,
        &state.gateway.model_retry_overrides,
        max_retries,
    );
    let max_total_attempts: u32 = model_retry_counts.iter().sum();
    let mut total_attempts: u32 = 0;
    let deadline =
        start + std::time::Duration::from_secs(state.gateway.request_timeout_secs.unwrap_or(120));

    // P2-8: Pre-compute serialized body length for pre-flight context estimation.
    // This is a rough heuristic (chars / 4 ≈ tokens) used to skip models whose
    // context window is definitely too small before entering the retry loop.
    let body_str_len = serde_json::to_string(body).unwrap_or_default().len();

    for (model_idx, current_model) in fallback_chain.iter().enumerate() {
        let model_max_retries = model_retry_counts[model_idx];
        let model_cfg =
            resolve_model_retry_config(current_model, &state.gateway.model_retry_overrides);
        let model_base_ms = model_cfg
            .retry_base_ms
            .unwrap_or(state.gateway.retry_base_ms);
        let model_max_ms = model_cfg.retry_max_ms.unwrap_or(state.gateway.retry_max_ms);

        // P2-8 + P2-9: Pre-flight checks — skip models whose context window is
        // too small or that have no available channel. This avoids wasting retry
        // budget on models that cannot possibly succeed.
        if pre_flight_skip_model(state, current_model, body_str_len, &channels).await {
            continue;
        }

        let mut attempt: u32 = 0;

        while attempt < model_max_retries && total_attempts < max_total_attempts {
            // Per-request deadline check
            if std::time::Instant::now() > deadline {
                tracing::warn!(
                    total_attempts,
                    "Per-request deadline exceeded, aborting dispatch"
                );
                break;
            }

            attempt += 1;
            total_attempts += 1;

            let routing_strategy = *state.routing_strategy.read();
            let channel = match select_channel_for_attempt(
                affinity_channel,
                &channels,
                current_model,
                routing_strategy,
                &RoutingContext {
                    active_requests: &state.router.active_requests,
                    rate_limiter: &state.limits.rate_limiter,
                    latency_tracker: &state.router.latency_tracker,
                    cooldown_tracker: &state.router.cooldown_tracker,
                },
                account_group.as_deref(),
            )
            .await
            {
                Some(ch) => ch,
                None => {
                    tracing::warn!(model = %current_model, "No available channel for model");
                    break;
                }
            };

            // Check per-provider budget — skip this channel if the provider's
            // daily or monthly cap has been reached.
            let provider_name = channel.provider.as_str().to_string();
            if !state
                .billing
                .provider_budgets
                .check_budget(&provider_name)
                .await
            {
                tracing::warn!(
                    provider = %provider_name,
                    channel = %channel.name,
                    "Provider budget exceeded, skipping channel"
                );
                continue;
            }

            // Per-channel retry cap: if the selected channel declares a lower
            // max_retries than the model-level default, skip it once the
            // attempt count exceeds the channel's cap.
            if let Some(ch_max) = channel.max_retries {
                if attempt > ch_max {
                    tracing::debug!(
                        channel = %channel.name,
                        attempt,
                        channel_max = ch_max,
                        "Channel retry cap reached, skipping channel"
                    );
                    continue;
                }
            }

            tracing::info!(
                attempt,
                total_attempts,
                model_max_retries,
                channel = %channel.name,
                model = %current_model,
                stream = is_stream,
                is_fallback = current_model != &original_model,
                "Attempting request"
            );

            let dispatch_ctx = DispatchContext {
                original_headers,
                body,
                provider,
                channel: &channel,
                current_model,
                original_model: &original_model,
                is_stream,
                session_id: &session_id,
                start,
                attempt,
                request_id,
                vk_id,
                reserved_cents,
                cache_key,
                cache_key_material: &cache_key_material,
                request_format,
            };
            match try_channel_attempt(state, &dispatch_ctx).await {
                AttemptOutcome::Respond(response) => return response,
                AttemptOutcome::Retry => {
                    crate::metrics::retries_total()
                        .with_label_values(&[channel.provider.as_str(), current_model.as_str()])
                        .inc();
                    // Exponential backoff with jitter to avoid thundering herd
                    let exp_delay = std::cmp::min(
                        model_base_ms.saturating_mul(1u64 << attempt.min(6)),
                        model_max_ms,
                    );
                    let jitter = rand::rng().random_range(0..50);
                    tokio::time::sleep(std::time::Duration::from_millis(exp_delay + jitter)).await;
                    continue;
                }
                AttemptOutcome::ContextOverflow => {
                    // Context window exceeded — skip remaining retries for this
                    // model and immediately try the next model in the fallback
                    // chain (which includes context fallback models with larger
                    // context windows).
                    tracing::info!(
                        model = %current_model,
                        "Context overflow — moving to next model in chain"
                    );
                    break;
                }
            }
        }
    }

    // All channels exhausted — refund the pre-charged amount since no API call succeeded.
    if reserved_cents > 0 {
        if let Some(vk) = vk_id {
            state
                .billing
                .virtual_key_store
                .reconcile_spend(vk, reserved_cents, 0)
                .await;
        }
    }

    log_all_exhausted(
        state,
        &original_model,
        cache_key,
        total_attempts,
        start,
        request_id,
        vk_id.map(|id| id.to_string()),
    )
    .await
}

/// Resolve per-model retry config, supporting wildcard pattern keys.
/// Checks exact match first, then wildcard patterns (longest prefix first),
/// matching the fallback chain resolver's wildcard logic.
fn resolve_model_retry_config(
    model: &str,
    overrides: &HashMap<String, crate::config::ModelRetryConfig>,
) -> crate::config::ModelRetryConfig {
    use crate::router::fallback;

    // Exact match first
    if let Some(cfg) = overrides.get(model) {
        return cfg.clone();
    }

    // Date-suffix stripped exact match
    let stripped = fallback::strip_date_suffix(model);
    if let Some(ref stripped_model) = stripped {
        if let Some(cfg) = overrides.get(stripped_model) {
            return cfg.clone();
        }
    }

    // Wildcard matches (longest prefix first)
    let mut best_match: Option<(&String, &crate::config::ModelRetryConfig)> = None;
    for (key, cfg) in overrides.iter() {
        if key.ends_with('*') && key.len() > 1 {
            let prefix = &key[..key.len() - 1];
            if model.starts_with(prefix)
                && (best_match.is_none() || key.len() > best_match.as_ref().unwrap().0.len())
            {
                best_match = Some((key, cfg));
            }
        } else if key.as_str() == "*" && best_match.is_none() {
            best_match = Some((key, cfg));
        }
    }

    if let Some((_, cfg)) = best_match {
        return cfg.clone();
    }

    crate::config::ModelRetryConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{
        Channel, ChannelStatus, Credential, CredentialType, Provider, SharedChannels,
    };
    use crate::config::ModelRetryConfig;
    use crate::proxy::rate_limiter::RateLimiter;
    use crate::router::active_requests::ActiveRequests;
    use crate::router::latency_tracker::LatencyTracker;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_test_channel(id: Uuid, model_mapping: HashMap<String, String>) -> Channel {
        Channel {
            id,
            name: "test-channel".to_string(),
            provider: Provider::OpenAI,
            priority: 1,
            weight: 1,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: "test-key".to_string(),
                api_key: Some("sk-test".to_string()),
                expires_at: None,
            },
            enabled: true,
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: "https://api.openai.com/v1".to_string(),
            model_mapping,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
            api_keys: vec![],
            excluded_models: vec![],
            model_cooldowns: HashMap::new(),
            proxy_url: None,
            headers: HashMap::new(),
            max_retries: None,
            models_endpoint: None,
            models_refresh_interval_secs: 300,
            tags: vec![],
            quota: None,
        }
    }

    fn make_routing_context<'a>(
        active_requests: &'a Arc<ActiveRequests>,
        rate_limiter: &'a Arc<RateLimiter>,
        latency_tracker: &'a Arc<LatencyTracker>,
        cooldown_tracker: &'a Arc<crate::router::cooldown::CooldownTracker>,
    ) -> RoutingContext<'a> {
        RoutingContext {
            active_requests,
            rate_limiter,
            latency_tracker,
            cooldown_tracker,
        }
    }

    fn make_shared_channels(channels: Vec<Channel>) -> SharedChannels {
        let map: std::collections::HashMap<uuid::Uuid, Arc<parking_lot::RwLock<Channel>>> =
            channels
                .into_iter()
                .map(|c| (c.id, Arc::new(parking_lot::RwLock::new(c))))
                .collect();
        Arc::new(RwLock::new(map))
    }

    #[tokio::test]
    async fn select_channel_returns_none_for_empty_channels() {
        let channels: SharedChannels = Arc::new(RwLock::new(HashMap::new()));
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );
        let result = select_channel_for_attempt(
            None,
            &channels,
            "gpt-4",
            crate::router::RoutingStrategyType::WeightedRandom,
            &ctx,
            None,
        )
        .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn select_channel_returns_some_for_available_channel() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = make_shared_channels(vec![channel]);
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );
        let result = select_channel_for_attempt(
            None,
            &channels,
            "gpt-4",
            crate::router::RoutingStrategyType::WeightedRandom,
            &ctx,
            None,
        )
        .await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn select_channel_uses_affinity_when_valid() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = make_shared_channels(vec![channel]);
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );
        let result = select_channel_for_attempt(
            Some(channel_id),
            &channels,
            "gpt-4",
            crate::router::RoutingStrategyType::WeightedRandom,
            &ctx,
            None,
        )
        .await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, channel_id);
    }

    // ── Account group routing tests ────────────────────────────────────────

    fn make_test_channel_with_group(
        id: Uuid,
        model_mapping: HashMap<String, String>,
        group: Option<&str>,
    ) -> Channel {
        let mut ch = make_test_channel(id, model_mapping);
        ch.account_group = group.map(|s| s.to_string());
        ch
    }

    #[tokio::test]
    async fn account_group_filters_to_matching_channels() {
        let matching_id = Uuid::new_v4();
        let ungrouped_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        let mut mm = HashMap::new();
        mm.insert("gpt-4".to_string(), "gpt-4".to_string());

        let channels: SharedChannels = make_shared_channels(vec![
            make_test_channel_with_group(matching_id, mm.clone(), Some("production")),
            make_test_channel_with_group(ungrouped_id, mm.clone(), None),
            make_test_channel_with_group(other_id, mm, Some("staging")),
        ]);

        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );

        // Call 10 times — should never pick the "staging" channel
        for _ in 0..10 {
            let ch = select_channel_for_attempt(
                None,
                &channels,
                "gpt-4",
                crate::router::RoutingStrategyType::WeightedRandom,
                &ctx,
                Some("production"),
            )
            .await
            .expect("should find a channel");
            assert!(
                ch.id == matching_id || ch.id == ungrouped_id,
                "selected channel {} should be production or ungrouped, not staging",
                ch.id
            );
        }
    }

    #[tokio::test]
    async fn account_group_none_uses_all_channels() {
        let prod_id = Uuid::new_v4();
        let staging_id = Uuid::new_v4();
        let mut mm = HashMap::new();
        mm.insert("gpt-4".to_string(), "gpt-4".to_string());

        let channels: SharedChannels = make_shared_channels(vec![
            make_test_channel_with_group(prod_id, mm.clone(), Some("production")),
            make_test_channel_with_group(staging_id, mm, Some("staging")),
        ]);

        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );

        // No account_group filter — both channels should be reachable.
        let mut seen_ids = std::collections::HashSet::new();
        for _ in 0..20 {
            if let Some(ch) = select_channel_for_attempt(
                None,
                &channels,
                "gpt-4",
                crate::router::RoutingStrategyType::WeightedRandom,
                &ctx,
                None,
            )
            .await
            {
                seen_ids.insert(ch.id);
            }
        }
        assert!(
            seen_ids.contains(&prod_id),
            "production channel should be reachable without account_group filter"
        );
        assert!(
            seen_ids.contains(&staging_id),
            "staging channel should be reachable without account_group filter"
        );
    }

    #[tokio::test]
    async fn account_group_excludes_non_matching() {
        let staging_id = Uuid::new_v4();
        let mut mm = HashMap::new();
        mm.insert("gpt-4".to_string(), "gpt-4".to_string());

        // Only a staging channel exists — requesting production should fail.
        let channels: SharedChannels = make_shared_channels(vec![make_test_channel_with_group(
            staging_id,
            mm,
            Some("staging"),
        )]);

        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let cooldown_tracker = Arc::new(crate::router::cooldown::CooldownTracker::new());
        let ctx = make_routing_context(
            &active_requests,
            &rate_limiter,
            &latency_tracker,
            &cooldown_tracker,
        );

        let result = select_channel_for_attempt(
            None,
            &channels,
            "gpt-4",
            crate::router::RoutingStrategyType::WeightedRandom,
            &ctx,
            Some("production"),
        )
        .await;
        assert!(
            result.is_none(),
            "no channel should be selected when account_group does not match any channel"
        );
    }

    // ── resolve_model_retry_config tests ─────────────────────────────────

    #[test]
    fn resolve_retry_config_exact_match() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "gpt-4o".to_string(),
            ModelRetryConfig {
                max_retries: Some(5),
                retry_base_ms: Some(200),
                retry_max_ms: Some(10000),
            },
        );
        let cfg = resolve_model_retry_config("gpt-4o", &overrides);
        assert_eq!(cfg.max_retries, Some(5));
        assert_eq!(cfg.retry_base_ms, Some(200));
    }

    #[test]
    fn resolve_retry_config_falls_back_to_default() {
        let overrides = HashMap::new();
        let cfg = resolve_model_retry_config("gpt-4o", &overrides);
        assert_eq!(cfg.max_retries, None);
        assert_eq!(cfg.retry_base_ms, None);
    }

    #[test]
    fn resolve_retry_config_wildcard_match() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "gpt-4*".to_string(),
            ModelRetryConfig {
                max_retries: Some(7),
                ..Default::default()
            },
        );
        let cfg = resolve_model_retry_config("gpt-4-turbo", &overrides);
        assert_eq!(cfg.max_retries, Some(7));
    }

    #[test]
    fn resolve_retry_config_date_suffix_stripped() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "claude-3-opus".to_string(),
            ModelRetryConfig {
                max_retries: Some(2),
                ..Default::default()
            },
        );
        let cfg = resolve_model_retry_config("claude-3-opus-20240229", &overrides);
        assert_eq!(cfg.max_retries, Some(2));
    }

    // ── is_model_allowed_with_groups edge cases ────────────────────────────

    use crate::virtual_key::{VirtualKey, VirtualKeySpend};
    use chrono::Utc;

    /// Helper: build a VirtualKey with the given `allowed_models` and no deny list.
    fn make_vk(allowed_models: Option<Vec<&str>>) -> VirtualKey {
        VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "test".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: allowed_models.map(|v| v.into_iter().map(String::from).collect()),
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        }
    }

    #[test]
    fn is_model_allowed_with_groups_none_allowed_returns_true() {
        // allowed_models = None means all models are permitted.
        let vk = make_vk(None);
        let groups = HashMap::new();
        assert!(is_model_allowed_with_groups(&vk, "gpt-4", &groups));
        assert!(is_model_allowed_with_groups(&vk, "claude-3-opus", &groups));
        assert!(is_model_allowed_with_groups(&vk, "anything", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_empty_list_returns_false() {
        // An explicit empty allow-list denies everything (no direct match, no
        // group entries to iterate).
        let vk = make_vk(Some(vec![]));
        let groups = HashMap::new();
        assert!(!is_model_allowed_with_groups(&vk, "gpt-4", &groups));
        assert!(!is_model_allowed_with_groups(&vk, "claude-3", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_direct_match_returns_true() {
        // Exact model name in allowed_models short-circuits to true via
        // VirtualKey::is_model_allowed before groups are consulted.
        let vk = make_vk(Some(vec!["gpt-4"]));
        let groups = HashMap::new();
        assert!(is_model_allowed_with_groups(&vk, "gpt-4", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_direct_no_match_returns_false() {
        // When the model is not in allowed_models and no groups can expand it,
        // the result must be false.
        let vk = make_vk(Some(vec!["gpt-4"]));
        let groups = HashMap::new();
        assert!(!is_model_allowed_with_groups(&vk, "claude-3-opus", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_group_match_returns_true() {
        // allowed_models references a group name whose members include the
        // requested model.
        let vk = make_vk(Some(vec!["openai-models"]));
        let mut groups = HashMap::new();
        groups.insert(
            "openai-models".to_string(),
            vec!["gpt-4".to_string(), "gpt-3.5-turbo".to_string()],
        );
        assert!(is_model_allowed_with_groups(&vk, "gpt-4", &groups));
        assert!(is_model_allowed_with_groups(&vk, "gpt-3.5-turbo", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_group_no_match_returns_false() {
        // allowed_models references a group, but the requested model is not
        // a member of that group.
        let vk = make_vk(Some(vec!["openai-models"]));
        let mut groups = HashMap::new();
        groups.insert(
            "openai-models".to_string(),
            vec!["gpt-4".to_string(), "gpt-3.5-turbo".to_string()],
        );
        assert!(!is_model_allowed_with_groups(&vk, "claude-3-opus", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_group_not_in_map_returns_false() {
        // allowed_models references a group name that does not exist in the
        // model_groups map — should return false rather than panicking.
        let vk = make_vk(Some(vec!["unknown-group"]));
        let groups = HashMap::new();
        assert!(!is_model_allowed_with_groups(&vk, "gpt-4", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_multiple_groups_second_matches() {
        // allowed_models lists multiple groups; the model is only in the
        // second one.
        let vk = make_vk(Some(vec!["group-a", "group-b"]));
        let mut groups = HashMap::new();
        groups.insert("group-a".to_string(), vec!["claude-3-opus".to_string()]);
        groups.insert("group-b".to_string(), vec!["gpt-4".to_string()]);
        assert!(is_model_allowed_with_groups(&vk, "gpt-4", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_glob_in_group_matches() {
        // A group member entry may itself be a glob pattern.
        let vk = make_vk(Some(vec!["openai-models"]));
        let mut groups = HashMap::new();
        groups.insert("openai-models".to_string(), vec!["gpt-4*".to_string()]);
        assert!(is_model_allowed_with_groups(&vk, "gpt-4o", &groups));
        assert!(is_model_allowed_with_groups(&vk, "gpt-4-turbo", &groups));
        assert!(!is_model_allowed_with_groups(&vk, "claude-3", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_mix_direct_and_group() {
        // allowed_models can mix a direct model name and a group; either path
        // to a match should succeed.
        let vk = make_vk(Some(vec!["claude-3-opus", "openai-models"]));
        let mut groups = HashMap::new();
        groups.insert("openai-models".to_string(), vec!["gpt-4".to_string()]);
        // Direct match
        assert!(is_model_allowed_with_groups(&vk, "claude-3-opus", &groups));
        // Group match
        assert!(is_model_allowed_with_groups(&vk, "gpt-4", &groups));
        // Neither
        assert!(!is_model_allowed_with_groups(&vk, "gemini-pro", &groups));
    }

    #[test]
    fn is_model_allowed_with_groups_empty_group_returns_false() {
        // A group that exists but has no members should not match anything.
        let vk = make_vk(Some(vec!["empty-group"]));
        let mut groups = HashMap::new();
        groups.insert("empty-group".to_string(), vec![]);
        assert!(!is_model_allowed_with_groups(&vk, "gpt-4", &groups));
    }

    // ── build_fallback_chain tests ─────────────────────────────────────────

    #[test]
    fn build_fallback_chain_normal_uses_resolve_fallback_chain() {
        // Non-group request: chain starts with the model itself, then any
        // configured fallbacks from model_fallbacks.
        let mut model_fallbacks = HashMap::new();
        model_fallbacks.insert(
            "gpt-4o".to_string(),
            vec!["gpt-4-turbo".to_string(), "gpt-4".to_string()],
        );
        let groups = HashMap::new();
        let ctx_fallbacks = HashMap::new();

        let chain =
            build_fallback_chain("gpt-4o", false, &groups, &model_fallbacks, &ctx_fallbacks);
        assert_eq!(chain, vec!["gpt-4o", "gpt-4-turbo", "gpt-4"]);
    }

    #[test]
    fn build_fallback_chain_group_request_expands_members() {
        // Group request: chain is the group's member list.
        let mut groups = HashMap::new();
        groups.insert(
            "reasoning".to_string(),
            vec!["o1".to_string(), "o3".to_string()],
        );
        let model_fallbacks = HashMap::new();
        let ctx_fallbacks = HashMap::new();

        let chain =
            build_fallback_chain("reasoning", true, &groups, &model_fallbacks, &ctx_fallbacks);
        assert_eq!(chain, vec!["o1", "o3"]);
    }

    #[test]
    fn build_fallback_chain_group_not_in_map_falls_back_to_model() {
        // Group request but the model is not actually a group key — should
        // fall back to a single-element chain containing the model name.
        let groups = HashMap::new();
        let model_fallbacks = HashMap::new();
        let ctx_fallbacks = HashMap::new();

        let chain = build_fallback_chain(
            "unknown-group",
            true,
            &groups,
            &model_fallbacks,
            &ctx_fallbacks,
        );
        assert_eq!(chain, vec!["unknown-group"]);
    }

    #[test]
    fn build_fallback_chain_appends_context_window_fallbacks() {
        // Context-window fallbacks are appended after the regular chain.
        let model_fallbacks = HashMap::new();
        let groups = HashMap::new();
        let mut ctx_fallbacks = HashMap::new();
        ctx_fallbacks.insert(
            "gpt-4".to_string(),
            vec!["gpt-4-128k".to_string(), "gpt-4-turbo".to_string()],
        );

        let chain = build_fallback_chain("gpt-4", false, &groups, &model_fallbacks, &ctx_fallbacks);
        assert_eq!(chain, vec!["gpt-4", "gpt-4-128k", "gpt-4-turbo"]);
    }

    #[test]
    fn build_fallback_chain_dedups_context_fallbacks() {
        // If a context-window fallback is already in the regular chain, it
        // should not be appended again.
        let mut model_fallbacks = HashMap::new();
        model_fallbacks.insert("gpt-4".to_string(), vec!["gpt-4-turbo".to_string()]);
        let groups = HashMap::new();
        let mut ctx_fallbacks = HashMap::new();
        ctx_fallbacks.insert(
            "gpt-4".to_string(),
            // gpt-4-turbo is already in the chain — should be deduped.
            vec!["gpt-4-turbo".to_string(), "claude-long-context".to_string()],
        );

        let chain = build_fallback_chain("gpt-4", false, &groups, &model_fallbacks, &ctx_fallbacks);
        assert_eq!(chain, vec!["gpt-4", "gpt-4-turbo", "claude-long-context"]);
    }

    #[test]
    fn build_fallback_chain_no_fallbacks_returns_single_element() {
        // No group, no fallbacks, no context fallbacks — chain is just the model.
        let model_fallbacks = HashMap::new();
        let groups = HashMap::new();
        let ctx_fallbacks = HashMap::new();

        let chain = build_fallback_chain(
            "claude-3-opus",
            false,
            &groups,
            &model_fallbacks,
            &ctx_fallbacks,
        );
        assert_eq!(chain, vec!["claude-3-opus"]);
    }

    #[test]
    fn build_fallback_chain_group_with_context_append() {
        // Group request with context fallbacks appended after the group members.
        let mut groups = HashMap::new();
        groups.insert(
            "fast".to_string(),
            vec!["gpt-4o-mini".to_string(), "claude-3-haiku".to_string()],
        );
        let model_fallbacks = HashMap::new();
        let mut ctx_fallbacks = HashMap::new();
        ctx_fallbacks.insert("fast".to_string(), vec!["gpt-4o".to_string()]);

        let chain = build_fallback_chain("fast", true, &groups, &model_fallbacks, &ctx_fallbacks);
        assert_eq!(chain, vec!["gpt-4o-mini", "claude-3-haiku", "gpt-4o"]);
    }

    // ── resolve_model_retry_counts tests ──────────────────────────────────

    #[test]
    fn resolve_retry_counts_uses_default_when_no_overrides() {
        let chain = vec!["gpt-4".to_string(), "claude-3".to_string()];
        let overrides = HashMap::new();

        let counts = resolve_model_retry_counts(&chain, &overrides, 3);
        assert_eq!(counts, vec![3, 3]);
    }

    #[test]
    fn resolve_retry_counts_uses_override_when_present() {
        let chain = vec!["gpt-4".to_string(), "claude-3".to_string()];
        let mut overrides = HashMap::new();
        overrides.insert(
            "gpt-4".to_string(),
            ModelRetryConfig {
                max_retries: Some(5),
                ..Default::default()
            },
        );

        let counts = resolve_model_retry_counts(&chain, &overrides, 3);
        assert_eq!(counts, vec![5, 3]);
    }

    #[test]
    fn resolve_retry_counts_wildcard_override_applies() {
        // Wildcard pattern "gpt-*" should match "gpt-4-turbo".
        let chain = vec!["gpt-4-turbo".to_string(), "claude-3-opus".to_string()];
        let mut overrides = HashMap::new();
        overrides.insert(
            "gpt-*".to_string(),
            ModelRetryConfig {
                max_retries: Some(7),
                ..Default::default()
            },
        );

        let counts = resolve_model_retry_counts(&chain, &overrides, 2);
        assert_eq!(counts, vec![7, 2]);
    }

    #[test]
    fn resolve_retry_counts_exact_match_beats_wildcard() {
        // Both an exact and a wildcard match exist — exact should win.
        let chain = vec!["gpt-4o".to_string()];
        let mut overrides = HashMap::new();
        overrides.insert(
            "gpt-4o".to_string(),
            ModelRetryConfig {
                max_retries: Some(10),
                ..Default::default()
            },
        );
        overrides.insert(
            "gpt-*".to_string(),
            ModelRetryConfig {
                max_retries: Some(1),
                ..Default::default()
            },
        );

        let counts = resolve_model_retry_counts(&chain, &overrides, 3);
        assert_eq!(counts, vec![10]);
    }

    #[test]
    fn resolve_retry_counts_empty_chain() {
        let chain: Vec<String> = vec![];
        let overrides = HashMap::new();
        let counts = resolve_model_retry_counts(&chain, &overrides, 3);
        assert!(counts.is_empty());
    }

    #[test]
    fn resolve_retry_counts_date_suffix_stripped_match() {
        // "claude-3-opus-20240229" should match override keyed on
        // "claude-3-opus" via date-suffix stripping.
        let chain = vec!["claude-3-opus-20240229".to_string()];
        let mut overrides = HashMap::new();
        overrides.insert(
            "claude-3-opus".to_string(),
            ModelRetryConfig {
                max_retries: Some(4),
                ..Default::default()
            },
        );

        let counts = resolve_model_retry_counts(&chain, &overrides, 2);
        assert_eq!(counts, vec![4]);
    }

    // ── pre_flight_skip_model tests ───────────────────────────────────────
    //
    // These tests construct a minimal AppState to exercise the two skip paths:
    // context-window-too-small and no-available-channel.

    use crate::channel::manager::ChannelManager;
    use crate::config::{AppConfig, ChannelConfig, GatewayConfig, SanitizerConfig};
    use crate::credential::{create_credential_store, SharedCredentialStore};
    use crate::log::DispatchLogger;
    use crate::mcp::McpManager;
    use crate::model_registry::ModelRegistry;
    use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
    use crate::proxy::payload_rules::ChannelPayloadRules;
    use crate::proxy::{
        AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
        SecurityState,
    };
    use crate::quota::QuotaStore;
    use crate::router::affinity::SessionAffinity;
    use crate::virtual_key::VirtualKeyStore;

    /// Build a minimal `AppState` for pre-flight skip tests. The model
    /// registry uses built-in defaults (e.g. gpt-4 has 8192 max context).
    fn build_skip_test_state() -> Arc<AppState> {
        let config = AppConfig {
            gateway: GatewayConfig {
                max_retries: 3,
                health_check_enabled: false,
                ..GatewayConfig::default()
            },
            channels: vec![],
            mcp_servers: vec![],
        };
        let credential_store: SharedCredentialStore = create_credential_store(None);
        let channel_mgr = Arc::new(ChannelManager::new(&config, Arc::clone(&credential_store)));
        let logger = Arc::new(DispatchLogger::new(1000));
        let http_pool = crate::http_pool::HttpPool::new(1, || {
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(30))
        })
        .expect("Failed to build HTTP client pool");

        let model_registry = Arc::new(parking_lot::RwLock::new(ModelRegistry::new()));

        Arc::new(AppState {
            channel_mgr,
            credential_store,
            logger,
            audit_log: Arc::new(crate::admin::audit::AuditLog::with_default_capacity()),
            http_pool,
            model_registry,
            gateway: ProxyParams {
                request_timeout_secs: Some(30),
                stream_keepalive_secs: None,
                stream_ttft_timeout_secs: Some(30),
                max_retries: config.gateway.max_retries,
                model_fallbacks: HashMap::new(),
                context_window_fallbacks: HashMap::new(),
                model_aliases: HashMap::new(),
                routing_strategy: crate::router::RoutingStrategyType::WeightedRandom,
                retry_base_ms: config.gateway.retry_base_ms,
                retry_max_ms: config.gateway.retry_max_ms,
                model_retry_overrides: HashMap::new(),
                nonstream_keepalive_interval_secs: 0,
                passthrough_headers: vec![],
                stream_bootstrap_retries: 0,
                disable_image_generation: false,
                model_groups: HashMap::new(),
                model_pricing: HashMap::new(),
                completion_ratios: HashMap::new(),
                group_ratios: HashMap::new(),
            },
            router: RouterState {
                session_affinity: SessionAffinity::default(),
                active_requests: Arc::new(ActiveRequests::new()),
                latency_tracker: Arc::new(LatencyTracker::new()),
                cooldown_tracker: Arc::new(crate::router::cooldown::CooldownTracker::new()),
            },
            cache: CacheState {
                request_cache: Arc::new(RequestCache::new(
                    std::time::Duration::from_secs(300),
                    1000,
                    CacheMode::On,
                )),
                in_flight: Arc::new(InFlightRequests::new()),
            },
            limits: LimitsState {
                payload_rules: Arc::new(ChannelPayloadRules::new()),
                rate_limiter: Arc::new(RateLimiter::new(None)),
            },
            billing: BillingState {
                quota_store: Arc::new(QuotaStore::new()),
                virtual_key_store: Arc::new(VirtualKeyStore::new()),
                provider_budgets: Arc::new(crate::provider_budget::ProviderBudgetStore::new()),
                key_rate_limiter: Arc::new(crate::proxy::rate_limiter::KeyRateLimiter::new()),
            },
            mcp: McpState {
                mcp_manager: Arc::new(McpManager::new()),
                mcp_max_iterations: 5,
                mcp_auto_inject: false,
                mcp_gateway_enabled: false,
            },
            security: SecurityState {
                admin_token: None,
                admin_roles: vec![],
                sanitizer_config: Arc::new(parking_lot::RwLock::new(SanitizerConfig::default())),
                allowed_origins: None,
                trust_forwarded_headers: false,
                allow_open_proxy: false,
            },
            guardrails: Arc::new(crate::guardrails::GuardrailsChecker::new(
                crate::guardrails::GuardrailsConfig::default(),
            )),
            redemption_codes: Arc::new(crate::quota::RedemptionCodeStore::new()),
            notifications: Arc::new(crate::notification::NotificationService::new(
                crate::notification::NotificationConfig::default(),
            )),
            completion_ratios: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            routing_strategy: Arc::new(parking_lot::RwLock::new(
                crate::router::RoutingStrategyType::WeightedRandom,
            )),
            ldap_config: None,
            oidc_config: None,
            oidc_state_secret: "test-state-secret".to_string(),
            started_at: std::time::Instant::now(),
        })
    }

    #[tokio::test]
    async fn pre_flight_skip_model_context_window_too_small() {
        // gpt-4 has max_context_tokens = 8192 in the built-in registry.
        // body_str_len = 40_000 → estimated_tokens = 10_000 > 8192 → skip.
        let state = build_skip_test_state();
        let channels: SharedChannels = make_shared_channels(vec![]);

        let skip = pre_flight_skip_model(&state, "gpt-4", 40_000, &channels).await;
        assert!(
            skip,
            "model should be skipped when estimated tokens exceed context window"
        );
    }

    #[tokio::test]
    async fn pre_flight_skip_model_context_window_ok_small_body() {
        // Small body — context check passes, but empty channels means no
        // available channel → should still skip.
        let state = build_skip_test_state();
        let channels: SharedChannels = make_shared_channels(vec![]);

        let skip = pre_flight_skip_model(&state, "gpt-4", 100, &channels).await;
        assert!(
            skip,
            "model should be skipped when no available channel exists"
        );
    }

    #[tokio::test]
    async fn pre_flight_skip_model_returns_false_with_available_channel() {
        // Small body + a channel that can serve the model → should NOT skip.
        let state = build_skip_test_state();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(Uuid::new_v4(), model_mapping);
        let channels: SharedChannels = make_shared_channels(vec![channel]);

        let skip = pre_flight_skip_model(&state, "gpt-4", 100, &channels).await;
        assert!(
            !skip,
            "model should not be skipped when context is fine and a channel is available"
        );
    }

    #[tokio::test]
    async fn pre_flight_skip_model_unknown_model_no_context_limit() {
        // Unknown model has max_context_tokens = None (no limit) → context
        // check is skipped. With empty channels → skip (no channel).
        let state = build_skip_test_state();
        let channels: SharedChannels = make_shared_channels(vec![]);

        let skip = pre_flight_skip_model(&state, "totally-unknown-model", 999_999, &channels).await;
        assert!(
            skip,
            "unknown model with no context limit should still be skipped when no channel"
        );
    }
}
