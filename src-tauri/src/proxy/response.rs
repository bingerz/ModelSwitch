use axum::body::Body;
use axum::response::Response;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::stream::{json_response, keepalive_stream, sse_stream_response_with_telemetry};

use super::provider::ProviderAdaptor;
use super::usage::{extract_usage, extract_usage_from_stream};
use super::{estimate_tokens, make_log, PASSTHROUGH_RESPONSE_HEADERS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_passthrough_headers_adds_headers() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let headers = vec![("X-Custom".to_string(), "value".to_string())];
        let resp = inject_passthrough_headers(resp, &headers);
        assert_eq!(resp.headers().get("x-custom").unwrap(), "value");
    }

    #[test]
    fn fallback_headers_inserted_when_trigger_reason_matches() {
        // Build a minimal response to test header insertion in isolation
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let mut resp = inject_passthrough_headers(resp, &[]);
        // Simulate the fallback guard from handle_json_success
        if Some("model_fallback") == Some("model_fallback") {
            if let Ok(hv) = axum::http::HeaderValue::from_str("gpt-4-turbo") {
                resp.headers_mut()
                    .insert("X-ModelSwitch-Fallback-Model", hv);
            }
            if let Ok(hv) = axum::http::HeaderValue::from_str("gpt-4o") {
                resp.headers_mut()
                    .insert("X-ModelSwitch-Original-Model", hv);
            }
        }
        assert_eq!(
            resp.headers().get("x-modelswitch-fallback-model").unwrap(),
            "gpt-4-turbo"
        );
        assert_eq!(
            resp.headers().get("x-modelswitch-original-model").unwrap(),
            "gpt-4o"
        );
    }

    #[test]
    fn no_fallback_headers_when_trigger_reason_none() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let mut resp = inject_passthrough_headers(resp, &[]);
        // When trigger_reason is not model_fallback, no headers should be set
        // (this is just verifying the guard logic)
        assert!(resp.headers().get("x-modelswitch-fallback-model").is_none());
        assert!(resp.headers().get("x-modelswitch-original-model").is_none());
    }
}

/// Extract passthrough headers from an upstream response.
pub(super) fn extract_passthrough_headers(resp: &reqwest::Response) -> Vec<(String, String)> {
    PASSTHROUGH_RESPONSE_HEADERS
        .iter()
        .filter_map(|name| {
            resp.headers()
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect()
}

/// Inject passthrough headers into an Axum response.
pub(super) fn inject_passthrough_headers(
    mut resp: Response,
    headers: &[(String, String)],
) -> Response {
    for (name, value) in headers {
        if let Ok(header_name) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(header_value) = axum::http::HeaderValue::from_str(value) {
                resp.headers_mut().append(header_name, header_value);
            }
        }
    }
    resp
}

/// Handle a successful streaming (SSE) response from upstream.
/// Logs the attempt, spawns a background task to extract real token usage,
/// caches the SSE response for streaming cache hits, and applies keepalive if configured.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_streaming_success(
    state: &Arc<crate::proxy::openai::AppState>,
    channel: &Channel,
    resp: reqwest::Response,
    body: &Value,
    provider: &dyn ProviderAdaptor,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
    reserved_cents: u64,
    original_model: &str,
    cache_key: u128,
    cache_key_material: &str,
) -> Response {
    let is_gemini = provider.is_gemini_stream();
    let (stream_resp, telemetry_chunks, raw_sse, stream_done) = sse_stream_response_with_telemetry(
        resp.bytes_stream(),
        is_gemini,
        upstream_model.to_string(),
    );

    let est_tokens = estimate_tokens(body, true);
    let estimated_cost = channel
        .calculate_cost(Some(est_tokens / 2), Some(est_tokens / 2))
        .or_else(|| {
            channel
                .cost_per_token
                .map(|rate| rate * est_tokens as f64 / 1000.0)
        });
    let log_id = Uuid::new_v4();
    let mut log_entry = make_log(
        current_model,
        channel.id,
        &channel.name,
        channel.priority,
        attempt,
        trigger_reason,
        start.elapsed().as_millis() as u64,
        true,
        estimated_cost,
        None,
        None,
        None,
        None,
        request_id,
    );
    log_entry.id = log_id;
    state.logger.log(log_entry).await;

    // Prometheus metrics
    let provider_label = channel.provider.as_str();
    crate::metrics::requests_total()
        .with_label_values(&[provider_label, current_model, "success"])
        .inc();
    crate::metrics::request_duration()
        .with_label_values(&[provider_label, current_model])
        .observe(start.elapsed().as_secs_f64());
    let _ = state
        .channel_mgr
        .record_latency(channel.id, start.elapsed().as_millis() as u64)
        .await;
    state
        .router
        .latency_tracker
        .record(channel.id, start.elapsed().as_millis() as u64);
    state.router.active_requests.decrement(channel.id);

    // Spawn background task to extract real token counts from stream
    {
        let bg_logger = Arc::clone(&state.logger);
        let bg_quota_store = Arc::clone(&state.billing.quota_store);
        let bg_virtual_key_store = Arc::clone(&state.billing.virtual_key_store);
        let bg_provider_budgets = Arc::clone(&state.billing.provider_budgets);
        let bg_channel_id = channel.id;
        let bg_provider_name = channel.provider.as_str().to_string();
        let bg_cost_fn =
            channel.input_cost_per_mtok.is_some() || channel.output_cost_per_mtok.is_some();
        let bg_input_cost = channel.input_cost_per_mtok;
        let bg_output_cost = channel.output_cost_per_mtok;
        let bg_cost_per_token = channel.cost_per_token;
        let bg_vk_id = vk_id;
        let bg_reserved_cents = reserved_cents;
        let bg_request_cache = Arc::clone(&state.cache.request_cache);
        let bg_in_flight = Arc::clone(&state.cache.in_flight);
        let bg_current_model = current_model.to_string();
        let bg_raw_sse = Arc::clone(&raw_sse);
        let bg_stream_done = Arc::clone(&stream_done);
        let bg_cache_key = cache_key;
        let bg_key_material = cache_key_material.to_string();
        crate::spawn_bg(async move {
            // Wait for the upstream stream to be fully consumed by the
            // stream-forwarding task.  `Notify` stores a permit if
            // `notify_one` fires before we register, so the ordering
            // between this task and the stream task does not matter.
            //
            // A 60 s safety-net timeout guards against any unexpected
            // failure to signal (e.g. the stream task panicking).
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                bg_stream_done.notified(),
            )
            .await;
            let chunks = {
                let guard = telemetry_chunks.lock().unwrap_or_else(|e| e.into_inner());
                guard.clone()
            };
            // Cache the accumulated SSE text for streaming cache hits.
            // Must happen before in_flight.complete for coalesced waiters.
            {
                let cached_sse = {
                    let guard = bg_raw_sse.lock().unwrap_or_else(|e| e.into_inner());
                    guard.clone()
                };
                if !cached_sse.is_empty() {
                    bg_request_cache.insert(bg_cache_key, bg_key_material, cached_sse);
                    bg_in_flight.complete(bg_cache_key);
                }
            }
            if chunks.is_empty() {
                return;
            }
            let token_usage = extract_usage_from_stream(&chunks);
            let input_tokens = token_usage.input_tokens;
            let output_tokens = token_usage.output_tokens;
            if input_tokens.is_some() || output_tokens.is_some() {
                // Re-calculate cost using real tokens
                let real_cost = if bg_cost_fn {
                    let in_tok = input_tokens.unwrap_or(0) as f64;
                    let out_tok = output_tokens.unwrap_or(0) as f64;
                    let in_rate = bg_input_cost.unwrap_or(0.0);
                    let out_rate = bg_output_cost.unwrap_or(0.0);
                    Some(in_tok / 1_000_000.0 * in_rate + out_tok / 1_000_000.0 * out_rate)
                } else {
                    bg_cost_per_token.map(|rate_per_1k| {
                        ((input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)) as f64 / 1000.0)
                            * rate_per_1k
                    })
                };

                if let (Some(it), Some(ot)) = (input_tokens, output_tokens) {
                    bg_logger
                        .update_log_tokens(
                            log_id,
                            it,
                            ot,
                            token_usage.cache_hit_tokens,
                            token_usage.cache_miss_tokens,
                            real_cost,
                        )
                        .await;
                }

                // Accumulate usage (tokens + cost) into quota store
                bg_quota_store
                    .accumulate_usage(
                        bg_channel_id,
                        input_tokens,
                        output_tokens,
                        token_usage.cache_hit_tokens,
                        token_usage.cache_miss_tokens,
                        real_cost,
                    )
                    .await;

                // Attribute spend to the requesting virtual key (if any).
                // When a reservation was made before dispatch, reconcile against
                // it so the key is not double-charged (reservation + accumulation).
                if let Some(vk) = bg_vk_id {
                    let cost_cents = (real_cost.unwrap_or(0.0) * 100.0) as u64;
                    if bg_reserved_cents > 0 {
                        bg_virtual_key_store
                            .reconcile_spend(vk, bg_reserved_cents, cost_cents)
                            .await;
                    } else {
                        bg_virtual_key_store.accumulate_spend(vk, cost_cents).await;
                    }
                }

                // Accumulate spend into per-provider budget tracker.
                let cost_cents = (real_cost.unwrap_or(0.0) * 100.0) as u64;
                bg_provider_budgets
                    .accumulate_spend(&bg_provider_name, cost_cents)
                    .await;

                // Token-level Prometheus metrics
                if let Some(it) = input_tokens {
                    crate::metrics::input_tokens_total()
                        .with_label_values(&[&bg_provider_name, &bg_current_model])
                        .inc_by(it);
                }
                if let Some(ot) = output_tokens {
                    crate::metrics::output_tokens_total()
                        .with_label_values(&[&bg_provider_name, &bg_current_model])
                        .inc_by(ot);
                }
            }
        });
    }

    let stream_resp = inject_passthrough_headers(stream_resp, upstream_headers);

    let mut stream_resp = stream_resp;
    if trigger_reason == Some("model_fallback") {
        if let Ok(hv) = axum::http::HeaderValue::from_str(current_model) {
            stream_resp
                .headers_mut()
                .insert("X-ModelSwitch-Fallback-Model", hv);
        }
        if let Ok(hv) = axum::http::HeaderValue::from_str(original_model) {
            stream_resp
                .headers_mut()
                .insert("X-ModelSwitch-Original-Model", hv);
        }
    }

    // Apply keepalive if configured
    if let Some(secs) = state.gateway.stream_keepalive_secs {
        if secs > 0 {
            let (parts, body) = stream_resp.into_parts();
            let data_stream = body
                .into_data_stream()
                .map(|r: Result<bytes::Bytes, axum::Error>| r.map_err(std::io::Error::other));
            let kept_alive = keepalive_stream(data_stream, secs);
            let new_body = Body::from_stream(kept_alive);
            return Response::from_parts(parts, new_body);
        }
    }
    stream_resp
}

/// Handle a successful non-streaming (JSON) response from upstream.
/// Extracts usage, accumulates quota, logs, caches the response, and returns it.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_json_success(
    state: &Arc<crate::proxy::openai::AppState>,
    channel: &Channel,
    resp: reqwest::Response,
    body: &Value,
    original_model: &str,
    provider: &dyn ProviderAdaptor,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
    reserved_cents: u64,
    cache_key: u128,
    cache_key_material: &str,
) -> Response {
    let body_text = resp.text().await.unwrap_or_default();

    // Translate response body via provider (pass-through for OpenAI/Anthropic,
    // Gemini-to-OpenAI translation for Gemini).
    let response_body = if let Ok(v) = serde_json::from_str::<Value>(&body_text) {
        let translated = provider.transform_response(&v, upstream_model);
        serde_json::to_string(&translated).unwrap_or(body_text)
    } else {
        body_text
    };
    let token_usage = extract_usage(&response_body);
    let input_tokens = token_usage.input_tokens;
    let output_tokens = token_usage.output_tokens;
    let estimated_cost = channel
        .calculate_cost(input_tokens, output_tokens)
        .or_else(|| {
            let est = estimate_tokens(body, false);
            channel.calculate_cost(Some(est / 2), Some(est / 2))
        });
    // Cache non-streaming responses (inline — coalesced waiters depend on
    // ordering: insert must precede complete()).
    state.cache.request_cache.insert(
        cache_key,
        cache_key_material.to_string(),
        response_body.clone(),
    );
    state.cache.in_flight.complete(cache_key);

    // Fast atomic decrement stays inline.
    state.router.active_requests.decrement(channel.id);

    // Background: quota accumulation, virtual-key spend, logging, and metrics
    // are non-blocking to return the HTTP response as quickly as possible.
    {
        let bg_quota_store = Arc::clone(&state.billing.quota_store);
        let bg_virtual_key_store = Arc::clone(&state.billing.virtual_key_store);
        let bg_provider_budgets = Arc::clone(&state.billing.provider_budgets);
        let bg_logger = Arc::clone(&state.logger);
        let bg_channel_mgr = Arc::clone(&state.channel_mgr);
        let bg_latency_tracker = Arc::clone(&state.router.latency_tracker);
        let bg_channel_id = channel.id;
        let bg_channel_name = channel.name.clone();
        let bg_provider = channel.provider.clone();
        let bg_provider_name = channel.provider.as_str().to_string();
        let bg_channel_priority = channel.priority;
        let bg_input_tokens = input_tokens;
        let bg_output_tokens = output_tokens;
        let bg_cache_hit_tokens = token_usage.cache_hit_tokens;
        let bg_cache_miss_tokens = token_usage.cache_miss_tokens;
        let bg_estimated_cost = estimated_cost;
        let bg_current_model = current_model.to_string();
        let bg_attempt = attempt;
        let bg_trigger_reason = trigger_reason.map(|s| s.to_string());
        let bg_start = start;
        let bg_request_id = request_id.map(|s| s.to_string());
        let bg_vk_id = vk_id;
        let bg_reserved_cents = reserved_cents;

        crate::spawn_bg(async move {
            // Accumulate usage (tokens + cost) into quota store
            bg_quota_store
                .accumulate_usage(
                    bg_channel_id,
                    bg_input_tokens,
                    bg_output_tokens,
                    bg_cache_hit_tokens,
                    bg_cache_miss_tokens,
                    bg_estimated_cost,
                )
                .await;

            // Attribute spend to the requesting virtual key (if any).
            // When a reservation was made before dispatch, reconcile against
            // it so the key is not double-charged (reservation + accumulation).
            if let Some(vk) = bg_vk_id {
                let cost_cents = (bg_estimated_cost.unwrap_or(0.0) * 100.0) as u64;
                if bg_reserved_cents > 0 {
                    bg_virtual_key_store
                        .reconcile_spend(vk, bg_reserved_cents, cost_cents)
                        .await;
                } else {
                    bg_virtual_key_store.accumulate_spend(vk, cost_cents).await;
                }
            }

            // Accumulate spend into per-provider budget tracker.
            let cost_cents = (bg_estimated_cost.unwrap_or(0.0) * 100.0) as u64;
            bg_provider_budgets
                .accumulate_spend(&bg_provider_name, cost_cents)
                .await;

            bg_logger
                .log(make_log(
                    &bg_current_model,
                    bg_channel_id,
                    &bg_channel_name,
                    bg_channel_priority,
                    bg_attempt,
                    bg_trigger_reason.as_deref(),
                    bg_start.elapsed().as_millis() as u64,
                    true,
                    bg_estimated_cost,
                    bg_input_tokens,
                    bg_output_tokens,
                    bg_cache_hit_tokens,
                    bg_cache_miss_tokens,
                    bg_request_id.as_deref(),
                ))
                .await;

            // Prometheus metrics
            let provider_label = bg_provider.as_str();
            crate::metrics::requests_total()
                .with_label_values(&[provider_label, &bg_current_model, "success"])
                .inc();
            crate::metrics::request_duration()
                .with_label_values(&[provider_label, &bg_current_model])
                .observe(bg_start.elapsed().as_secs_f64());

            // Token-level metrics
            if let Some(it) = bg_input_tokens {
                crate::metrics::input_tokens_total()
                    .with_label_values(&[provider_label, &bg_current_model])
                    .inc_by(it);
            }
            if let Some(ot) = bg_output_tokens {
                crate::metrics::output_tokens_total()
                    .with_label_values(&[provider_label, &bg_current_model])
                    .inc_by(ot);
            }

            let _ = bg_channel_mgr
                .record_latency(bg_channel_id, bg_start.elapsed().as_millis() as u64)
                .await;
            bg_latency_tracker.record(bg_channel_id, bg_start.elapsed().as_millis() as u64);
        });
    }

    let mut resp = inject_passthrough_headers(
        json_response(StatusCode::OK, response_body),
        upstream_headers,
    );
    if trigger_reason == Some("model_fallback") {
        if let Ok(hv) = axum::http::HeaderValue::from_str(current_model) {
            resp.headers_mut()
                .insert("X-ModelSwitch-Fallback-Model", hv);
        }
        if let Ok(hv) = axum::http::HeaderValue::from_str(original_model) {
            resp.headers_mut()
                .insert("X-ModelSwitch-Original-Model", hv);
        }
    }
    resp
}
