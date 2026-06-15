use axum::body::Body;
use axum::response::Response;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::cache::RequestCache;
use crate::proxy::stream::{json_response, keepalive_stream, sse_stream_response_with_telemetry};
use crate::proxy::translate::gemini_to_openai;

use super::usage::{extract_usage, extract_usage_from_stream};
use super::{
    estimate_tokens, make_log, AuthStyle, ProxyConfig, PASSTHROUGH_RESPONSE_HEADERS,
};

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
/// and applies keepalive if configured.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_streaming_success(
    state: &Arc<crate::proxy::openai::AppState>,
    channel: &Channel,
    resp: reqwest::Response,
    body: &Value,
    proxy_config: &ProxyConfig,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
) -> Response {
    let is_gemini = matches!(proxy_config.auth_style, AuthStyle::GeminiUrl);
    let (stream_resp, telemetry_chunks) = sse_stream_response_with_telemetry(
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
    let _ = state
        .channel_mgr
        .record_latency(channel.id, start.elapsed().as_millis() as u64)
        .await;
    state.router.active_requests.decrement(channel.id);

    // Spawn background task to extract real token counts from stream
    {
        let bg_logger = Arc::clone(&state.logger);
        let bg_quota_store = Arc::clone(&state.billing.quota_store);
        let bg_virtual_key_store = Arc::clone(&state.billing.virtual_key_store);
        let bg_channel_id = channel.id;
        let bg_cost_fn =
            channel.input_cost_per_mtok.is_some() || channel.output_cost_per_mtok.is_some();
        let bg_input_cost = channel.input_cost_per_mtok;
        let bg_output_cost = channel.output_cost_per_mtok;
        let bg_cost_per_token = channel.cost_per_token;
        let bg_vk_id = vk_id;
        crate::spawn_bg(async move {
            // Wait for chunks to accumulate (stream finishing)
            let chunks = {
                let mut prev_len = 0usize;
                let mut empty_rounds = 0u32;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let cur_len = {
                        let guard = telemetry_chunks.lock().unwrap_or_else(|e| e.into_inner());
                        guard.len()
                    };
                    if cur_len > 0 && cur_len == prev_len {
                        break telemetry_chunks.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    }
                    prev_len = cur_len;
                    if cur_len == 0 {
                        empty_rounds += 1;
                        if empty_rounds > 150 {
                            break telemetry_chunks.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        }
                    }
                }
            };
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
                if let Some(vk) = bg_vk_id {
                    let cost_cents = (real_cost.unwrap_or(0.0) * 100.0) as u64;
                    bg_virtual_key_store.accumulate_spend(vk, cost_cents).await;
                }
            }
        });
    }

    // Apply keepalive if configured
    let stream_resp = inject_passthrough_headers(stream_resp, upstream_headers);
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
    proxy_config: &ProxyConfig,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
) -> Response {
    let body_text = resp.text().await.unwrap_or_default();

    // Translate Gemini response to OpenAI format
    let response_body = if matches!(proxy_config.auth_style, AuthStyle::GeminiUrl) {
        if let Ok(v) = serde_json::from_str::<Value>(&body_text) {
            let translated = gemini_to_openai(&v, upstream_model);
            serde_json::to_string(&translated).unwrap_or(body_text)
        } else {
            body_text
        }
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
    // Accumulate usage into quota store
    state
        .billing
        .quota_store
        .accumulate_usage(
            channel.id,
            input_tokens,
            output_tokens,
            token_usage.cache_hit_tokens,
            token_usage.cache_miss_tokens,
            estimated_cost,
        )
        .await;
    // Attribute spend to the requesting virtual key (if any).
    if let Some(vk) = vk_id {
        let cost_cents = (estimated_cost.unwrap_or(0.0) * 100.0) as u64;
        state
            .billing
            .virtual_key_store
            .accumulate_spend(vk, cost_cents)
            .await;
    }
    state
        .logger
        .log(make_log(
            current_model,
            channel.id,
            &channel.name,
            channel.priority,
            attempt,
            trigger_reason,
            start.elapsed().as_millis() as u64,
            true,
            estimated_cost,
            input_tokens,
            output_tokens,
            token_usage.cache_hit_tokens,
            token_usage.cache_miss_tokens,
            request_id,
        ))
        .await;
    let _ = state
        .channel_mgr
        .record_latency(channel.id, start.elapsed().as_millis() as u64)
        .await;

    // Cache non-streaming responses
    let (cache_key, key_material) = RequestCache::compute_key(original_model, body);
    state
        .cache
        .request_cache
        .insert(cache_key, key_material, response_body.clone());
    state.cache.in_flight.complete(cache_key);

    state.router.active_requests.decrement(channel.id);
    inject_passthrough_headers(
        json_response(StatusCode::OK, response_body),
        upstream_headers,
    )
}
