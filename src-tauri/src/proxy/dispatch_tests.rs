//! Integration tests for the proxy dispatch flow using wiremock.
//!
//! These tests exercise the full dispatch pipeline — channel selection,
//! upstream forwarding, retry on 429/5xx, circuit breaking, and caching —
//! against a real HTTP mock server. They live inside the `proxy` module
//! because `dispatch()` is `pub(crate)` and therefore inaccessible from
//! `tests/` integration test binaries.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderMap;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::channel::manager::ChannelManager;
use crate::config::{AppConfig, ChannelConfig, GatewayConfig, SanitizerConfig};
use crate::credential::{create_credential_store, SharedCredentialStore};
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::openai::{
    AppState, BillingState, CacheState, GatewayParams, LimitsState, McpState, RouterState,
    SecurityState,
};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::provider::OpenAIAdaptor;
use crate::proxy::dispatch;
use crate::quota::QuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::virtual_key::VirtualKeyStore;

// ── Test helpers ───────────────────────────────────────────────────────────

/// Build a standard OpenAI chat completion provider adaptor.
fn openai_provider() -> OpenAIAdaptor {
    OpenAIAdaptor
}

/// Build a valid OpenAI chat completion JSON body for a non-streaming request.
fn chat_request_body(model: &str, content: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
        "max_tokens": 50
    })
}

/// Build a mock upstream chat completion response body.
fn chat_completion_response(content: &str) -> String {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1234567890_u64,
        "model": "gpt-4",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
    .to_string()
}

/// Build a single `ChannelConfig` pointing at the given base URL.
fn channel_config(id: &str, name: &str, base_url: &str, priority: u8) -> ChannelConfig {
    ChannelConfig {
        id: id.to_string(),
        name: name.to_string(),
        provider: "openai".to_string(),
        priority,
        weight: 100,
        cost_per_token: None,
        input_cost_per_mtok: None,
        output_cost_per_mtok: None,
        credential_type: "api_key".to_string(),
        credential_ref: "unused".to_string(),
        api_key: Some("sk-test-key".to_string()),
        base_url: base_url.to_string(),
        enabled: true,
        model_mapping: HashMap::new(),
        cooldown_minutes: None,
        rpm_limit: None,
        tpm_limit: None,
        payload_rules: None,
        quota: None,
        account_group: None,
        max_concurrent: None,
        api_keys: vec![],
    }
}

/// Build a minimal `AppConfig` with channels pointing at the given base URLs.
fn test_config(channels: Vec<ChannelConfig>) -> AppConfig {
    AppConfig {
        gateway: GatewayConfig {
            max_retries: 3,
            health_check_enabled: false,
            ..GatewayConfig::default()
        },
        channels,
        mcp_servers: vec![],
    }
}

/// Construct a minimal `AppState` whose channels point to the provided
/// mock server URLs. All sub-structs use real implementations with
/// permissive defaults so dispatch behaves naturally.
fn build_test_state(channel_configs: Vec<ChannelConfig>) -> Arc<AppState> {
    let config = test_config(channel_configs);
    let credential_store: SharedCredentialStore = create_credential_store();
    let channel_mgr = Arc::new(ChannelManager::new(&config, Arc::clone(&credential_store)));

    let logger = Arc::new(DispatchLogger::new(1000));

    let http_pool = crate::http_pool::HttpPool::new(1, || {
        reqwest::Client::builder().timeout(Duration::from_secs(30))
    })
    .expect("Failed to build HTTP client pool");

    let active_requests = Arc::new(ActiveRequests::new());
    let request_cache = Arc::new(RequestCache::new(
        Duration::from_secs(300),
        1000,
        CacheMode::On,
    ));
    let in_flight = Arc::new(InFlightRequests::new());
    let payload_rules = Arc::new(ChannelPayloadRules::new());
    let rate_limiter = Arc::new(RateLimiter::new(None));
    let quota_store = Arc::new(QuotaStore::new());
    let virtual_key_store = Arc::new(VirtualKeyStore::new());
    let mcp_manager = Arc::new(McpManager::new());

    Arc::new(AppState {
        channel_mgr,
        credential_store,
        logger,
        http_pool,
        gateway: GatewayParams {
            request_timeout_secs: Some(30),
            stream_keepalive_secs: None,
            stream_ttft_timeout_secs: Some(30),
            max_retries: config.gateway.max_retries,
            model_fallbacks: HashMap::new(),
            routing_strategy: "weighted_random".to_string(),
            retry_base_ms: config.gateway.retry_base_ms,
            retry_max_ms: config.gateway.retry_max_ms,
        },
        router: RouterState {
            session_affinity: SessionAffinity::default(),
            active_requests,
            latency_tracker: Arc::new(crate::router::latency_tracker::LatencyTracker::new()),
        },
        cache: CacheState {
            request_cache,
            in_flight,
        },
        limits: LimitsState {
            payload_rules,
            rate_limiter,
        },
        billing: BillingState {
            quota_store,
            virtual_key_store,
            provider_budgets: Arc::new(
                crate::provider_budget::ProviderBudgetStore::new(),
            ),
        },
        mcp: McpState {
            mcp_manager,
            mcp_max_iterations: 5,
            mcp_auto_inject: false,
            mcp_gateway_enabled: false,
        },
        security: SecurityState {
            admin_token: None,
            sanitizer_config: SanitizerConfig::default(),
        },
        started_at: std::time::Instant::now(),
    })
}

/// Extract the HTTP status code from an axum Response.
fn response_status(response: &axum::response::Response) -> u16 {
    response.status().as_u16()
}

/// Buffer the axum Response body and parse as JSON.
async fn response_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("Failed to buffer response body");
    serde_json::from_slice(&bytes).expect("Response body is not valid JSON")
}

// ── Test scenarios ─────────────────────────────────────────────────────────

/// Simple dispatch success — mock server returns 200 with a valid chat
/// completion. Verify dispatch returns a 200 response with the expected
/// content.
#[tokio::test]
async fn dispatch_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Hello!")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "primary",
        &mock_server.uri(),
        1,
    )]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Say hello");
    let provider = openai_provider();

    let response = dispatch(&state, &headers, &body, &provider).await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
    assert_eq!(json["model"], "gpt-4");
}

/// Dispatch retries on 429 — the first channel returns 429 (rate limited),
/// the second channel returns 200. Verify the request eventually succeeds.
#[tokio::test]
async fn dispatch_retry_on_429() {
    let mock_fail = MockServer::start().await;
    let mock_success = MockServer::start().await;

    // First channel: always returns 429
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"message": "Rate limited", "type": "rate_limit_error"}
        })))
        .mount(&mock_fail)
        .await;

    // Second channel: returns 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Retried successfully!")),
        )
        .mount(&mock_success)
        .await;

    let state = build_test_state(vec![
        channel_config(
            "00000000-0000-0000-0000-000000000001",
            "fail-channel",
            &mock_fail.uri(),
            1,
        ),
        channel_config(
            "00000000-0000-0000-0000-000000000002",
            "success-channel",
            &mock_success.uri(),
            2,
        ),
    ]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test retry");
    let provider = openai_provider();

    let response = dispatch(&state, &headers, &body, &provider).await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed after retrying on the second channel"
    );
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Retried successfully!"
    );

    // The failing server should have been hit at least once
    let fail_requests = mock_fail.received_requests().await.unwrap();
    assert!(
        !fail_requests.is_empty(),
        "The failing channel should have received at least one request"
    );

    // The success server should have been hit exactly once
    let success_requests = mock_success.received_requests().await.unwrap();
    assert_eq!(
        success_requests.len(),
        1,
        "The success channel should have received exactly one request"
    );
}

/// All channels exhausted — both mock servers return 500. Verify dispatch
/// returns a 429 all-exhausted response.
#[tokio::test]
async fn dispatch_all_channels_exhausted() {
    let mock_1 = MockServer::start().await;
    let mock_2 = MockServer::start().await;

    for server in [&mock_1, &mock_2] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "error": {"message": "Internal server error", "type": "server_error"}
            })))
            .mount(server)
            .await;
    }

    let state = build_test_state(vec![
        channel_config(
            "00000000-0000-0000-0000-000000000001",
            "fail-1",
            &mock_1.uri(),
            1,
        ),
        channel_config(
            "00000000-0000-0000-0000-000000000002",
            "fail-2",
            &mock_2.uri(),
            1,
        ),
    ]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "This should fail");
    let provider = openai_provider();

    let response = dispatch(&state, &headers, &body, &provider).await;

    // The all-exhausted response is a 429
    assert_eq!(
        response_status(&response),
        429,
        "dispatch should return 429 when all channels are exhausted"
    );
    let json = response_json(response).await;
    assert_eq!(json["error"]["code"], "all_channels_rate_limited");
}

/// Cache hit — send the same non-streaming request twice. The mock server
/// should only be hit once; the second request is served from cache.
#[tokio::test]
async fn dispatch_cache_hit() {
    let mock_server = MockServer::start().await;

    // Mount a mock that returns 200. We will verify call count afterwards.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Cached!")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "cached-channel",
        &mock_server.uri(),
        1,
    )]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Cache test");
    let provider = openai_provider();

    // First request — should hit the upstream
    let response1 = dispatch(&state, &headers, &body, &provider).await;
    assert_eq!(response_status(&response1), 200);
    let json1 = response_json(response1).await;
    assert_eq!(json1["choices"][0]["message"]["content"], "Cached!");

    // Second identical request — should be served from cache
    let response2 = dispatch(&state, &headers, &body, &provider).await;
    assert_eq!(response_status(&response2), 200);
    let json2 = response_json(response2).await;
    assert_eq!(json2["choices"][0]["message"]["content"], "Cached!");

    // The mock server should have received exactly one request
    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        1,
        "Second identical request should be served from cache, not upstream"
    );
}
