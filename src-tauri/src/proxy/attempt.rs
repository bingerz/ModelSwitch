use axum::http::HeaderMap;
use axum::response::Response;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;
use uuid::Uuid;

use crate::channel::Channel;
use crate::http_pool::PooledClient;
use crate::proxy::stream::json_response;
use crate::router::active_requests::ActiveRequestGuard;

use super::payload_rules::ChannelPayloadRules;
use super::provider::ProviderAdaptor;
use super::response::{extract_passthrough_headers, handle_json_success, handle_streaming_success};
use super::translate::translate_request;
use super::{
    estimate_tokens, make_log, DispatchLogInput, FailureReason, RequestFormat, SKIP_HEADERS,
};

/// Context bundle for channel dispatch — eliminates the 17-parameter signature.
///
/// Constructed by the dispatch loop and passed to `try_channel_attempt` and the
/// extracted helpers (`build_and_send_request`, `check_error_status`,
/// `check_sse_bootstrap`). All fields are references or `Copy` types, so the
/// struct itself is `Copy` and can be passed by value without cloning.
#[derive(Clone, Copy)]
pub(super) struct DispatchContext<'a> {
    pub original_headers: &'a HeaderMap,
    pub body: &'a Value,
    pub provider: &'a dyn ProviderAdaptor,
    pub channel: &'a Channel,
    pub current_model: &'a str,
    pub original_model: &'a str,
    pub is_stream: bool,
    pub session_id: &'a Option<String>,
    pub start: std::time::Instant,
    pub attempt: u32,
    pub request_id: Option<&'a str>,
    pub vk_id: Option<Uuid>,
    pub reserved_cents: u64,
    pub cache_key: u128,
    pub cache_key_material: &'a str,
    pub request_format: RequestFormat,
}

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
#[allow(clippy::too_many_arguments)]
async fn log_attempt_failure(
    logger: &Arc<crate::log::DispatchLogger>,
    model: &str,
    channel: &Channel,
    attempt: u32,
    reason: FailureReason,
    start: std::time::Instant,
    request_id: Option<&str>,
    virtual_key_id: Option<String>,
) {
    let reason_str = reason.log_str();
    let log_input = DispatchLogInput {
        model,
        channel_id: channel.id,
        channel_name: &channel.name,
        channel_priority: channel.priority,
        retry_count: attempt,
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
    logger.log(make_log(&log_input)).await;
}

/// Record a channel failure (circuit breaker + cooldown + log) and return `Retry`.
/// Centralises the failure-handling pattern to avoid drift across 6+ call sites.
#[allow(clippy::too_many_arguments)]
async fn fail_and_retry(
    state: &Arc<crate::proxy::AppState>,
    channel: &Channel,
    current_model: &str,
    attempt: u32,
    reason: FailureReason,
    start: std::time::Instant,
    request_id: Option<&str>,
    vk_id: Option<Uuid>,
) -> AttemptOutcome {
    state.channel_mgr.mark_circuit_open(channel.id).await;
    state
        .router
        .cooldown_tracker
        .record_attempt(channel.id, false);
    log_attempt_failure(
        &state.logger,
        current_model,
        channel,
        attempt,
        reason,
        start,
        request_id,
        vk_id.map(|id| id.to_string()),
    )
    .await;
    AttemptOutcome::Retry
}

/// Headers denied when injecting per-channel custom headers.
///
/// Security-sensitive headers are denied to prevent credential leakage
/// or proxy metadata injection.
const CUSTOM_HEADER_DENYLIST: &[&str] = &[
    "host",
    "transfer-encoding",
    "content-length",
    "connection",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "authorization",
    "x-api-key",
    "cookie",
    "forwarded",
];

/// Build the reqwest RequestBuilder for an upstream channel request.
///
/// Handles URL construction, pool client selection, header forwarding,
/// auth injection, and custom channel headers. Returns the [`RequestBuilder`]
/// and the [`PooledClient`] guard that must outlive the request.
async fn build_upstream_request(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    upstream_body: &Value,
    upstream_model: &str,
    api_key: &str,
) -> Result<(reqwest::RequestBuilder, PooledClient), AttemptOutcome> {
    // Build URL via provider (Gemini embeds model in URL; others use base_url + path)
    let url = ctx
        .provider
        .build_url(&ctx.channel.base_url, upstream_model, ctx.is_stream);

    let pool_guard = if let Some(ref proxy_url) = ctx.channel.proxy_url {
        match state.http_pool.proxied_pooled_client(proxy_url) {
            Ok(guard) => guard,
            Err(e) => {
                tracing::error!(
                    channel = %ctx.channel.name,
                    proxy_url = %proxy_url,
                    error = %e,
                    "Failed to build proxied client"
                );
                return Err(fail_and_retry(
                    state,
                    ctx.channel,
                    ctx.current_model,
                    ctx.attempt,
                    FailureReason::ConnectionError,
                    ctx.start,
                    ctx.request_id,
                    ctx.vk_id,
                )
                .await);
            }
        }
    } else {
        state.http_pool.get()
    };
    let mut req_builder = pool_guard.post(&url).json(upstream_body);

    // Forward original request headers (excluding hop-by-hop and auth headers)
    for (name, value) in ctx.original_headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            req_builder = req_builder.header(name.clone(), value.clone());
        }
    }

    // Set provider-specific auth headers
    let is_web_session =
        ctx.channel.credential.cred_type == crate::channel::CredentialType::WebSession;
    req_builder = ctx
        .provider
        .apply_auth(req_builder, api_key, is_web_session);

    // Inject per-channel custom headers.
    for (name, value) in &ctx.channel.headers {
        let name_lower = name.to_lowercase();
        if CUSTOM_HEADER_DENYLIST.contains(&name_lower.as_str()) {
            tracing::warn!(header = %name, "Skipping denylisted custom header");
            continue;
        }
        if let (Ok(hn), Ok(hv)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(value),
        ) {
            req_builder = req_builder.header(hn, hv);
        }
    }

    if ctx.is_stream {
        req_builder = req_builder.header("Accept", "text/event-stream");
    }

    Ok((req_builder, pool_guard))
}

/// Send the prepared request with an optional first-byte (TTFT) timeout.
///
/// Acquires the active-request RAII guard, sends with a TTFT timeout when
/// streaming is enabled and a positive timeout is configured, records rate
/// limiter consumption, and translates transport errors into
/// [`AttemptOutcome::Retry`] through [`fail_and_retry`].
async fn send_with_timeout(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    req_builder: reqwest::RequestBuilder,
    estimated_tokens: u64,
) -> Result<(reqwest::Response, ActiveRequestGuard), AttemptOutcome> {
    // Track active request count for least-busy routing via RAII guard.
    let active_guard = state.router.active_requests.acquire(ctx.channel.id);

    let resp_result = if ctx.is_stream {
        match state.gateway.stream_ttft_timeout_secs {
            Some(secs) if secs > 0 => {
                let send_future = req_builder.send();
                match tokio::time::timeout(std::time::Duration::from_secs(secs), send_future).await
                {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        tracing::warn!(
                            channel = %ctx.channel.name,
                            ttft_timeout_secs = secs,
                            "TTFT timeout exceeded — aborting channel"
                        );
                        state
                            .limits
                            .rate_limiter
                            .record(ctx.channel.id, estimated_tokens)
                            .await;
                        return Err(fail_and_retry(
                            state,
                            ctx.channel,
                            ctx.current_model,
                            ctx.attempt,
                            FailureReason::Timeout,
                            ctx.start,
                            ctx.request_id,
                            ctx.vk_id,
                        )
                        .await);
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
        .record(ctx.channel.id, estimated_tokens)
        .await;

    match resp_result {
        Ok(r) => Ok((r, active_guard)),
        Err(e) => {
            tracing::error!(channel = %ctx.channel.name, error = %e, "Request failed");
            Err(fail_and_retry(
                state,
                ctx.channel,
                ctx.current_model,
                ctx.attempt,
                FailureReason::ConnectionError,
                ctx.start,
                ctx.request_id,
                ctx.vk_id,
            )
            .await)
        }
    }
}

/// Build the upstream HTTP request, inject headers/auth, and send with TTFT timeout.
///
/// Returns the upstream response on success alongside the RAII guards (pool
/// client + active request tracker) that must outlive the response body, or
/// an `AttemptOutcome` (always `Retry`) on failure.
async fn build_and_send_request(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    upstream_body: &Value,
    upstream_model: &str,
    api_key: &str,
    estimated_tokens: u64,
) -> Result<(reqwest::Response, PooledClient, ActiveRequestGuard), AttemptOutcome> {
    let (req_builder, pool_guard) =
        build_upstream_request(state, ctx, upstream_body, upstream_model, api_key).await?;
    let (resp, active_guard) = send_with_timeout(state, ctx, req_builder, estimated_tokens).await?;
    Ok((resp, pool_guard, active_guard))
}

/// Check for non-success HTTP status codes and return the appropriate AttemptOutcome.
///
/// Returns `Ok(resp)` when the status is successful and the caller should
/// continue with success handling. Returns `Err(outcome)` when the status
/// indicates an error (429, 5xx, 4xx context overflow, or generic 4xx).
async fn check_error_status(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    resp: reqwest::Response,
) -> Result<reqwest::Response, AttemptOutcome> {
    let status = resp.status();

    // Record TTFT — time from dispatch start to first byte from upstream
    crate::metrics::ttft_seconds()
        .with_label_values(&[ctx.channel.provider.as_str(), ctx.current_model])
        .observe(ctx.start.elapsed().as_secs_f64());

    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_secs = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        tracing::warn!(channel = %ctx.channel.name, retry_after_secs, "Rate limited (429)");
        // Record per-model cooldown so other models on this channel remain available
        state
            .channel_mgr
            .mark_model_rate_limited(ctx.channel.id, ctx.current_model, retry_after_secs)
            .await;
        // Also open the channel circuit breaker (existing behavior — may be refined later)
        state
            .channel_mgr
            .mark_circuit_open_with_retry(ctx.channel.id, retry_after_secs)
            .await;
        state
            .router
            .cooldown_tracker
            .record_attempt(ctx.channel.id, false);
        log_attempt_failure(
            &state.logger,
            ctx.current_model,
            ctx.channel,
            ctx.attempt,
            FailureReason::RateLimited,
            ctx.start,
            ctx.request_id,
            ctx.vk_id.map(|id| id.to_string()),
        )
        .await;
        return Err(AttemptOutcome::Retry);
    }

    if status.is_server_error() {
        tracing::warn!(channel = %ctx.channel.name, status = %status, "Server error");
        return Err(fail_and_retry(
            state,
            ctx.channel,
            ctx.current_model,
            ctx.attempt,
            FailureReason::ServerError,
            ctx.start,
            ctx.request_id,
            ctx.vk_id,
        )
        .await);
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
                channel = %ctx.channel.name,
                model = %ctx.current_model,
                "Context window exceeded — trying context fallback"
            );
            log_attempt_failure(
                &state.logger,
                ctx.current_model,
                ctx.channel,
                ctx.attempt,
                FailureReason::ContextOverflow,
                ctx.start,
                ctx.request_id,
                ctx.vk_id.map(|id| id.to_string()),
            )
            .await;
            return Err(AttemptOutcome::ContextOverflow);
        }

        log_attempt_failure(
            &state.logger,
            ctx.current_model,
            ctx.channel,
            ctx.attempt,
            FailureReason::ClientError(status_code.as_u16()),
            ctx.start,
            ctx.request_id,
            ctx.vk_id.map(|id| id.to_string()),
        )
        .await;
        return Err(AttemptOutcome::Respond(json_response(
            status_code,
            body_text,
        )));
    }

    Ok(resp)
}

/// Peek at the first SSE chunk to decide whether to proceed or retry.
///
/// When bootstrap retries are enabled, reads the first chunk from the stream
/// to check for upstream errors before committing to this channel's stream.
/// Returns `Ok((stream, first_chunk))` when the stream looks healthy, or
/// `Err(AttemptOutcome::Retry)` when the first chunk indicates an error.
async fn check_sse_bootstrap(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    resp: reqwest::Response,
    estimated_tokens: u64,
) -> Result<
    (
        futures::stream::BoxStream<'static, Result<Bytes, reqwest::Error>>,
        Option<Bytes>,
    ),
    AttemptOutcome,
> {
    let bootstrap_retries = state.gateway.stream_bootstrap_retries;
    if bootstrap_retries > 0 && ctx.attempt <= bootstrap_retries {
        bootstrap_sse_stream(state, ctx, resp, estimated_tokens).await
    } else {
        Ok((resp.bytes_stream().boxed(), None))
    }
}

/// Bootstrap an SSE stream by waiting for the first data chunk within
/// a TTFT timeout and handling protocol-specific bootstrap errors.
///
/// Reads the first chunk from the stream to check for upstream errors
/// before committing to this channel's stream.
async fn bootstrap_sse_stream(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    resp: reqwest::Response,
    estimated_tokens: u64,
) -> Result<
    (
        futures::stream::BoxStream<'static, Result<Bytes, reqwest::Error>>,
        Option<Bytes>,
    ),
    AttemptOutcome,
> {
    let mut stream = resp.bytes_stream();
    let ttft_secs = state.gateway.stream_ttft_timeout_secs.unwrap_or(30).max(1);
    let first_result =
        tokio::time::timeout(std::time::Duration::from_secs(ttft_secs), stream.next()).await;

    match first_result {
        Ok(Some(Ok(bytes))) => {
            let preview = String::from_utf8_lossy(&bytes);
            if is_stream_error_chunk(&preview) {
                tracing::warn!(
                    channel = %ctx.channel.name,
                    "Bootstrap retry: first SSE chunk indicates upstream error"
                );
                state.channel_mgr.mark_circuit_open(ctx.channel.id).await;
                state
                    .router
                    .cooldown_tracker
                    .record_attempt(ctx.channel.id, false);
                log_attempt_failure(
                    &state.logger,
                    ctx.current_model,
                    ctx.channel,
                    ctx.attempt,
                    FailureReason::ServerError,
                    ctx.start,
                    ctx.request_id,
                    ctx.vk_id.map(|id| id.to_string()),
                )
                .await;
                return Err(AttemptOutcome::Retry);
            }
            tracing::debug!(
                channel = %ctx.channel.name,
                bytes = bytes.len(),
                "Bootstrap check passed — first chunk is clean"
            );
            Ok((stream.boxed(), Some(bytes)))
        }
        Ok(Some(Err(_e))) => {
            tracing::warn!(
                channel = %ctx.channel.name,
                error = %_e,
                "Bootstrap retry: stream error on first chunk"
            );
            state
                .router
                .cooldown_tracker
                .record_attempt(ctx.channel.id, false);
            log_attempt_failure(
                &state.logger,
                ctx.current_model,
                ctx.channel,
                ctx.attempt,
                FailureReason::ConnectionError,
                ctx.start,
                ctx.request_id,
                ctx.vk_id.map(|id| id.to_string()),
            )
            .await;
            Err(AttemptOutcome::Retry)
        }
        Ok(None) => {
            tracing::warn!(
                channel = %ctx.channel.name,
                "Bootstrap retry: upstream stream ended before first chunk"
            );
            state
                .router
                .cooldown_tracker
                .record_attempt(ctx.channel.id, false);
            log_attempt_failure(
                &state.logger,
                ctx.current_model,
                ctx.channel,
                ctx.attempt,
                FailureReason::ServerError,
                ctx.start,
                ctx.request_id,
                ctx.vk_id.map(|id| id.to_string()),
            )
            .await;
            Err(AttemptOutcome::Retry)
        }
        Err(_elapsed) => {
            tracing::warn!(
                channel = %ctx.channel.name,
                ttft_secs,
                "Bootstrap retry: TTFT timeout waiting for first chunk"
            );
            state
                .limits
                .rate_limiter
                .record(ctx.channel.id, estimated_tokens)
                .await;
            state.channel_mgr.mark_circuit_open(ctx.channel.id).await;
            state
                .router
                .cooldown_tracker
                .record_attempt(ctx.channel.id, false);
            log_attempt_failure(
                &state.logger,
                ctx.current_model,
                ctx.channel,
                ctx.attempt,
                FailureReason::Timeout,
                ctx.start,
                ctx.request_id,
                ctx.vk_id.map(|id| id.to_string()),
            )
            .await;
            Err(AttemptOutcome::Retry)
        }
    }
}

/// Apply model mapping and payload rules to produce the upstream request body.
fn prepare_upstream_body<'a>(
    body: &'a Value,
    channel: &Channel,
    current_model: &str,
    payload_rules: &ChannelPayloadRules,
) -> (String, Cow<'a, Value>) {
    let upstream_model = channel.map_model(current_model);
    let has_payload_rules = payload_rules.has_rules(channel.id);
    let body_model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or(current_model);
    let model_needs_change = upstream_model != body_model;

    let needs_mutation = model_needs_change || has_payload_rules;

    let upstream_body: Cow<'a, Value> = if needs_mutation {
        let mut cloned = body.clone();
        if let Some(obj) = cloned.as_object_mut() {
            if model_needs_change {
                obj.insert("model".to_string(), Value::String(upstream_model.clone()));
            }
        }
        if has_payload_rules {
            cloned = payload_rules.apply_for_model(
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

    (upstream_model, upstream_body)
}

/// Returns Ok(()) if the request passes pre-flight checks, or ContextOverflow if the
/// body exceeds the upstream model's context window.
async fn check_pre_flight_guards(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    upstream_body: &mut Value,
) -> Result<(), AttemptOutcome> {
    let preflight_max_context = {
        let registry = state.model_registry.read();
        let caps = registry.get(ctx.current_model);

        if !caps.supports_thinking {
            if let Some(obj) = upstream_body.as_object_mut() {
                obj.remove("thinking");
                obj.remove("reasoning_effort");
                obj.remove("reasoning");
            }
            if let Some(gen_config) = upstream_body
                .get_mut("generationConfig")
                .and_then(|gc| gc.as_object_mut())
            {
                gen_config.remove("thinkingConfig");
            }
        }

        if !caps.supports_tools {
            if let Some(obj) = upstream_body.as_object_mut() {
                obj.remove("tools");
                obj.remove("tool_choice");
            }
        }

        caps.max_context_tokens
    };

    if let Some(max_context) = preflight_max_context {
        let body_len = serde_json::to_string(&upstream_body)
            .unwrap_or_default()
            .len();
        let preflight_tokens = (body_len / 4) as u64;
        if preflight_tokens > max_context {
            tracing::warn!(
                channel = %ctx.channel.name,
                model = %ctx.current_model,
                estimated_tokens = preflight_tokens,
                max_context,
                "Pre-flight context check failed — skipping upstream call"
            );
            log_attempt_failure(
                &state.logger,
                ctx.current_model,
                ctx.channel,
                ctx.attempt,
                FailureReason::ContextOverflow,
                ctx.start,
                ctx.request_id,
                ctx.vk_id.map(|id| id.to_string()),
            )
            .await;
            return Err(AttemptOutcome::ContextOverflow);
        }
    }

    Ok(())
}

/// Record success metrics, latency, and dispatch log after a successful upstream response.
async fn record_success_side_effects(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
    resp: &reqwest::Response,
) -> Vec<(String, String)> {
    state
        .router
        .cooldown_tracker
        .record_attempt(ctx.channel.id, true);
    if let Some(ref sid) = ctx.session_id {
        state
            .router
            .session_affinity
            .set_channel(sid, ctx.channel.id)
            .await;
    }

    let upstream_headers = extract_passthrough_headers(resp, &state.gateway.passthrough_headers);

    // Passive rate-limit extraction — update quota store from response headers
    if let Some(qh) = crate::quota::collectors::response_header::QuotaHeaders::extract(
        ctx.channel.provider.as_str(),
        resp.headers(),
    ) {
        state
            .billing
            .quota_store
            .update_rate_limits(
                ctx.channel.id,
                qh.remaining_requests,
                qh.limit_requests,
                qh.remaining_tokens,
                qh.limit_tokens,
            )
            .await;
    }

    upstream_headers
}

/// Attempt to dispatch a request to a single channel.
/// Returns `Respond(response)` if a final response is ready, or `Retry` to try next.
pub(super) async fn try_channel_attempt(
    state: &Arc<crate::proxy::AppState>,
    ctx: &DispatchContext<'_>,
) -> AttemptOutcome {
    // ── 1. Body preparation (model mapping, payload rules) ──
    let (upstream_model, mut upstream_body) = prepare_upstream_body(
        ctx.body,
        ctx.channel,
        ctx.current_model,
        &state.limits.payload_rules,
    );
    let upstream_format = ctx.provider.provider_request_format();

    // ── 2. Rate limit check ──
    let estimated_tokens = estimate_tokens(&upstream_body, ctx.is_stream);
    let (allowed, rate_reason) = state
        .limits
        .rate_limiter
        .check(ctx.channel.id, estimated_tokens)
        .await;
    if !allowed {
        tracing::warn!(
            channel = %ctx.channel.name,
            reason = rate_reason,
            tokens = estimated_tokens,
            "Rate limited, skipping channel"
        );
        log_attempt_failure(
            &state.logger,
            ctx.current_model,
            ctx.channel,
            ctx.attempt,
            FailureReason::RateLimited,
            ctx.start,
            ctx.request_id,
            ctx.vk_id.map(|id| id.to_string()),
        )
        .await;
        return AttemptOutcome::Retry;
    }

    // Inject stream_options.include_usage for providers that support it (OpenAI-compatible)
    // to ensure upstream returns token usage in the final SSE chunk
    if ctx.is_stream && ctx.provider.inject_stream_usage() {
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

    // ── 3. Credential lookup ──
    let api_key = match state.channel_mgr.get_credential(ctx.channel.id).await {
        Some(key) => key,
        None => {
            tracing::error!(channel = %ctx.channel.name, "No credential found");
            return fail_and_retry(
                state,
                ctx.channel,
                ctx.current_model,
                ctx.attempt,
                FailureReason::NoCredential,
                ctx.start,
                ctx.request_id,
                ctx.vk_id,
            )
            .await;
        }
    };

    // ── 4. Format translation + provider transform + thinking normalization ──
    if ctx.request_format != upstream_format {
        tracing::info!(
            from = ?ctx.request_format,
            to = ?upstream_format,
            channel = %ctx.channel.name,
            "Translating request body"
        );
        upstream_body = Cow::Owned(translate_request(
            &upstream_body,
            ctx.request_format,
            upstream_format,
        ));
    }

    let mut upstream_body = ctx.provider.transform_request(&upstream_body);

    if upstream_body.is_object() {
        crate::proxy::thinking::normalize_for_provider(
            &mut upstream_body,
            ctx.channel.provider.as_str(),
        );
    }

    // ── 5. Model registry guards + pre-flight context check ──
    if let Err(outcome) = check_pre_flight_guards(state, ctx, &mut upstream_body).await {
        return outcome;
    }

    // ── 6. Build and send request ──
    let (resp, pool_guard, active_guard) = match build_and_send_request(
        state,
        ctx,
        &upstream_body,
        &upstream_model,
        &api_key,
        estimated_tokens,
    )
    .await
    {
        Ok(v) => v,
        Err(outcome) => return outcome,
    };

    // ── 7. Check error status ──
    let resp = match check_error_status(state, ctx, resp).await {
        Ok(r) => r,
        Err(outcome) => return outcome,
    };

    // ── 8. Success — record cooldown, affinity, quota headers, passthrough headers ──
    let upstream_headers = record_success_side_effects(state, ctx, &resp).await;

    let trigger_reason: Option<&str> = if ctx.current_model != ctx.original_model {
        Some("model_fallback")
    } else {
        None
    };

    // ── 9. Handle streaming or JSON success ──
    if ctx.is_stream {
        let needs_proto_translate =
            ctx.request_format != upstream_format && !ctx.provider.is_gemini_stream();
        let protocol_translation = if needs_proto_translate {
            Some((ctx.request_format, upstream_format))
        } else {
            None
        };

        let (upstream_stream, first_chunk) =
            match check_sse_bootstrap(state, ctx, resp, estimated_tokens).await {
                Ok(v) => v,
                Err(outcome) => return outcome,
            };

        let response = {
            let resp_ctx = super::response::ResponseContext {
                channel: ctx.channel,
                body: ctx.body,
                current_model: ctx.current_model,
                upstream_model: &upstream_model,
                original_model: ctx.original_model,
                attempt: ctx.attempt,
                trigger_reason,
                start: ctx.start,
                request_id: ctx.request_id,
                upstream_headers: &upstream_headers,
                vk_id: ctx.vk_id,
                reserved_cents: ctx.reserved_cents,
                cache_key: ctx.cache_key,
                cache_key_material: ctx.cache_key_material,
            };
            handle_streaming_success(
                state,
                resp_ctx,
                ctx.provider,
                upstream_stream,
                first_chunk,
                super::response::ResponseGuards {
                    pool: pool_guard,
                    active: active_guard,
                },
                protocol_translation,
            )
            .await
        };
        AttemptOutcome::Respond(response)
    } else {
        let response = {
            let resp_ctx = super::response::ResponseContext {
                channel: ctx.channel,
                body: ctx.body,
                current_model: ctx.current_model,
                upstream_model: &upstream_model,
                original_model: ctx.original_model,
                attempt: ctx.attempt,
                trigger_reason,
                start: ctx.start,
                request_id: ctx.request_id,
                upstream_headers: &upstream_headers,
                vk_id: ctx.vk_id,
                reserved_cents: ctx.reserved_cents,
                cache_key: ctx.cache_key,
                cache_key_material: ctx.cache_key_material,
            };
            handle_json_success(
                state,
                resp_ctx,
                ctx.provider,
                resp,
                super::response::ResponseGuards {
                    pool: pool_guard,
                    active: active_guard,
                },
                ctx.request_format,
                upstream_format,
            )
            .await
        };
        AttemptOutcome::Respond(response)
    }
}

/// Heuristic check for SSE error events in the first chunk of a streaming
/// response.
///
/// Looks for common error patterns across providers:
/// - Anthropic: `event: error\ndata: {"type":"error",...}`
/// - OpenAI / OpenAI-compatible: `data: {"error":{...}}`
/// - Generic: `"type":"error"` in the JSON payload
///
/// Normal first chunks (content deltas) always contain `"choices"` (OpenAI)
/// or `"content_block"` / `"message_start"` (Anthropic), and never contain
/// `"error"` at the top level.
fn is_stream_error_chunk(chunk: &str) -> bool {
    // Anthropic error events are prefixed with `event: error`
    if chunk.contains("event: error") {
        return true;
    }
    // Generic `"type":"error"` or `"type": "error"` in the JSON payload
    if chunk.contains("\"type\":\"error\"") || chunk.contains("\"type\": \"error\"") {
        return true;
    }
    // OpenAI / OpenAI-compatible error: `{"error":{...}}`
    // Normal chunks always have `"choices"`, so the absence of `"choices"`
    // combined with the presence of `"error"` is a strong signal.
    if chunk.contains("\"error\"") && !chunk.contains("\"choices\"") {
        return true;
    }
    false
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
    use super::{is_context_window_error, is_stream_error_chunk};
    use reqwest::StatusCode;

    // ── is_stream_error_chunk tests ──────────────────────────────────────

    #[test]
    fn detects_openai_stream_error() {
        let chunk = r#"data: {"error":{"message":"server error","type":"server_error"}}"#;
        assert!(is_stream_error_chunk(chunk));
    }

    #[test]
    fn detects_anthropic_event_error() {
        let chunk = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}\n\n";
        assert!(is_stream_error_chunk(chunk));
    }

    #[test]
    fn detects_generic_type_error() {
        let chunk = r#"data: {"type":"error","error":{"message":"..."}}"#;
        assert!(is_stream_error_chunk(chunk));
    }

    #[test]
    fn does_not_flag_normal_openai_chunk() {
        let chunk = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"hello"}}]}"#;
        assert!(!is_stream_error_chunk(chunk));
    }

    #[test]
    fn does_not_flag_normal_anthropic_chunk() {
        let chunk = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n";
        assert!(!is_stream_error_chunk(chunk));
    }

    #[test]
    fn does_not_flag_anthropic_message_start() {
        let chunk = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n";
        assert!(!is_stream_error_chunk(chunk));
    }

    #[test]
    fn does_not_flag_empty_chunk() {
        assert!(!is_stream_error_chunk(""));
    }

    #[test]
    fn does_not_flag_done_marker() {
        assert!(!is_stream_error_chunk("data: [DONE]\n\n"));
    }

    // ── is_context_window_error tests ───────────────────────────────────

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
