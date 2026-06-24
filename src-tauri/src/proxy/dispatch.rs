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
use crate::virtual_key::ReserveResult;

use super::attempt::{try_channel_attempt, AttemptOutcome};
use super::provider::ProviderAdaptor;
use super::request_meta::{extract_request_meta, extract_virtual_key_id, RequestMeta};
use super::{estimate_tokens, make_log, FailureReason, RequestFormat};

/// Check if a model is allowed for a virtual key, with model group awareness.
/// A model is allowed if:
/// 1. It passes the existing `is_model_allowed` check (direct or prefix match)
/// 2. OR it is a member of a group whose name is in `allowed_models`
fn is_model_allowed_with_groups(
    virtual_key: &crate::virtual_key::VirtualKey,
    model: &str,
    model_groups: &HashMap<String, Vec<String>>,
) -> bool {
    // Direct check first (handles None = all allowed, and prefix matching)
    if virtual_key.is_model_allowed(model) {
        return true;
    }

    // Check if any group in allowed_models contains this model
    if let Some(ref allowed) = virtual_key.allowed_models {
        for allowed_entry in allowed {
            if let Some(group_models) = model_groups.get(allowed_entry) {
                if group_models
                    .iter()
                    .any(|gm| model == gm || model.starts_with(gm))
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
) -> Response {
    // Complete in-flight entry (no-op if not registered) so coalesced waiters
    // can proceed and re-check the cache.
    state.cache.in_flight.complete(cache_key);
    state
        .logger
        .log(make_log(
            original_model,
            Uuid::nil(),
            "none",
            0,
            total_attempts,
            Some(&FailureReason::AllExhausted.log_str()),
            start.elapsed().as_millis() as u64,
            false,
            None,
            None,
            None,
            None,
            None,
            request_id,
        ))
        .await;
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
            if group_ok && model_ok && ch.is_available() {
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

    // Check virtual key model whitelist (with model group awareness)
    if let Some(vk) = vk_id {
        if let Some(virtual_key) = state.billing.virtual_key_store.get(vk).await {
            if !is_model_allowed_with_groups(&virtual_key, &original_model, &state.gateway.model_groups) {
                let error_body = serde_json::json!({
                    "error": {
                        "message": format!("Model '{}' is not allowed for this virtual key", original_model),
                        "type": "model_not_allowed",
                        "code": "virtual_key_model_not_allowed"
                    }
                });
                return json_response(reqwest::StatusCode::FORBIDDEN, error_body.to_string());
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
                let error_body = serde_json::json!({
                    "error": {
                        "message": "Virtual key budget exceeded. Please increase your budget limit or try again later.",
                        "type": "budget_exceeded",
                        "code": "virtual_key_budget_exceeded"
                    }
                });
                return json_response(
                    reqwest::StatusCode::PAYMENT_REQUIRED,
                    error_body.to_string(),
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
    let mut fallback_chain = if is_group_request {
        state
            .gateway
            .model_groups
            .get(&original_model)
            .cloned()
            .unwrap_or_else(|| vec![original_model.clone()])
    } else {
        router::fallback::resolve_fallback_chain(&original_model, &state.gateway.model_fallbacks)
    };

    // Append context window fallbacks to the chain. When a request fails due
    // to context length exceeded, the dispatch loop moves to the next model
    // in the chain. Appending context fallbacks here means a context overflow
    // on the original model (or any regular fallback) will naturally proceed
    // to the larger-context models without a separate retry loop.
    if let Some(ctx_fallbacks) = state.gateway.context_window_fallbacks.get(&original_model) {
        for fb in ctx_fallbacks {
            if !fallback_chain.contains(fb) {
                fallback_chain.push(fb.clone());
            }
        }
    }

    // Compute per-model retry counts, falling back to the global default
    let model_retry_counts: Vec<u32> = fallback_chain
        .iter()
        .map(|m| {
            resolve_model_retry_config(m, &state.gateway.model_retry_overrides)
                .max_retries
                .unwrap_or(max_retries)
        })
        .collect();
    let max_total_attempts: u32 = model_retry_counts.iter().sum();
    let mut total_attempts: u32 = 0;
    let deadline =
        start + std::time::Duration::from_secs(state.gateway.request_timeout_secs.unwrap_or(120));

    for (model_idx, current_model) in fallback_chain.iter().enumerate() {
        let model_max_retries = model_retry_counts[model_idx];
        let model_cfg =
            resolve_model_retry_config(current_model, &state.gateway.model_retry_overrides);
        let model_base_ms = model_cfg
            .retry_base_ms
            .unwrap_or(state.gateway.retry_base_ms);
        let model_max_ms = model_cfg.retry_max_ms.unwrap_or(state.gateway.retry_max_ms);

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

            let channel = match select_channel_for_attempt(
                affinity_channel,
                &channels,
                current_model,
                state.gateway.routing_strategy,
                &RoutingContext {
                    active_requests: &state.router.active_requests,
                    rate_limiter: &state.limits.rate_limiter,
                    latency_tracker: &state.router.latency_tracker,
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

            match try_channel_attempt(
                state,
                original_headers,
                body,
                provider,
                &channel,
                current_model,
                &original_model,
                is_stream,
                &session_id,
                start,
                attempt,
                request_id,
                vk_id,
                reserved_cents,
                cache_key,
                &cache_key_material,
                request_format,
            )
            .await
            {
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
        }
    }

    fn make_routing_context<'a>(
        active_requests: &'a Arc<ActiveRequests>,
        rate_limiter: &'a Arc<RateLimiter>,
        latency_tracker: &'a Arc<LatencyTracker>,
    ) -> RoutingContext<'a> {
        RoutingContext {
            active_requests,
            rate_limiter,
            latency_tracker,
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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);

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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);

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
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);

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
}
