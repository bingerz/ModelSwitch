use axum::http::HeaderMap;
use axum::response::Response;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::stream::json_response;

use super::response::{extract_passthrough_headers, handle_json_success, handle_streaming_success};
use super::{
    estimate_tokens, make_log, upstream_url, AuthStyle, FailureReason, ProxyConfig, SKIP_HEADERS,
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
    reserved_cents: u64,
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
    let (allowed, rate_reason) = state
        .limits
        .rate_limiter
        .check(channel.id, estimated_tokens);
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

    // Inject stream_options.include_usage for OpenAI-compatible streaming requests
    // to ensure upstream returns token usage in the final SSE chunk
    if is_stream && !matches!(effective_auth, AuthStyle::GeminiUrl | AuthStyle::Anthropic) {
        if let Some(obj) = upstream_body.as_object_mut() {
            let needs_injection = obj
                .get("stream_options")
                .and_then(|v| v.as_object())
                .map(|so| !so.contains_key("include_usage"))
                .unwrap_or(true);
            if needs_injection {
                if let Some(existing) = obj
                    .get_mut("stream_options")
                    .and_then(|v| v.as_object_mut())
                {
                    existing.insert("include_usage".to_string(), serde_json::Value::Bool(true));
                } else {
                    let mut so = serde_json::Map::new();
                    so.insert("include_usage".to_string(), serde_json::Value::Bool(true));
                    obj.insert("stream_options".to_string(), serde_json::Value::Object(so));
                }
            }
        }
    }

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

    let mut req_builder = state.http_pool.get().post(&url).json(&upstream_body);

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

    let resp_result = if is_stream {
        match state.gateway.stream_ttft_timeout_secs {
            Some(secs) if secs > 0 => {
                let send_future = req_builder.send();
                match tokio::time::timeout(std::time::Duration::from_secs(secs), send_future).await
                {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        tracing::warn!(
                            channel = %channel.name,
                            ttft_timeout_secs = secs,
                            "TTFT timeout exceeded — aborting channel"
                        );
                        state
                            .limits
                            .rate_limiter
                            .record(channel.id, estimated_tokens);
                        state.router.active_requests.decrement(channel.id);
                        log_attempt_failure(
                            &state.logger,
                            current_model,
                            channel,
                            attempt,
                            FailureReason::Timeout,
                            start,
                            request_id,
                        )
                        .await;
                        return AttemptOutcome::Retry;
                    }
                }
            }
            _ => req_builder.send().await,
        }
    } else {
        req_builder.send().await
    };

    // Record rate limiter usage — the request was sent regardless of outcome
    state
        .limits
        .rate_limiter
        .record(channel.id, estimated_tokens);

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
        state
            .router
            .session_affinity
            .set_channel(sid, channel.id)
            .await;
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
                .billing
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
            vk_id,
            reserved_cents,
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
            reserved_cents,
        )
        .await;
        AttemptOutcome::Respond(response)
    }
}
