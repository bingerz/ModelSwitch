use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::cache::RequestCache;
use crate::proxy::stream::{all_channels_exhausted_response, json_response};
use crate::router;
use crate::router::RoutingContext;
use crate::virtual_key::ReserveResult;

use super::attempt::{try_channel_attempt, AttemptOutcome};
use super::provider::ProviderAdaptor;
use super::request_meta::{
    extract_request_meta, extract_virtual_key_id, is_affinity_valid, RequestMeta,
};
use super::{estimate_tokens, make_log, FailureReason};

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
    state: &Arc<crate::proxy::openai::AppState>,
    original_model: &str,
    body: &Value,
    is_stream: bool,
    total_attempts: u32,
    start: std::time::Instant,
    request_id: Option<&str>,
) -> Response {
    if !is_stream {
        let cache_key = RequestCache::cache_key(original_model, body);
        state.cache.in_flight.complete(cache_key);
    }
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
async fn check_request_cache(
    state: &Arc<crate::proxy::openai::AppState>,
    original_model: &str,
    body: &Value,
) -> Option<Response> {
    let (cache_key, key_material) = RequestCache::compute_key(original_model, body);
    if let Some(cached) = state.cache.request_cache.get(cache_key, &key_material) {
        tracing::info!("Cache hit for request");
        crate::metrics::cache_hits().inc();
        return Some(json_response(reqwest::StatusCode::OK, cached));
    }
    if !state.cache.in_flight.register(cache_key) {
        // Another request is in flight — wait for it, then check cache
        state.cache.in_flight.wait(cache_key).await;
        if let Some(cached) = state.cache.request_cache.get(cache_key, &key_material) {
            tracing::info!("Coalesced request served from cache");
            crate::metrics::cache_hits().inc();
            return Some(json_response(reqwest::StatusCode::OK, cached));
        }
        // Cache miss after wait — proceed normally as second attempt
        let _ = state.cache.in_flight.register(cache_key);
    }
    crate::metrics::cache_misses().inc();
    None
}

/// Select a channel for the current attempt, preferring session affinity.
/// Returns `None` if no channel is available for the model.
async fn select_channel_for_attempt(
    affinity_channel: Option<Uuid>,
    channels: &crate::channel::SharedChannels,
    current_model: &str,
    routing_strategy: &str,
    ctx: &RoutingContext<'_>,
) -> Option<Channel> {
    // Try affinity channel first if still valid
    if let Some(aff_id) = affinity_channel {
        if is_affinity_valid(aff_id, channels, current_model).await {
            let guard = channels.read().await;
            let found = guard.iter().find(|c| c.id == aff_id).map(|c| {
                let mut c = c.clone();
                c.recover_if_expired();
                c
            });
            drop(guard);
            if let Some(ch) = found {
                return Some(ch);
            }
        }
    }
    // No valid affinity channel — use normal routing
    router::select_channel(channels.clone(), current_model, routing_strategy, ctx).await
}

/// Shared dispatch logic for both OpenAI and Anthropic proxy handlers.
/// Implements priority-based channel selection, hot retry, circuit breaking,
/// model fallback chains, SSE streaming, and header passthrough.
pub(crate) async fn dispatch(
    state: &Arc<crate::proxy::openai::AppState>,
    original_headers: &HeaderMap,
    body: &Value,
    provider: &dyn ProviderAdaptor,
) -> Response {
    let _guard = ActiveRequestGuard::new();

    let meta = extract_request_meta(body, original_headers, state, provider.default_model()).await;
    let RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
    } = meta;
    let vk_id = extract_virtual_key_id(original_headers);

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

    // Check request cache (only for non-streaming requests)
    if !is_stream {
        if let Some(cached) = check_request_cache(state, &original_model, body).await {
            return cached;
        }
    }

    // Resolve fallback chain: [original_model, fallback1, fallback2, ...]
    let fallback_chain =
        router::fallback::resolve_fallback_chain(&original_model, &state.gateway.model_fallbacks);

    let mut total_attempts: u32 = 0;
    let max_total_attempts = max_retries * fallback_chain.len() as u32;
    let deadline =
        start + std::time::Duration::from_secs(state.gateway.request_timeout_secs.unwrap_or(120));

    for current_model in &fallback_chain {
        let mut attempt: u32 = 0;

        while total_attempts < max_total_attempts {
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
                &state.gateway.routing_strategy,
                &RoutingContext {
                    active_requests: &state.router.active_requests,
                    rate_limiter: &state.limits.rate_limiter,
                    latency_tracker: &state.router.latency_tracker,
                },
            )
            .await
            {
                Some(ch) => ch,
                None => {
                    tracing::warn!(model = %current_model, "No available channel for model");
                    break;
                }
            };

            tracing::info!(
                attempt,
                total_attempts,
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
            )
            .await
            {
                AttemptOutcome::Respond(response) => return response,
                AttemptOutcome::Retry => continue,
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
        body,
        is_stream,
        total_attempts,
        start,
        request_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{
        Channel, ChannelStatus, Credential, CredentialType, Provider, SharedChannels,
    };
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
            failure_window_start: None,
            window_failure_count: 0,
            max_concurrent: None,
            api_keys: vec![],
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

    #[tokio::test]
    async fn select_channel_returns_none_for_empty_channels() {
        let channels: SharedChannels = Arc::new(RwLock::new(vec![]));
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
        let result = select_channel_for_attempt(None, &channels, "gpt-4", "weighted", &ctx).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn select_channel_returns_some_for_available_channel() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = Arc::new(RwLock::new(vec![channel]));
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
        let result = select_channel_for_attempt(None, &channels, "gpt-4", "weighted", &ctx).await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn select_channel_uses_affinity_when_valid() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = Arc::new(RwLock::new(vec![channel]));
        let active_requests = Arc::new(ActiveRequests::new());
        let rate_limiter = Arc::new(RateLimiter::new(None));
        let latency_tracker = Arc::new(LatencyTracker::new());
        let ctx = make_routing_context(&active_requests, &rate_limiter, &latency_tracker);
        let result =
            select_channel_for_attempt(Some(channel_id), &channels, "gpt-4", "weighted", &ctx)
                .await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, channel_id);
    }
}
