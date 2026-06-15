use axum::body::Body;
use axum::http::HeaderMap;
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
    estimate_tokens, make_log, upstream_url, AuthStyle, FailureReason, ProxyConfig,
    PASSTHROUGH_RESPONSE_HEADERS, SKIP_HEADERS,
};

/// Validate and sanitize a model name for safe URL interpolation.
///
/// Model names must be flat identifiers (e.g. `gpt-4`, `claude-3-opus-20240229`,
/// `gemini-1.5-pro`). This function:
/// - Removes any character outside `[a-zA-Z0-9._:-]`
/// - Strips leading/trailing dots and collapses consecutive dots to prevent
///   path-traversal sequences (`..`)
/// - Rejects empty results (returns "unknown" as fallback)
fn sanitize_model_for_url(model: &str) -> String {
    // Allow only alphanumeric, hyphens, dots, underscores, colons
    let mut sanitized: String = model
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'))
        .collect();

    // Collapse consecutive dots and strip leading/trailing dots
    while sanitized.contains("..") {
        sanitized = sanitized.replace("..", ".");
    }
    sanitized = sanitized.trim_matches('.').to_string();

    // Fallback for empty or all-unsafe input
    if sanitized.is_empty() {
        tracing::warn!(original = %model, "Model name was entirely unsafe, using fallback");
        return "unknown".to_string();
    }

    if sanitized != model {
        tracing::warn!(
            original = %model,
            sanitized = %sanitized,
            "Model name contained unsafe characters, sanitized for URL"
        );
    }
    sanitized
}

/// Outcome of a single channel dispatch attempt.
pub(super) enum AttemptOutcome {
    /// Got a response — dispatch should return it immediately.
    Respond(Response),
    /// Attempt failed; dispatch should retry with the next channel/attempt.
    Retry,
}

/// Extract passthrough headers from an upstream response.
fn extract_passthrough_headers(resp: &reqwest::Response) -> Vec<(String, String)> {
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
fn inject_passthrough_headers(mut resp: Response, headers: &[(String, String)]) -> Response {
    for (name, value) in headers {
        if let Ok(header_name) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(header_value) = axum::http::HeaderValue::from_str(value) {
                resp.headers_mut().append(header_name, header_value);
            }
        }
    }
    resp
}

/// Log a failed attempt to a channel.
async fn log_attempt_failure(
    logger: &Arc<crate::log::DispatchLogger>,
    model: &str,
    channel: &Channel,
    attempt: u32,
    reason: FailureReason,
    start: std::time::Instant,
    request_id: Option<&str>,
) {
    let reason_str = reason.log_str();
    logger
        .log(make_log(
            model,
            channel.id,
            &channel.name,
            channel.priority,
            attempt,
            Some(&reason_str),
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
}

/// Handle a successful streaming (SSE) response from upstream.
/// Logs the attempt, spawns a background task to extract real token usage,
/// and applies keepalive if configured.
#[allow(clippy::too_many_arguments)]
async fn handle_streaming_success(
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
                .map(|r: Result<bytes::Bytes, axum::Error>| {
                    r.map_err(std::io::Error::other)
                });
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
async fn handle_json_success(
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
        .billing.quota_store
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
            .billing.virtual_key_store
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
    state.cache.request_cache.insert(cache_key, key_material, response_body.clone());
    state.cache.in_flight.complete(cache_key);

    state.router.active_requests.decrement(channel.id);
    inject_passthrough_headers(
        json_response(StatusCode::OK, response_body),
        upstream_headers,
    )
}

/// Attempt to dispatch a request to a single channel.
/// Returns `Respond(response)` if a final response is ready, or `Retry` to try next.
#[allow(clippy::too_many_arguments)]
pub(super) async fn try_channel_attempt(
    state: &Arc<crate::proxy::openai::AppState>,
    original_headers: &HeaderMap,
    body: &Value,
    proxy_config: &ProxyConfig,
    channel: &Channel,
    current_model: &str,
    original_model: &str,
    is_stream: bool,
    session_id: &Option<String>,
    start: std::time::Instant,
    attempt: u32,
    request_id: Option<&str>,
    vk_id: Option<Uuid>,
) -> AttemptOutcome {
    let upstream_model = channel.map_model(current_model);
    let mut upstream_body = body.clone();
    if let Some(obj) = upstream_body.as_object_mut() {
        obj.insert("model".to_string(), Value::String(upstream_model.clone()));
    }

    // Apply per-channel payload rules (defaults, overrides, strip)
    if let Some(rules) = state.limits.payload_rules.get(channel.id) {
        upstream_body = rules.apply(upstream_body);
    }

    // Estimate tokens and check rate limiter before sending
    let estimated_tokens = estimate_tokens(&upstream_body, is_stream);
    let (allowed, rate_reason) = state.limits.rate_limiter.check(channel.id, estimated_tokens);
    if !allowed {
        tracing::warn!(
            channel = %channel.name,
            reason = rate_reason,
            tokens = estimated_tokens,
            "Rate limited, skipping channel"
        );
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            FailureReason::RateLimited,
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Retry;
    }

    // Determine effective auth style: cookie overrides if credential type is web_session
    let effective_auth =
        if channel.credential.cred_type == crate::channel::CredentialType::WebSession {
            AuthStyle::Cookie
        } else {
            proxy_config.auth_style.clone()
        };

    let api_key = match state.channel_mgr.get_credential(channel.id).await {
        Some(key) => key,
        None => {
            tracing::error!(channel = %channel.name, "No credential found");
            state.channel_mgr.mark_circuit_open(channel.id).await;
            log_attempt_failure(
                &state.logger,
                current_model,
                channel,
                attempt,
                FailureReason::NoCredential,
                start,
                request_id,
            )
            .await;
            return AttemptOutcome::Retry;
        }
    };

    // Build URL: Gemini embeds model in URL; others use base_url + path
    let url = match &effective_auth {
        AuthStyle::GeminiUrl => {
            let base = channel.base_url.trim_end_matches('/');
            if is_stream {
                format!(
                    "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
                    base,
                    sanitize_model_for_url(&upstream_model)
                )
            } else {
                format!(
                    "{}/v1beta/models/{}:generateContent",
                    base,
                    sanitize_model_for_url(&upstream_model)
                )
            }
        }
        _ => upstream_url(channel, proxy_config.upstream_path),
    };

    let mut req_builder = state.http_client.post(&url).json(&upstream_body);

    // Forward original request headers (excluding hop-by-hop and auth headers)
    for (name, value) in original_headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            req_builder = req_builder.header(name.clone(), value.clone());
        }
    }

    // Set provider-specific auth headers
    req_builder = match &effective_auth {
        AuthStyle::OpenAI => req_builder
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json"),
        AuthStyle::Anthropic => req_builder
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json"),
        AuthStyle::Cookie => req_builder
            .header("Cookie", &api_key)
            .header("Content-Type", "application/json"),
        AuthStyle::GeminiUrl => req_builder
            .header("x-goog-api-key", &api_key)
            .header("Content-Type", "application/json"),
    };

    if is_stream {
        req_builder = req_builder.header("Accept", "text/event-stream");
    }

    // Track active request count for least-busy routing
    state.router.active_requests.increment(channel.id);

    let resp_result = req_builder.send().await;

    // Record rate limiter usage — the request was sent regardless of outcome
    state.limits.rate_limiter.record(channel.id, estimated_tokens);

    let resp = match resp_result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(channel = %channel.name, error = %e, "Request failed");
            state.router.active_requests.decrement(channel.id);
            state.channel_mgr.mark_circuit_open(channel.id).await;
            log_attempt_failure(
                &state.logger,
                current_model,
                channel,
                attempt,
                FailureReason::ConnectionError,
                start,
                request_id,
            )
            .await;
            return AttemptOutcome::Retry;
        }
    };

    let status = resp.status();

    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_secs = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        tracing::warn!(channel = %channel.name, retry_after_secs, "Rate limited (429)");
        state.router.active_requests.decrement(channel.id);
        state
            .channel_mgr
            .mark_circuit_open_with_retry(channel.id, retry_after_secs)
            .await;
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            FailureReason::RateLimited,
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Retry;
    }

    if status.is_server_error() {
        tracing::warn!(channel = %channel.name, status = %status, "Server error");
        state.router.active_requests.decrement(channel.id);
        state.channel_mgr.mark_circuit_open(channel.id).await;
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            FailureReason::ServerError,
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Retry;
    }

    if !status.is_success() {
        let status_code = status;
        let body_text = resp.text().await.unwrap_or_default();
        state.router.active_requests.decrement(channel.id);
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            FailureReason::ClientError(status_code.as_u16()),
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Respond(json_response(status_code, body_text));
    }

    // Success — record session affinity if applicable
    if let Some(ref sid) = session_id {
        state.router.session_affinity.set_channel(sid, channel.id).await;
    }

    // Extract upstream response headers for passthrough before consuming body
    let upstream_headers = extract_passthrough_headers(&resp);

    // Passive rate-limit extraction — update quota store from response headers
    {
        if let Some(qh) = crate::quota::collectors::response_header::QuotaHeaders::extract(
            channel.provider.as_str(),
            resp.headers(),
        ) {
            state
                .billing.quota_store
                .update_rate_limits(
                    channel.id,
                    qh.remaining_requests,
                    qh.limit_requests,
                    qh.remaining_tokens,
                    qh.limit_tokens,
                )
                .await;
        }
    }

    let trigger_reason = if current_model != original_model {
        Some("model_fallback".to_string())
    } else {
        None
    };

    if is_stream {
        let response = handle_streaming_success(
            state,
            channel,
            resp,
            body,
            proxy_config,
            current_model,
            &upstream_model,
            attempt,
            trigger_reason.as_deref(),
            start,
            request_id,
            &upstream_headers,
            vk_id,
        )
        .await;
        AttemptOutcome::Respond(response)
    } else {
        let response = handle_json_success(
            state,
            channel,
            resp,
            body,
            original_model,
            proxy_config,
            current_model,
            &upstream_model,
            attempt,
            trigger_reason.as_deref(),
            start,
            request_id,
            &upstream_headers,
            vk_id,
        )
        .await;
        AttemptOutcome::Respond(response)
    }
}
