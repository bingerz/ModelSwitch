//! Handler for the OpenAI-compatible `/v1/embeddings` endpoint.
//!
//! Embeddings requests are simpler than chat completions: no streaming, no
//! MCP tools, no fallback chains. The handler reuses the same routing,
//! auth, rate-limiting, and budget enforcement as chat completions, then
//! forwards the request to the upstream `/v1/embeddings` endpoint and
//! returns the response.

use crate::proxy::AppState;
use crate::proxy::stream::json_response;
use crate::proxy::SKIP_HEADERS;
use crate::router::{self, RoutingContext};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;

/// Upstream path for OpenAI-compatible embeddings.
const EMBEDDINGS_UPSTREAM_PATH: &str = "v1/embeddings";

/// Handle `POST /v1/embeddings` requests.
///
/// Selects a healthy channel for the requested model, forwards the request
/// to the upstream embeddings endpoint, and returns the response.
/// Supports gateway-level model aliases, tag-based routing via the
/// `x-account-group` header, per-channel rate limiting, circuit breaker,
/// and per-provider budget enforcement.
pub async fn handle_embeddings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    // ── Validate request ────────────────────────────────────────────────
    let requested_model = match body.get("model").and_then(|m| m.as_str()) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "error": {
                        "message": "Missing required field: model",
                        "type": "invalid_request_error",
                        "code": "missing_model"
                    }
                })
                .to_string(),
            );
        }
    };

    if body.get("input").is_none() {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": {
                    "message": "Missing required field: input",
                    "type": "invalid_request_error",
                    "code": "missing_input"
                }
            })
            .to_string(),
        );
    }

    // ── Resolve gateway-level model alias ──────────────────────────────
    // Per-channel model_mapping is applied after channel selection.
    let resolved_model = state
        .gateway
        .model_aliases
        .get(&requested_model)
        .cloned()
        .unwrap_or(requested_model.clone());

    // ── Account-group routing filter ───────────────────────────────────
    let account_group = headers
        .get("x-account-group")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Narrow the channel pool when an account group tag is present, matching
    // the semantics in dispatch.rs: channels with a matching tag or no tag.
    let channels = if let Some(ref tag) = account_group {
        let all = state.channel_mgr.channels();
        let guard = all.read().await;
        let filtered: Vec<crate::channel::Channel> = guard
            .iter()
            .filter(|c| c.account_group.as_deref() == Some(tag.as_str()) || c.account_group.is_none())
            .map(|c| {
                let mut c = c.clone();
                c.recover_if_expired();
                c
            })
            .collect();
        drop(guard);
        std::sync::Arc::new(tokio::sync::RwLock::new(filtered))
    } else {
        state.channel_mgr.channels()
    };

    // ── Channel selection ──────────────────────────────────────────────
    let ctx = RoutingContext {
        active_requests: &state.router.active_requests,
        rate_limiter: &state.limits.rate_limiter,
        latency_tracker: &state.router.latency_tracker,
    };

    let channel = match router::select_channel(
        channels,
        &resolved_model,
        state.gateway.routing_strategy,
        &ctx,
        None,
    )
    .await
    {
        Some(ch) => ch,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": {
                        "message": format!("No available channel for model: {}", resolved_model),
                        "type": "server_error",
                        "code": "no_channel_available"
                    }
                })
                .to_string(),
            );
        }
    };

    // ── Track active request via RAII guard ───────────────────────────
    let _active_guard = state.router.active_requests.acquire(channel.id);

    // ── Provider budget check ──────────────────────────────────────────
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
            "Provider budget exceeded, rejecting embeddings request"
        );
        return json_response(
            StatusCode::PAYMENT_REQUIRED,
            serde_json::json!({
                "error": {
                    "message": format!("Provider '{}' budget exceeded", provider_name),
                    "type": "budget_exceeded",
                    "code": "provider_budget_exceeded"
                }
            })
            .to_string(),
        );
    }

    // ── Rate limiter ───────────────────────────────────────────────────
    // Embeddings requests typically have small token counts; use a conservative
    // estimate of 1000 tokens for the rate limiter check.
    let estimated_tokens: u64 = 1000;
    let (allowed, rate_reason) = state
        .limits
        .rate_limiter
        .check(channel.id, estimated_tokens);
    if !allowed {
        tracing::warn!(
            channel = %channel.name,
            reason = rate_reason,
            "Embeddings request rate limited"
        );
        return json_response(
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({
                "error": {
                    "message": "Rate limit exceeded for channel. Please retry after a brief wait.",
                    "type": "rate_limit_error",
                    "code": "rate_limit_exceeded"
                }
            })
            .to_string(),
        );
    }

    // ── Credential lookup ──────────────────────────────────────────────
    let api_key = match state.channel_mgr.get_credential(channel.id).await {
        Some(key) => key,
        None => {
            tracing::error!(channel = %channel.name, "No credential found for embeddings");
            state
                .limits
                .rate_limiter
                .record(channel.id, estimated_tokens);
            state.channel_mgr.mark_circuit_open(channel.id).await;
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": {
                        "message": "No credential available for channel",
                        "type": "server_error",
                        "code": "no_credential"
                    }
                })
                .to_string(),
            );
        }
    };

    // ── Build upstream request ─────────────────────────────────────────
    let upstream_model = channel.map_model(&resolved_model);
    let model_needs_change = upstream_model != resolved_model;
    let mut upstream_body = body;
    if model_needs_change {
        if let Some(obj) = upstream_body.as_object_mut() {
            obj.insert(
                "model".to_string(),
                Value::String(upstream_model.clone()),
            );
        }
    }

    let url = format!(
        "{}/{}",
        channel.base_url.trim_end_matches('/'),
        EMBEDDINGS_UPSTREAM_PATH
    );

    let pool_guard = state.http_pool.get();
    let mut req_builder = pool_guard.post(&url).json(&upstream_body);

    // Forward client headers (excluding hop-by-hop and auth headers that the
    // gateway manages).
    for (name, value) in headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            req_builder = req_builder.header(name.clone(), value.clone());
        }
    }

    // Apply provider-specific auth. Embeddings are OpenAI-compatible, so we
    // use Bearer auth for all providers except web-session channels.
    let is_web_session =
        channel.credential.cred_type == crate::channel::CredentialType::WebSession;
    req_builder = if is_web_session {
        req_builder
            .header("Cookie", &api_key)
            .header("Content-Type", "application/json")
    } else {
        req_builder
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
    };

    let provider_label = channel.provider.as_str().to_string();
    let channel_id = channel.id;
    let channel_name = channel.name.clone();

    // ── Send upstream request ──────────────────────────────────────────
    let resp = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                channel = %channel_name,
                error = %e,
                "Embeddings upstream request failed"
            );
            state
                .limits
                .rate_limiter
                .record(channel.id, estimated_tokens);
            state.channel_mgr.mark_circuit_open(channel_id).await;

            crate::metrics::requests_total()
                .with_label_values(&[&provider_label, &resolved_model, "error"])
                .inc();

            return json_response(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "error": {
                        "message": format!("Upstream connection error: {}", e),
                        "type": "server_error",
                        "code": "upstream_connection_error"
                    }
                })
                .to_string(),
            );
        }
    };

    // ── Process response ───────────────────────────────────────────────
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    // Record rate limiter usage (the request was sent)
    state
        .limits
        .rate_limiter
        .record(channel.id, estimated_tokens);

    // Active-request decrement handled by `_active_guard` drop at function end.

    // Record metrics
    let status_label = if status.is_success() { "success" } else { "error" };
    crate::metrics::requests_total()
        .with_label_values(&[&provider_label, &resolved_model, status_label])
        .inc();

    // Handle upstream rate-limit and server errors by tripping the circuit
    // breaker so subsequent requests pick a different channel.
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        tracing::warn!(
            channel = %channel_name,
            status = %status,
            "Embeddings upstream returned error, marking circuit open"
        );
        state.channel_mgr.mark_circuit_open(channel_id).await;
    }

    // Log dispatch
    let logger = Arc::clone(&state.logger);
    let model_for_log = resolved_model.clone();
    let status_for_log = status;
    tokio::spawn(async move {
        let reason = if status_for_log.is_success() {
            None
        } else {
            Some(format!("embeddings_{}", status_for_log.as_u16()))
        };
        logger
            .log(crate::proxy::make_log(
                &model_for_log,
                channel_id,
                &channel_name,
                1,
                1,
                reason.as_deref(),
                0,
                status_for_log.is_success(),
                None,
                None,
                None,
                None,
                None,
                None,
            ))
            .await;
    });

    json_response(status, body_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_path_constant_is_correct() {
        assert_eq!(EMBEDDINGS_UPSTREAM_PATH, "v1/embeddings");
    }

    #[test]
    fn skip_headers_excludes_auth_and_hop_by_hop() {
        // Verify that SKIP_HEADERS (re-exported from the proxy module) contains
        // the critical headers we must not forward from the client.
        assert!(SKIP_HEADERS.contains(&"authorization"));
        assert!(SKIP_HEADERS.contains(&"host"));
        assert!(SKIP_HEADERS.contains(&"content-type"));
        assert!(SKIP_HEADERS.contains(&"content-length"));
    }
}
