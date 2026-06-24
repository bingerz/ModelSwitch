use axum::http::HeaderMap;
use axum::response::Response;
use reqwest::StatusCode;
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::proxy::stream::json_response;

use super::provider::ProviderAdaptor;
use super::response::{extract_passthrough_headers, handle_json_success, handle_streaming_success};
use super::translate::translate_request;
use super::{estimate_tokens, make_log, FailureReason, RequestFormat, SKIP_HEADERS};

/// Outcome of a single channel dispatch attempt.
pub(super) enum AttemptOutcome {
    /// Got a response — dispatch should return it immediately.
    Respond(Response),
    /// Attempt failed; dispatch should retry with the next channel/attempt.
    Retry,
    /// Upstream rejected the prompt because it exceeded the model's context
    /// window. The dispatch loop should skip remaining retries for this model
    /// and move to the next model in the (possibly extended) fallback chain.
    ContextOverflow,
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
    state: &Arc<crate::proxy::AppState>,
    original_headers: &HeaderMap,
    body: &Value,
    provider: &dyn ProviderAdaptor,
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
    cache_key: u128,
    cache_key_material: &str,
    request_format: RequestFormat,
) -> AttemptOutcome {
    let upstream_model = channel.map_model(current_model);
    let upstream_format = provider.provider_request_format();
    let has_payload_rules = state.limits.payload_rules.has_rules(channel.id);
    let model_needs_change = upstream_model != current_model;

    // Determine if any mutation is needed — avoid cloning a potentially large
    // body when no modifications are required (copy-on-write via Cow).
    let needs_mutation = model_needs_change || has_payload_rules;

    let mut upstream_body: Cow<'_, Value> = if needs_mutation {
        let mut cloned = body.clone();
        if let Some(obj) = cloned.as_object_mut() {
            if model_needs_change {
                obj.insert("model".to_string(), Value::String(upstream_model.clone()));
            }
        }
        // Apply per-channel payload rules (defaults, overrides, strip),
        // including any per-model rules whose glob/protocol match.
        if has_payload_rules {
            cloned = state.limits.payload_rules.apply_for_model(
                channel.id,
                cloned,
                current_model,
                channel.provider.as_str(),
            );
        }
        Cow::Owned(cloned)
    } else {
        Cow::Borrowed(body)
    };

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

    // Determine whether to use cookie-based auth (web session) or native provider auth
    let is_web_session = channel.credential.cred_type == crate::channel::CredentialType::WebSession;

    // Inject stream_options.include_usage for providers that support it (OpenAI-compatible)
    // to ensure upstream returns token usage in the final SSE chunk
    if is_stream && provider.inject_stream_usage() {
        let needs_injection = upstream_body
            .get("stream_options")
            .and_then(|v| v.as_object())
            .map(|so| !so.contains_key("include_usage"))
            .unwrap_or(true);

        if needs_injection {
            if let Some(obj) = upstream_body.to_mut().as_object_mut() {
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

    // If the request format differs from the channel's upstream provider format,
    // translate the body before provider-specific transformation.
    if request_format != upstream_format {
        tracing::info!(
            from = ?request_format,
            to = ?upstream_format,
            channel = %channel.name,
            "Translating request body"
        );
        upstream_body = Cow::Owned(translate_request(
            &upstream_body,
            request_format,
            upstream_format,
        ));
    }

    // Transform request body for provider-specific format (e.g., Gemini)
    let upstream_body = provider.transform_request(&upstream_body);

    // Build URL via provider (Gemini embeds model in URL; others use base_url + path)
    let url = provider.build_url(&channel.base_url, &upstream_model, is_stream);

    let pool_guard = if let Some(ref proxy_url) = channel.proxy_url {
        match state.http_pool.proxied_pooled_client(proxy_url) {
            Ok(guard) => guard,
            Err(e) => {
                tracing::error!(
                    channel = %channel.name,
                    proxy_url = %proxy_url,
                    error = %e,
                    "Failed to build proxied client"
                );
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
        }
    } else {
        state.http_pool.get()
    };
    let mut req_builder = pool_guard.post(&url).json(&upstream_body);

    // Forward original request headers (excluding hop-by-hop and auth headers)
    for (name, value) in original_headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            req_builder = req_builder.header(name.clone(), value.clone());
        }
    }

    // Set provider-specific auth headers
    req_builder = provider.apply_auth(req_builder, &api_key, is_web_session);

    if is_stream {
        req_builder = req_builder.header("Accept", "text/event-stream");
    }

    // Track active request count for least-busy routing via RAII guard.
    // The guard decrements automatically on drop — no manual decrement needed.
    let _active_guard = state.router.active_requests.acquire(channel.id);

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
                        state.channel_mgr.mark_circuit_open(channel.id).await;
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

    // Record TTFT — time from dispatch start to first byte from upstream
    crate::metrics::ttft_seconds()
        .with_label_values(&[channel.provider.as_str(), current_model])
        .observe(start.elapsed().as_secs_f64());

    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_secs = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        tracing::warn!(channel = %channel.name, retry_after_secs, "Rate limited (429)");
        // Record per-model cooldown so other models on this channel remain available
        state
            .channel_mgr
            .mark_model_rate_limited(channel.id, current_model, retry_after_secs)
            .await;
        // Also open the channel circuit breaker (existing behavior — may be refined later)
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

        // Check for context window exceeded error before falling back to the
        // generic client-error path. When detected, signal the dispatch loop
        // to skip remaining retries for this model and try the next model in
        // the fallback chain (which may include larger-context models via
        // `context_window_fallbacks`).
        if is_context_window_error(status_code, &body_text) {
            tracing::info!(
                channel = %channel.name,
                model = %current_model,
                "Context window exceeded — trying context fallback"
            );
            log_attempt_failure(
                &state.logger,
                current_model,
                channel,
                attempt,
                FailureReason::ContextOverflow,
                start,
                request_id,
            )
            .await;
            return AttemptOutcome::ContextOverflow;
        }

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

    // Extract upstream response headers for passthrough before consuming body.
    // Uses the configurable list from gateway state (falls back to built-in
    // defaults when the user hasn't customized it).
    let upstream_headers = extract_passthrough_headers(&resp, &state.gateway.passthrough_headers);

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
        let needs_proto_translate =
            request_format != upstream_format && !provider.is_gemini_stream();
        let protocol_translation = if needs_proto_translate {
            Some((request_format, upstream_format))
        } else {
            None
        };

        let response = handle_streaming_success(
            state,
            channel,
            resp,
            body,
            provider,
            current_model,
            &upstream_model,
            attempt,
            trigger_reason.as_deref(),
            start,
            request_id,
            &upstream_headers,
            vk_id,
            reserved_cents,
            original_model,
            cache_key,
            cache_key_material,
            pool_guard,
            _active_guard,
            protocol_translation,
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
            provider,
            current_model,
            &upstream_model,
            attempt,
            trigger_reason.as_deref(),
            start,
            request_id,
            &upstream_headers,
            vk_id,
            reserved_cents,
            cache_key,
            cache_key_material,
            pool_guard,
            _active_guard,
            request_format,
            upstream_format,
        )
        .await;
        AttemptOutcome::Respond(response)
    }
}

/// Detect if an upstream error response indicates the prompt exceeded the
/// model's context window.
///
/// Context-length errors are typically returned as `400 Bad Request` (OpenAI,
/// Anthropic, most OpenAI-compatible providers) or `413 Request Entity Too
/// Large`. The body is inspected for common error indicators across providers.
fn is_context_window_error(status: StatusCode, body: &str) -> bool {
    // Context errors are typically 400 (Bad Request) or 413 (Payload Too Large).
    if status != StatusCode::BAD_REQUEST && status != StatusCode::PAYLOAD_TOO_LARGE {
        return false;
    }

    // Check for common context-length error indicators across providers.
    let body_lower = body.to_lowercase();

    // OpenAI: "This model's maximum context length is..."
    // OpenAI: "Please reduce the length of the messages"
    // OpenAI error code: "context_length_exceeded"
    if body_lower.contains("maximum context length")
        || body_lower.contains("context_length_exceeded")
        || body_lower.contains("context length exceeded")
        || body_lower.contains("reduce the length of the messages")
    {
        return true;
    }

    // Anthropic: "prompt is too long"
    if body_lower.contains("prompt is too long") {
        return true;
    }

    // Generic: "context window" paired with "exceed" or "limit"
    if body_lower.contains("context window")
        && (body_lower.contains("exceed") || body_lower.contains("limit"))
    {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::is_context_window_error;
    use reqwest::StatusCode;

    #[test]
    fn detects_openai_context_length_exceeded_code() {
        let body = r#"{"error":{"code":"context_length_exceeded","message":"..."}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn detects_openai_maximum_context_length_message() {
        let body = r#"{"error":{"message":"This model's maximum context length is 8192 tokens."}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn detects_openai_reduce_messages_message() {
        let body = r#"{"error":{"message":"Please reduce the length of the messages."}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn detects_anthropic_prompt_too_long() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn detects_generic_context_window_exceeded() {
        let body = r#"{"error":{"message":"context window limit exceeded"}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn detects_413_payload_too_large() {
        let body = r#"{"error":{"message":"context window exceeded"}}"#;
        assert!(is_context_window_error(StatusCode::PAYLOAD_TOO_LARGE, body));
    }

    #[test]
    fn rejects_400_without_context_indicators() {
        let body = r#"{"error":{"message":"Invalid model name"}}"#;
        assert!(!is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn rejects_success_statuses() {
        let body = r#"{"ok":true}"#;
        assert!(!is_context_window_error(StatusCode::OK, body));
    }

    #[test]
    fn rejects_429_rate_limit() {
        let body = r#"{"error":{"message":"Rate limited"}}"#;
        assert!(!is_context_window_error(
            StatusCode::TOO_MANY_REQUESTS,
            body
        ));
    }

    #[test]
    fn rejects_500_server_error() {
        let body = r#"{"error":{"message":"internal error"}}"#;
        assert!(!is_context_window_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            body
        ));
    }

    #[test]
    fn detects_is_case_insensitive() {
        let body = r#"{"error":{"message":"CONTEXT LENGTH EXCEEDED"}}"#;
        assert!(is_context_window_error(StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn rejects_empty_body() {
        assert!(!is_context_window_error(StatusCode::BAD_REQUEST, ""));
    }
}
