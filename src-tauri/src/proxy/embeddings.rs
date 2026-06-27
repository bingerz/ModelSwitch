//! Handler for the OpenAI-compatible `/v1/embeddings` endpoint.
//!
//! Embeddings requests are simpler than chat completions: no streaming, no
//! MCP tools, no fallback chains. The handler reuses the same routing,
//! auth, rate-limiting, and budget enforcement as chat completions, then
//! forwards the request to the upstream `/v1/embeddings` endpoint and
//! returns the response.

use crate::channel::Channel;
use crate::proxy::error_response;
use crate::proxy::stream::json_response;
use crate::proxy::AppState;
use crate::proxy::SKIP_HEADERS;
use crate::router::{self, RoutingContext};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Upstream path for OpenAI-compatible embeddings.
const EMBEDDINGS_UPSTREAM_PATH: &str = "v1/embeddings";

/// Estimated token count used for embeddings rate limiting (conservative).
const EMBEDDINGS_ESTIMATED_TOKENS: u64 = 1000;

// ── Helper functions ──────────────────────────────────────────────────────

/// Validate that the request body contains non-empty `model` and `input` fields.
///
/// Returns the requested model string on success, or an OpenAI-compatible error
/// response on failure.
#[allow(clippy::result_large_err)]
fn validate_embedding_request(body: &Value) -> Result<String, axum::response::Response> {
    let requested_model = match body.get("model").and_then(|m| m.as_str()) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Missing required field: model",
                "invalid_request_error",
                "missing_model",
            ));
        }
    };

    if body.get("input").is_none() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Missing required field: input",
            "invalid_request_error",
            "missing_input",
        ));
    }

    Ok(requested_model)
}

/// Select a channel for the embedding request via `router::select_channel`.
///
/// Returns the selected channel on success, or a 503 error response when no
/// channel is available for the requested model.
#[allow(clippy::result_large_err)]
async fn select_embedding_channel(
    state: &AppState,
    model: &str,
    account_group: Option<&str>,
) -> Result<Channel, axum::response::Response> {
    let channels = state.channel_mgr.channels();
    let ctx = RoutingContext {
        active_requests: &state.router.active_requests,
        rate_limiter: &state.limits.rate_limiter,
        latency_tracker: &state.router.latency_tracker,
        cooldown_tracker: &state.router.cooldown_tracker,
    };

    match router::select_channel(channels, model, state.gateway.routing_strategy, &ctx, account_group).await {
        Some(ch) => Ok(ch),
        None => Err(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({
                "error": {
                    "message": format!("No available channel for model: {}", model),
                    "type": "server_error",
                    "code": "no_channel_available"
                }
            })
            .to_string(),
        )),
    }
}

/// Check provider budget and rate limiter before dispatching the request.
///
/// Returns `Ok(())` if all checks pass, or an error response (402 for budget
/// exceeded, 429 for rate limited) on failure.
#[allow(clippy::result_large_err)]
async fn check_embedding_pre_dispatch(
    state: &AppState,
    channel: &Channel,
    channel_id: Uuid,
    estimated_tokens: u64,
) -> Result<(), axum::response::Response> {
    // ── Provider budget check ──
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
        return Err(json_response(
            StatusCode::PAYMENT_REQUIRED,
            serde_json::json!({
                "error": {
                    "message": format!("Provider '{}' budget exceeded", provider_name),
                    "type": "budget_exceeded",
                    "code": "provider_budget_exceeded"
                }
            })
            .to_string(),
        ));
    }

    // ── Rate limiter check ──
    let (allowed, rate_reason) = state
        .limits
        .rate_limiter
        .check(channel_id, estimated_tokens);
    if !allowed {
        tracing::warn!(
            channel = %channel.name,
            reason = rate_reason,
            "Embeddings request rate limited"
        );
        return Err(json_response(
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({
                "error": {
                    "message": "Rate limit exceeded for channel. Please retry after a brief wait.",
                    "type": "rate_limit_error",
                    "code": "rate_limit_exceeded"
                }
            })
            .to_string(),
        ));
    }

    Ok(())
}

/// Look up the channel credential, build the upstream URL, apply model
/// mapping, forward client headers, and attach provider-specific auth.
///
/// Returns the `RequestBuilder` (ready to send) plus the `channel_id`,
/// `channel_name`, and `provider_label` needed for subsequent logging and
/// metrics recording.
#[allow(clippy::result_large_err)]
async fn build_embedding_request(
    state: &AppState,
    headers: &HeaderMap,
    channel: &Channel,
    body: Value,
    resolved_model: &str,
    estimated_tokens: u64,
) -> Result<(reqwest::RequestBuilder, Uuid, String, String), axum::response::Response> {
    // ── Credential lookup ──
    let api_key = match state.channel_mgr.get_credential(channel.id).await {
        Some(key) => key,
        None => {
            tracing::error!(channel = %channel.name, "No credential found for embeddings");
            state
                .limits
                .rate_limiter
                .record(channel.id, estimated_tokens);
            state.channel_mgr.mark_circuit_open(channel.id).await;
            return Err(json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": {
                        "message": "No credential available for channel",
                        "type": "server_error",
                        "code": "no_credential"
                    }
                })
                .to_string(),
            ));
        }
    };

    // ── Model mapping ──
    let upstream_model = channel.map_model(resolved_model);
    let model_needs_change = upstream_model != resolved_model;
    let mut upstream_body = body;
    if model_needs_change {
        if let Some(obj) = upstream_body.as_object_mut() {
            obj.insert("model".to_string(), Value::String(upstream_model.clone()));
        }
    }

    // ── Build URL ──
    let url = format!(
        "{}/{}",
        channel.base_url.trim_end_matches('/'),
        EMBEDDINGS_UPSTREAM_PATH
    );

    // ── Build reqwest request ──
    let _pool_guard = state.http_pool.get();
    let mut req_builder = _pool_guard.post(&url).json(&upstream_body);

    // Forward client headers (excluding hop-by-hop and auth headers)
    for (name, value) in headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            req_builder = req_builder.header(name.clone(), value.clone());
        }
    }

    // Apply provider-specific auth
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

    Ok((req_builder, channel_id, channel_name, provider_label))
}

/// Process the upstream response: record rate limiter usage, emit metrics,
/// trip the circuit breaker on error statuses (429, 5xx), log the dispatch
/// event, and return the body to the client.
async fn handle_embedding_response(
    status: StatusCode,
    body_text: String,
    state: &AppState,
    channel: &Channel,
    provider_label: &str,
    resolved_model: &str,
    estimated_tokens: u64,
) -> axum::response::Response {
    // ── Record rate limiter usage ──
    state
        .limits
        .rate_limiter
        .record(channel.id, estimated_tokens);

    // ── Record metrics ──
    let status_label = if status.is_success() {
        "success"
    } else {
        "error"
    };
    crate::metrics::requests_total()
        .with_label_values(&[provider_label, resolved_model, status_label])
        .inc();

    // ── Circuit breaker for upstream errors ──
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        tracing::warn!(
            channel = %channel.name,
            status = %status,
            "Embeddings upstream returned error, marking circuit open"
        );
        state.channel_mgr.mark_circuit_open(channel.id).await;
    }

    // ── Log dispatch ──
    let logger = Arc::clone(&state.logger);
    let model_for_log = resolved_model.to_string();
    let status_for_log = status;
    let channel_name_for_log = channel.name.clone();
    let channel_id_for_log = channel.id;
    tokio::spawn(async move {
        let reason = if status_for_log.is_success() {
            None
        } else {
            Some(format!("embeddings_{}", status_for_log.as_u16()))
        };
        logger
            .log(crate::proxy::make_log(
                &model_for_log,
                channel_id_for_log,
                &channel_name_for_log,
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
                None,
            ))
            .await;
    });

    json_response(status, body_text)
}

// ── Main handler ──────────────────────────────────────────────────────────

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
    // ── Validate request ──
    let requested_model = match validate_embedding_request(&body) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // ── Resolve gateway-level model alias ──
    let resolved_model = state
        .gateway
        .model_aliases
        .get(&requested_model)
        .cloned()
        .unwrap_or(requested_model);

    // ── Account-group routing filter ──
    let account_group = headers
        .get("x-account-group")
        .and_then(|v| v.to_str().ok());

    // ── Channel selection ──
    let channel = match select_embedding_channel(&state, &resolved_model, account_group).await {
        Ok(ch) => ch,
        Err(e) => return e,
    };

    // ── Track active request via RAII guard ──
    let _active_guard = state.router.active_requests.acquire(channel.id);

    // ── Pre-dispatch checks (budget + rate limiter) ──
    if let Err(e) = check_embedding_pre_dispatch(
        &state,
        &channel,
        channel.id,
        EMBEDDINGS_ESTIMATED_TOKENS,
    )
    .await
    {
        return e;
    }

    // ── Build upstream request ──
    let (req_builder, channel_id, channel_name, provider_label) = match build_embedding_request(
        &state,
        &headers,
        &channel,
        body,
        &resolved_model,
        EMBEDDINGS_ESTIMATED_TOKENS,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return e,
    };

    // ── Send upstream request ──
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
                .record(channel_id, EMBEDDINGS_ESTIMATED_TOKENS);
            state.channel_mgr.mark_circuit_open(channel_id).await;

            crate::metrics::requests_total()
                .with_label_values(&[&provider_label, &resolved_model, "error"])
                .inc();

            return json_response(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "error": {
                        "message": "Upstream connection error".to_string(),
                        "type": "server_error",
                        "code": "upstream_connection_error"
                    }
                })
                .to_string(),
            );
        }
    };

    // ── Process response ──
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    handle_embedding_response(
        status,
        body_text,
        &state,
        &channel,
        &provider_label,
        &resolved_model,
        EMBEDDINGS_ESTIMATED_TOKENS,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{build_test_state, channel_config, response_json, response_status};
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    // ── Error path tests (no wiremock needed) ───────────────────────────────

    #[tokio::test]
    async fn handle_embeddings_rejects_missing_model() {
        let state = build_test_state(vec![]);
        let headers = HeaderMap::new();
        let response = handle_embeddings(State(state), headers, Json(json!({}))).await;

        assert_eq!(response_status(&response), 400);
        let v = response_json(response).await;
        assert_eq!(v["error"]["code"], "missing_model");
    }

    #[tokio::test]
    async fn handle_embeddings_rejects_missing_input() {
        let state = build_test_state(vec![]);
        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 400);
        let v = response_json(response).await;
        assert_eq!(v["error"]["code"], "missing_input");
    }

    #[tokio::test]
    async fn handle_embeddings_returns_503_when_no_channel() {
        let state = build_test_state(vec![]);
        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello world"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 503);
        let v = response_json(response).await;
        assert_eq!(v["error"]["code"], "no_channel_available");
    }

    // ── Success path tests (with wiremock) ──────────────────────────────────

    #[tokio::test]
    async fn handle_embeddings_success_forwards_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [{
                    "object": "embedding",
                    "index": 0,
                    "embedding": [0.1, 0.2, 0.3]
                }],
                "model": "text-embedding-3-small",
                "usage": {"prompt_tokens": 5, "total_tokens": 5}
            })))
            .mount(&mock_server)
            .await;

        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "primary",
            &mock_server.uri(),
            1,
        )]);

        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello world"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 200);
        let v = response_json(response).await;
        assert_eq!(v["data"][0]["object"], "embedding");
        assert_eq!(v["model"], "text-embedding-3-small");
    }

    #[tokio::test]
    async fn handle_embeddings_applies_model_mapping() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [{"object": "embedding", "index": 0, "embedding": [0.1]}],
                "model": "upstream-model",
                "usage": {"prompt_tokens": 1, "total_tokens": 1}
            })))
            .mount(&mock_server)
            .await;

        let mut cfg = channel_config(
            "00000000-0000-0000-0000-000000000001",
            "mapped",
            &mock_server.uri(),
            1,
        );
        cfg.model_mapping = [("test-model".to_string(), "upstream-model".to_string())].into();

        let state = build_test_state(vec![cfg]);
        let headers = HeaderMap::new();
        let body = json!({"model": "test-model", "input": "hello"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 200);

        // Verify the upstream received the mapped model name
        let received = mock_server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let req_body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(req_body["model"], "upstream-model");
    }

    #[tokio::test]
    async fn handle_embeddings_skips_denylisted_headers() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [{"object": "embedding", "index": 0, "embedding": [0.1]}],
                "model": "text-embedding-3-small",
                "usage": {"prompt_tokens": 1, "total_tokens": 1}
            })))
            .mount(&mock_server)
            .await;

        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "primary",
            &mock_server.uri(),
            1,
        )]);

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer client-secret".parse().unwrap());
        headers.insert("host", "gateway.example.com".parse().unwrap());
        headers.insert("x-safe-header", "safe-value".parse().unwrap());

        let body = json!({"model": "text-embedding-3-small", "input": "hello"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 200);

        let received = mock_server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let req = &received[0];

        // The client's Authorization must NOT be forwarded — the gateway
        // substitutes the channel credential instead.
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            auth, "Bearer sk-test-key",
            "should use channel credential, not client auth"
        );

        // The client-supplied Host header must not leak through.
        let host = req.headers.get("host").and_then(|v| v.to_str().ok());
        assert_ne!(
            host,
            Some("gateway.example.com"),
            "client Host must not be forwarded"
        );

        // Non-denylisted custom headers are forwarded.
        let safe = req
            .headers
            .get("x-safe-header")
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            safe,
            Some("safe-value"),
            "non-denylisted headers should be forwarded"
        );
    }

    // ── Error handling tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_embeddings_returns_502_on_upstream_connection_error() {
        // Point at a port with no listener — connection refused.
        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "dead-channel",
            "http://127.0.0.1:1",
            1,
        )]);

        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 502);
        let v = response_json(response).await;
        assert_eq!(v["error"]["code"], "upstream_connection_error");
    }

    #[tokio::test]
    async fn handle_embeddings_forwards_upstream_429() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(429).set_body_json(json!({
                "error": {"message": "Rate limited", "type": "rate_limit_error"}
            })))
            .mount(&mock_server)
            .await;

        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "primary",
            &mock_server.uri(),
            1,
        )]);

        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 429);
    }

    #[tokio::test]
    async fn handle_embeddings_applies_bearer_auth() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [{"object": "embedding", "index": 0, "embedding": [0.1]}],
                "model": "text-embedding-3-small",
                "usage": {"prompt_tokens": 1, "total_tokens": 1}
            })))
            .mount(&mock_server)
            .await;

        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "primary",
            &mock_server.uri(),
            1,
        )]);

        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello"});
        let response = handle_embeddings(State(state), headers, Json(body)).await;

        assert_eq!(response_status(&response), 200);

        let received = mock_server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let auth = received[0]
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        assert_eq!(auth, Some("Bearer sk-test-key"));
    }

    #[tokio::test]
    async fn handle_embeddings_records_rate_limiter_usage() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [{"object": "embedding", "index": 0, "embedding": [0.1]}],
                "model": "text-embedding-3-small",
                "usage": {"prompt_tokens": 1, "total_tokens": 1}
            })))
            .mount(&mock_server)
            .await;

        let channel_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let state = build_test_state(vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "primary",
            &mock_server.uri(),
            1,
        )]);

        // Clone the Arc so we can inspect state after the handler call
        let headers = HeaderMap::new();
        let body = json!({"model": "text-embedding-3-small", "input": "hello"});
        let response = handle_embeddings(State(Arc::clone(&state)), headers, Json(body)).await;

        assert_eq!(response_status(&response), 200);

        // The handler records estimated_tokens = 1000 after dispatching
        let tpm = state.limits.rate_limiter.current_tpm(channel_id);
        assert_eq!(tpm, 1000);
    }
}