pub mod anthropic;
pub mod cache;
pub mod gemini;
pub mod openai;
pub mod payload_rules;
pub mod rate_limiter;
pub mod stream;
pub mod translate;

use crate::channel::Channel;
use crate::log::DispatchLog;
use crate::proxy::stream::{
    all_channels_exhausted_response, json_response, keepalive_stream,
    sse_stream_response_with_telemetry,
};
use crate::router;
use crate::router::affinity::SessionAffinity;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::Utc;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Check if an affinity channel is still valid (available, supports model, not circuit-open).
async fn is_affinity_valid(
    affinity_id: Uuid,
    channels: &crate::channel::SharedChannels,
    requested_model: &str,
) -> bool {
    let guard = channels.read().await;
    let valid = guard.iter().any(|c| {
        if c.id != affinity_id {
            return false;
        }
        let mut c = c.clone();
        c.recover_if_expired();
        c.is_available()
            && (c.model_mapping.is_empty() || c.model_mapping.contains_key(requested_model))
    });
    drop(guard);
    valid
}

/// Build upstream URL for a channel given a path suffix.
pub fn upstream_url(channel: &Channel, path: &str) -> String {
    let base = channel.base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{}/{}", base, path)
}

/// Build a DispatchLog entry.
pub(crate) fn make_log(
    model: &str,
    channel_id: Uuid,
    channel_name: &str,
    channel_priority: u8,
    retry_count: u32,
    reason: Option<&str>,
    latency_ms: u64,
    success: bool,
    estimated_cost: Option<f64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_hit_tokens: Option<u64>,
    cache_miss_tokens: Option<u64>,
    request_id: Option<&str>,
) -> DispatchLog {
    DispatchLog {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        request_model: model.to_string(),
        channel_id,
        channel_name: channel_name.to_string(),
        channel_priority,
        retry_count: retry_count as u8,
        trigger_reason: reason.map(|s| s.to_string()),
        latency_ms,
        success,
        estimated_cost,
        input_tokens,
        output_tokens,
        cache_hit_tokens,
        cache_miss_tokens,
        request_id: request_id.map(|s| s.to_string()),
    }
}

/// Validate required fields in a chat completion request.
/// Returns OpenAI-compatible 400 error if validation fails.
pub(crate) fn validate_chat_request(body: &Value) -> Result<(), Response> {
    if body
        .get("model")
        .and_then(|m| m.as_str())
        .map_or(true, |s| s.is_empty())
    {
        return Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({
            "error": { "message": "Missing required field: model", "type": "invalid_request_error", "code": "missing_model" }
        }).to_string()));
    }
    if body
        .get("messages")
        .and_then(|m| m.as_array())
        .map_or(true, |a| a.is_empty())
    {
        return Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({
            "error": { "message": "Missing required field: messages", "type": "invalid_request_error", "code": "missing_messages" }
        }).to_string()));
    }
    Ok(())
}

/// Token usage extracted from an upstream API response.
pub(crate) struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
}

/// Extract token usage from an upstream response body.
pub(crate) fn extract_usage(body: &str) -> TokenUsage {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return TokenUsage {
                input_tokens: None,
                output_tokens: None,
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            }
        }
    };
    let usage = match v.get("usage") {
        Some(u) => u,
        None => {
            return TokenUsage {
                input_tokens: None,
                output_tokens: None,
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            }
        }
    };
    // OpenAI/DeepSeek: prompt_tokens / completion_tokens
    // Anthropic: input_tokens / output_tokens
    let input = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("input_tokens").and_then(|v| v.as_u64()));
    let output = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("output_tokens").and_then(|v| v.as_u64()));
    // DeepSeek: prompt_cache_hit_tokens / prompt_cache_miss_tokens
    // Anthropic: cache_read_input_tokens / cache_creation_input_tokens
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
        });
    let cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
        });
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_hit_tokens: cache_hit,
        cache_miss_tokens: cache_miss,
    }
}

/// Extract token usage from accumulated SSE stream data.
/// Looks for usage in the last few SSE chunks before [DONE].
fn extract_usage_from_stream(chunks: &[String]) -> TokenUsage {
    let mut anthropic_input: Option<u64> = None;
    let mut anthropic_output: Option<u64> = None;
    let mut anthropic_cache_read: Option<u64> = None;
    let mut anthropic_cache_creation: Option<u64> = None;

    for chunk in chunks.iter().rev().take(10) {
        if chunk.trim() == "[DONE]" || chunk.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(chunk) {
            let event_type = v.get("type").and_then(|t| t.as_str());
            // Anthropic message_delta: usage.output_tokens
            if event_type == Some("message_delta") {
                if let Some(usage) = v.get("usage") {
                    anthropic_output = usage.get("output_tokens").and_then(|t| t.as_u64());
                }
                continue;
            }
            // Anthropic message_start: message.usage.input_tokens + cache
            if event_type == Some("message_start") {
                if let Some(message) = v.get("message") {
                    if let Some(usage) = message.get("usage") {
                        anthropic_input = usage.get("input_tokens").and_then(|t| t.as_u64());
                        anthropic_cache_read = usage
                            .get("cache_read_input_tokens")
                            .and_then(|t| t.as_u64());
                        anthropic_cache_creation = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|t| t.as_u64());
                    }
                }
                continue;
            }
        }
        // OpenAI/DeepSeek: use extract_usage for chunks without Anthropic type
        let token_usage = extract_usage(chunk);
        if token_usage.input_tokens.is_some() || token_usage.output_tokens.is_some() {
            return token_usage;
        }
    }
    if anthropic_input.is_some() || anthropic_output.is_some() {
        return TokenUsage {
            input_tokens: anthropic_input,
            output_tokens: anthropic_output,
            cache_hit_tokens: anthropic_cache_read,
            cache_miss_tokens: anthropic_cache_creation,
        };
    }
    TokenUsage {
        input_tokens: None,
        output_tokens: None,
        cache_hit_tokens: None,
        cache_miss_tokens: None,
    }
}

/// Headers to skip when forwarding from IDE to upstream.

/// Estimate token count from request body for cost calculation.
/// Uses a rough heuristic: ~4 chars per token for English, reads max_tokens if present.
fn estimate_tokens(body: &Value, _is_stream: bool) -> u64 {
    // If max_tokens is specified, use that as output estimate
    let output_tokens = body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);

    // Estimate input tokens from message content (~4 chars per token)
    let input_chars = body
        .get("messages")
        .and_then(|m| {
            m.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|msg| {
                        msg.get("content").and_then(|c| {
                            if let Some(s) = c.as_str() {
                                Some(s.len())
                            } else {
                                // Array content blocks
                                Some(
                                    c.as_array()
                                        .map(|blocks| {
                                            blocks
                                                .iter()
                                                .filter_map(|b| {
                                                    b.get("text")
                                                        .and_then(|t| t.as_str())
                                                        .map(|t| t.len())
                                                })
                                                .sum::<usize>()
                                        })
                                        .unwrap_or(0),
                                )
                            }
                        })
                    })
                    .sum::<usize>()
            })
        })
        .unwrap_or(0);
    let input_tokens = (input_chars as u64) / 4;

    input_tokens + output_tokens
}
const SKIP_HEADERS: &[&str] = &[
    // Hop-by-hop headers
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "content-length",
    // Auth headers managed by dispatch
    "authorization",
    "x-api-key",
    "anthropic-version",
    // Content headers managed by dispatch
    "content-type",
];

/// Upstream response headers to pass through to the client.
const PASSTHROUGH_RESPONSE_HEADERS: &[&str] = &[
    "x-ratelimit-remaining",
    "x-ratelimit-limit",
    "x-ratelimit-reset",
    "x-ratelimit-limit-requests",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-reset-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-tokens",
    "x-ratelimit-reset-tokens",
    "anthropic-ratelimit-requests-limit",
    "anthropic-ratelimit-requests-remaining",
    "anthropic-ratelimit-requests-reset",
    "anthropic-ratelimit-tokens-limit",
    "anthropic-ratelimit-tokens-remaining",
    "anthropic-ratelimit-tokens-reset",
    "x-request-id",
];

/// Provider-specific authentication style.
#[derive(Clone)]
pub(crate) enum AuthStyle {
    OpenAI,
    Anthropic,
    Cookie,
    GeminiUrl,
}

/// Configuration for a proxy dispatch.
pub(crate) struct ProxyConfig {
    pub default_model: &'static str,
    pub upstream_path: &'static str,
    pub auth_style: AuthStyle,
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

// ---------------------------------------------------------------------------
// Decomposed dispatch helpers
// ---------------------------------------------------------------------------

/// Outcome of a single channel dispatch attempt.
enum AttemptOutcome {
    /// Got a response — dispatch should return it immediately.
    Respond(Response),
    /// Attempt failed; dispatch should retry with the next channel/attempt.
    Retry,
}

/// Metadata extracted from the incoming request at the start of dispatch.
struct RequestMeta<'a> {
    original_model: String,
    is_stream: bool,
    request_id: Option<&'a str>,
    session_id: Option<String>,
    affinity_channel: Option<Uuid>,
}

/// Extract request metadata (model, stream flag, session affinity, request ID).
async fn extract_request_meta<'a>(
    body: &Value,
    original_headers: &'a HeaderMap,
    state: &Arc<crate::proxy::openai::AppState>,
    default_model: &'static str,
) -> RequestMeta<'a> {
    let original_model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or(default_model)
        .to_string();
    let is_stream = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let request_id = original_headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok());
    let session_id = SessionAffinity::extract_session_id(body);
    let affinity_channel = if let Some(ref sid) = &session_id {
        state.session_affinity.get_channel(sid).await
    } else {
        None
    };
    RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
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
        let cache_key = crate::proxy::cache::RequestCache::cache_key(original_model, body);
        state.in_flight.complete(cache_key);
    }
    state
        .logger
        .log(make_log(
            original_model,
            Uuid::nil(),
            "none",
            0,
            total_attempts,
            Some("all_exhausted"),
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
    let cache_key = crate::proxy::cache::RequestCache::cache_key(original_model, body);
    if let Some(cached) = state.request_cache.get(cache_key) {
        tracing::info!("Cache hit for request");
        return Some(json_response(reqwest::StatusCode::OK, cached));
    }
    if !state.in_flight.register(cache_key) {
        // Another request is in flight — wait for it, then check cache
        state.in_flight.wait(cache_key).await;
        if let Some(cached) = state.request_cache.get(cache_key) {
            tracing::info!("Coalesced request served from cache");
            return Some(json_response(reqwest::StatusCode::OK, cached));
        }
        // Cache miss after wait — proceed normally as second attempt
        let _ = state.in_flight.register(cache_key);
    }
    None
}

/// Select a channel for the current attempt, preferring session affinity.
/// Returns `None` if no channel is available for the model.
async fn select_channel_for_attempt(
    affinity_channel: Option<Uuid>,
    channels: &crate::channel::SharedChannels,
    current_model: &str,
    routing_strategy: &str,
    active_requests: &Arc<crate::router::active_requests::ActiveRequests>,
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
    router::select_channel(
        channels.clone(),
        current_model,
        routing_strategy,
        active_requests,
    )
    .await
}

/// Log a failed attempt to a channel.
async fn log_attempt_failure(
    logger: &Arc<crate::log::DispatchLogger>,
    model: &str,
    channel: &Channel,
    attempt: u32,
    reason: &str,
    start: std::time::Instant,
    request_id: Option<&str>,
) {
    logger
        .log(make_log(
            model,
            channel.id,
            &channel.name,
            channel.priority,
            attempt,
            Some(reason),
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
    state.active_requests.decrement(channel.id);

    // Spawn background task to extract real token counts from stream
    {
        let bg_logger = Arc::clone(&state.logger);
        let bg_quota_store = Arc::clone(&state.quota_store);
        let bg_model = current_model.to_string();
        let bg_channel_id = channel.id;
        let bg_channel_name = channel.name.clone();
        let bg_priority = channel.priority;
        let bg_request_id = request_id.map(|s| s.to_string());
        let bg_cost_fn =
            channel.input_cost_per_mtok.is_some() || channel.output_cost_per_mtok.is_some();
        let bg_input_cost = channel.input_cost_per_mtok;
        let bg_output_cost = channel.output_cost_per_mtok;
        let bg_cost_per_token = channel.cost_per_token;
        crate::spawn_bg(async move {
            // Wait for chunks to accumulate (stream finishing)
            let chunks = {
                let mut prev_len = 0usize;
                let mut empty_rounds = 0u32;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let cur_len = {
                        let guard = telemetry_chunks.lock().await;
                        guard.len()
                    };
                    if cur_len > 0 && cur_len == prev_len {
                        break telemetry_chunks.lock().await.clone();
                    }
                    prev_len = cur_len;
                    if cur_len == 0 {
                        empty_rounds += 1;
                        if empty_rounds > 150 {
                            break telemetry_chunks.lock().await.clone();
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
                } else if let Some(rate_per_1k) = bg_cost_per_token {
                    Some(
                        ((input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)) as f64 / 1000.0)
                            * rate_per_1k,
                    )
                } else {
                    None
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
            }
        });
    }

    // Apply keepalive if configured
    let stream_resp = inject_passthrough_headers(stream_resp, upstream_headers);
    if let Some(secs) = state.stream_keepalive_secs {
        if secs > 0 {
            let (parts, body) = stream_resp.into_parts();
            let data_stream =
                body.into_data_stream()
                    .map(|r: Result<bytes::Bytes, axum::Error>| {
                        r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
) -> Response {
    let body_text = resp.text().await.unwrap_or_default();

    // Translate Gemini response to OpenAI format
    let response_body = if matches!(proxy_config.auth_style, AuthStyle::GeminiUrl) {
        if let Ok(v) = serde_json::from_str::<Value>(&body_text) {
            let translated = crate::proxy::translate::gemini_to_openai(&v, upstream_model);
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
    let cache_key = crate::proxy::cache::RequestCache::cache_key(original_model, body);
    state.request_cache.insert(cache_key, response_body.clone());
    state.in_flight.complete(cache_key);

    state.active_requests.decrement(channel.id);
    inject_passthrough_headers(
        json_response(StatusCode::OK, response_body),
        upstream_headers,
    )
}

/// Attempt to dispatch a request to a single channel.
/// Returns `Respond(response)` if a final response is ready, or `Retry` to try next.
async fn try_channel_attempt(
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
) -> AttemptOutcome {
    let upstream_model = channel.map_model(current_model);
    let mut upstream_body = body.clone();
    if let Some(obj) = upstream_body.as_object_mut() {
        obj.insert("model".to_string(), Value::String(upstream_model.clone()));
    }

    // Apply per-channel payload rules (defaults, overrides, strip)
    if let Some(rules) = state.payload_rules.get(channel.id) {
        upstream_body = rules.apply(upstream_body);
    }

    // Estimate tokens and check rate limiter before sending
    let estimated_tokens = estimate_tokens(&upstream_body, is_stream);
    let (allowed, rate_reason) = state.rate_limiter.check(channel.id, estimated_tokens);
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
            &rate_reason,
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
                "no_credential",
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
                    base, upstream_model
                )
            } else {
                format!("{}/v1beta/models/{}:generateContent", base, upstream_model)
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
    state.active_requests.increment(channel.id);

    let resp_result = req_builder.send().await;

    // Record rate limiter usage — the request was sent regardless of outcome
    state.rate_limiter.record(channel.id, estimated_tokens);

    let resp = match resp_result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(channel = %channel.name, error = %e, "Request failed");
            state.active_requests.decrement(channel.id);
            state.channel_mgr.mark_circuit_open(channel.id).await;
            log_attempt_failure(
                &state.logger,
                current_model,
                channel,
                attempt,
                "connection_error",
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
        state.active_requests.decrement(channel.id);
        state
            .channel_mgr
            .mark_circuit_open_with_retry(channel.id, retry_after_secs)
            .await;
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            "429",
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Retry;
    }

    if status.is_server_error() {
        tracing::warn!(channel = %channel.name, status = %status, "Server error");
        state.active_requests.decrement(channel.id);
        state.channel_mgr.mark_circuit_open(channel.id).await;
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            &format!("{}xx", status.as_u16() / 100),
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Retry;
    }

    if !status.is_success() {
        let status_code = status;
        let body_text = resp.text().await.unwrap_or_default();
        state.active_requests.decrement(channel.id);
        log_attempt_failure(
            &state.logger,
            current_model,
            channel,
            attempt,
            &status_code.as_u16().to_string(),
            start,
            request_id,
        )
        .await;
        return AttemptOutcome::Respond(json_response(status_code, body_text));
    }

    // Success — record session affinity if applicable
    if let Some(ref sid) = session_id {
        state.session_affinity.set_channel(sid, channel.id).await;
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
                .quota_store
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
        )
        .await;
        AttemptOutcome::Respond(response)
    }
}

/// Shared dispatch logic for both OpenAI and Anthropic proxy handlers.
/// Implements priority-based channel selection, hot retry, circuit breaking,
/// model fallback chains, SSE streaming, and header passthrough.
pub(crate) async fn dispatch(
    state: &Arc<crate::proxy::openai::AppState>,
    original_headers: &HeaderMap,
    body: &Value,
    proxy_config: &ProxyConfig,
) -> axum::response::Response {
    let meta =
        extract_request_meta(body, original_headers, state, proxy_config.default_model).await;
    let RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
    } = meta;

    let max_retries = state.max_retries;
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
        router::fallback::resolve_fallback_chain(&original_model, &state.model_fallbacks);

    let mut total_attempts: u32 = 0;
    let max_total_attempts = max_retries * fallback_chain.len() as u32;
    let deadline =
        start + std::time::Duration::from_secs(state.request_timeout_secs.unwrap_or(120));

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
                &state.routing_strategy,
                &state.active_requests,
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
                proxy_config,
                &channel,
                current_model,
                &original_model,
                is_stream,
                &session_id,
                start,
                attempt,
                request_id,
            )
            .await
            {
                AttemptOutcome::Respond(response) => return response,
                AttemptOutcome::Retry => continue,
            }
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

    #[test]
    fn extract_deepseek_usage_with_cache() {
        let body = r#"{"id":"chatcmpl-123","usage":{"prompt_tokens":1500,"completion_tokens":300,"prompt_cache_hit_tokens":1200,"prompt_cache_miss_tokens":300}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(300));
        assert_eq!(usage.cache_hit_tokens, Some(1200));
        assert_eq!(usage.cache_miss_tokens, Some(300));
    }

    #[test]
    fn extract_anthropic_usage_with_cache() {
        let body = r#"{"id":"msg_123","usage":{"input_tokens":2000,"output_tokens":500,"cache_read_input_tokens":1800,"cache_creation_input_tokens":200}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(2000));
        assert_eq!(usage.output_tokens, Some(500));
        assert_eq!(usage.cache_hit_tokens, Some(1800));
        assert_eq!(usage.cache_miss_tokens, Some(200));
    }

    #[test]
    fn extract_openai_usage_no_cache() {
        let body = r#"{"id":"chatcmpl-456","usage":{"prompt_tokens":100,"completion_tokens":50}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_miss_tokens, None);
    }

    #[test]
    fn extract_usage_no_usage_field() {
        let body = r#"{"id":"chatcmpl-789","choices":[]}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_miss_tokens, None);
    }

    #[test]
    fn extract_stream_deepseek_usage() {
        let chunks = vec![
            r#"{"id":"chatcmpl-1","choices":[{"delta":{"content":"Hi"}}]}"#.to_string(),
            r#"{"id":"chatcmpl-1","usage":{"prompt_tokens":800,"completion_tokens":100,"prompt_cache_hit_tokens":600,"prompt_cache_miss_tokens":200}}"#.to_string(),
        ];
        let usage = extract_usage_from_stream(&chunks);
        assert_eq!(usage.input_tokens, Some(800));
        assert_eq!(usage.output_tokens, Some(100));
        assert_eq!(usage.cache_hit_tokens, Some(600));
        assert_eq!(usage.cache_miss_tokens, Some(200));
    }

    #[test]
    fn extract_stream_anthropic_usage() {
        let chunks = vec![
            r#"{"type":"message_start","message":{"usage":{"input_tokens":3000,"cache_read_input_tokens":2500,"cache_creation_input_tokens":500}}}"#.to_string(),
            r#"{"type":"message_delta","usage":{"output_tokens":400}}"#.to_string(),
        ];
        let usage = extract_usage_from_stream(&chunks);
        assert_eq!(usage.input_tokens, Some(3000));
        assert_eq!(usage.output_tokens, Some(400));
        assert_eq!(usage.cache_hit_tokens, Some(2500));
        assert_eq!(usage.cache_miss_tokens, Some(500));
    }
}
