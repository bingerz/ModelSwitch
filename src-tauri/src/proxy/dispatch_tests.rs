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
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::channel::manager::ChannelManager;
use crate::config::{AppConfig, ChannelConfig, GatewayConfig, SanitizerConfig};
use crate::credential::{create_credential_store, SharedCredentialStore};
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::ModelRegistry;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::provider::OpenAIAdaptor;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::{dispatch, RequestFormat};
use crate::proxy::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};
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

/// Build a streaming OpenAI chat completion JSON body (with `stream: true`).
fn streaming_request_body(model: &str, content: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
        "stream": true,
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
        excluded_models: vec![],
        proxy_url: None,
        headers: None,
        max_retries: None,
        models_endpoint: None,
        models_refresh_interval_secs: 0,
        tags: vec![],
    }
}

/// Build a disabled `ChannelConfig` pointing at the given base URL.
fn channel_config_disabled(id: &str, name: &str, base_url: &str, priority: u8) -> ChannelConfig {
    let mut cfg = channel_config(id, name, base_url, priority);
    cfg.enabled = false;
    cfg
}

/// Build a `ChannelConfig` with an account group tag.
fn channel_config_with_group(
    id: &str,
    name: &str,
    base_url: &str,
    priority: u8,
    group: &str,
) -> ChannelConfig {
    let mut cfg = channel_config(id, name, base_url, priority);
    cfg.account_group = Some(group.to_string());
    cfg
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
    build_test_state_with_opts(channel_configs, 0, HashMap::new())
}

/// Same as `build_test_state` but allows configuring `stream_bootstrap_retries`
/// and `model_fallbacks`.
fn build_test_state_with_opts(
    channel_configs: Vec<ChannelConfig>,
    stream_bootstrap_retries: u32,
    model_fallbacks: HashMap<String, Vec<String>>,
) -> Arc<AppState> {
    let config = test_config(channel_configs);
    let credential_store: SharedCredentialStore = create_credential_store(None);
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
    let model_registry = Arc::new(parking_lot::RwLock::new(ModelRegistry::new()));

    Arc::new(AppState {
        channel_mgr,
        credential_store,
        logger,
        audit_log: Arc::new(crate::admin::audit::AuditLog::with_default_capacity()),
        http_pool,
        gateway: ProxyParams {
            request_timeout_secs: Some(30),
            stream_keepalive_secs: None,
            stream_ttft_timeout_secs: Some(30),
            max_retries: config.gateway.max_retries,
            model_fallbacks,
            context_window_fallbacks: HashMap::new(),
            model_aliases: HashMap::new(),
            routing_strategy: crate::router::RoutingStrategyType::WeightedRandom,
            retry_base_ms: config.gateway.retry_base_ms,
            retry_max_ms: config.gateway.retry_max_ms,
            model_retry_overrides: HashMap::new(),
            nonstream_keepalive_interval_secs: 0,
            passthrough_headers: vec![],
            stream_bootstrap_retries,
            disable_image_generation: false,
            model_groups: HashMap::new(),
            model_pricing: HashMap::new(),
            completion_ratios: HashMap::new(),
            group_ratios: HashMap::new(),
        },
        router: RouterState {
            session_affinity: SessionAffinity::default(),
            active_requests,
            latency_tracker: Arc::new(crate::router::latency_tracker::LatencyTracker::new()),
            cooldown_tracker: Arc::new(crate::router::cooldown::CooldownTracker::new()),
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
            provider_budgets: Arc::new(crate::provider_budget::ProviderBudgetStore::new()),
            key_rate_limiter: Arc::new(crate::proxy::rate_limiter::KeyRateLimiter::new()),
        },
        mcp: McpState {
            mcp_manager,
            mcp_max_iterations: 5,
            mcp_auto_inject: false,
            mcp_gateway_enabled: false,
        },
        security: SecurityState {
            admin_token: None,
            admin_roles: vec![],
            sanitizer_config: Arc::new(parking_lot::RwLock::new(SanitizerConfig::default())),
            allowed_origins: None,
            trust_forwarded_headers: false,
            allow_open_proxy: false,
        },
        guardrails: Arc::new(crate::guardrails::GuardrailsChecker::new(
            crate::guardrails::GuardrailsConfig::default(),
        )),
        redemption_codes: Arc::new(crate::quota::RedemptionCodeStore::new()),
        notifications: Arc::new(crate::notification::NotificationService::new(
            crate::notification::NotificationConfig::default(),
        )),
        completion_ratios: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        routing_strategy: Arc::new(parking_lot::RwLock::new(
            crate::router::RoutingStrategyType::WeightedRandom,
        )),
        model_registry,
        ldap_config: None,
        oidc_config: None,
        oidc_state_secret: "test-state-secret".to_string(),
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

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

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

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

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

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

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
    let response1 = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;
    assert_eq!(response_status(&response1), 200);
    let json1 = response_json(response1).await;
    assert_eq!(json1["choices"][0]["message"]["content"], "Cached!");

    // Second identical request — should be served from cache
    let response2 = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;
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

/// Dispatch retries on 5xx — the first channel returns 500 (internal server
/// error), the second channel returns 200. Verify the request eventually
/// succeeds after retrying.
#[tokio::test]
async fn dispatch_retry_on_5xx() {
    let mock_fail = MockServer::start().await;
    let mock_success = MockServer::start().await;

    // First channel: always returns 500
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "Internal server error", "type": "server_error"}
        })))
        .mount(&mock_fail)
        .await;

    // Second channel: returns 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Recovered from 5xx!")),
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
    let body = chat_request_body("gpt-4", "Test 5xx retry");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed after retrying on the second channel"
    );
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Recovered from 5xx!"
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

/// Dispatch retries on connection error — the first channel points to a port
/// with no server (connection refused), the second channel returns 200.
/// Verify the request eventually succeeds.
#[tokio::test]
async fn dispatch_connection_error() {
    let mock_success = MockServer::start().await;

    // Second channel: returns 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Connection recovered!")),
        )
        .mount(&mock_success)
        .await;

    let state = build_test_state(vec![
        channel_config(
            "00000000-0000-0000-0000-000000000001",
            "dead-channel",
            "http://127.0.0.1:1",
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
    let body = chat_request_body("gpt-4", "Test connection error retry");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed after retrying past the connection error"
    );
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Connection recovered!"
    );

    // The success server should have been hit exactly once
    let success_requests = mock_success.received_requests().await.unwrap();
    assert_eq!(
        success_requests.len(),
        1,
        "The success channel should have received exactly one request"
    );
}

/// Cache miss on different bodies — send two requests with different bodies
/// to the same channel. Both should hit the upstream (no caching).
#[tokio::test]
async fn dispatch_cache_miss_different_body() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Response")),
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
    let provider = openai_provider();

    // First request with "first message"
    let body1 = chat_request_body("gpt-4", "first message");
    let response1 = dispatch(
        &state,
        &headers,
        &body1,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;
    assert_eq!(response_status(&response1), 200);

    // Second request with "second message" — different body, should NOT be cached
    let body2 = chat_request_body("gpt-4", "second message");
    let response2 = dispatch(
        &state,
        &headers,
        &body2,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;
    assert_eq!(response_status(&response2), 200);

    // The mock server should have received BOTH requests
    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        2,
        "Both requests with different bodies should hit the upstream, not cache"
    );
}

/// Streaming success — mock the upstream to return a streaming SSE response.
/// Verify dispatch returns 200 with Content-Type containing "text/event-stream".
#[tokio::test]
async fn dispatch_streaming_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n",
                ),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "streaming-channel",
        &mock_server.uri(),
        1,
    )]);

    let headers = HeaderMap::new();
    let body = streaming_request_body("gpt-4", "Stream test");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "streaming dispatch should return 200"
    );

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/event-stream"),
        "Content-Type should contain 'text/event-stream', got: {content_type}"
    );
}

/// No available channel — all channels are disabled. Verify dispatch returns
/// 429 (all channels exhausted).
#[tokio::test]
async fn dispatch_no_available_channel() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Should not reach")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config_disabled(
        "00000000-0000-0000-0000-000000000001",
        "disabled-channel",
        &mock_server.uri(),
        1,
    )]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "No channel available");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        429,
        "dispatch should return 429 when all channels are disabled"
    );
    let json = response_json(response).await;
    assert_eq!(
        json["error"]["code"], "all_channels_rate_limited",
        "error code should be all_channels_rate_limited"
    );

    // The mock server should not have received any requests
    let received = mock_server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "No requests should reach the upstream when all channels are disabled"
    );
}

// ── Account group routing tests ────────────────────────────────────────────

/// Request with `X-Account-Group: production` should only route to channels
/// tagged with "production" (or ungrouped channels), skipping channels tagged
/// with a different group — even if that channel has higher priority.
#[tokio::test]
async fn dispatch_with_account_group_header_routes_to_matching_channel() {
    let prod_server = MockServer::start().await;
    let staging_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Production!")),
        )
        .mount(&prod_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Staging!")),
        )
        .mount(&staging_server)
        .await;

    // Staging channel has priority 1 (higher), production has priority 2.
    // Without the header, staging would be preferred.
    let state = build_test_state(vec![
        channel_config_with_group(
            "00000000-0000-0000-0000-000000000001",
            "staging-channel",
            &staging_server.uri(),
            1,
            "staging",
        ),
        channel_config_with_group(
            "00000000-0000-0000-0000-000000000002",
            "prod-channel",
            &prod_server.uri(),
            2,
            "production",
        ),
    ]);

    let mut headers = HeaderMap::new();
    headers.insert("x-account-group", "production".parse().unwrap());
    let body = chat_request_body("gpt-4", "Route by group");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"], "Production!",
        "should route to the production channel when X-Account-Group: production is set"
    );

    // Staging server should never have been hit
    let staging_requests = staging_server.received_requests().await.unwrap();
    assert!(
        staging_requests.is_empty(),
        "staging channel should be excluded when X-Account-Group: production is set"
    );

    // Production server should have been hit
    let prod_requests = prod_server.received_requests().await.unwrap();
    assert_eq!(
        prod_requests.len(),
        1,
        "production channel should receive exactly one request"
    );
}

/// Request without the `X-Account-Group` header should route to all channels
/// (backward compatibility).
#[tokio::test]
async fn dispatch_without_account_group_header_uses_all_channels() {
    let primary_server = MockServer::start().await;
    let secondary_server = MockServer::start().await;

    for server in [&primary_server, &secondary_server] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(chat_completion_response("OK")),
            )
            .mount(server)
            .await;
    }

    let state = build_test_state(vec![
        channel_config_with_group(
            "00000000-0000-0000-0000-000000000001",
            "prod-channel",
            &primary_server.uri(),
            1,
            "production",
        ),
        channel_config_with_group(
            "00000000-0000-0000-0000-000000000002",
            "staging-channel",
            &secondary_server.uri(),
            1,
            "staging",
        ),
    ]);

    // No X-Account-Group header — all channels are eligible.
    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "No group header");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed when no account group header is present"
    );
}

/// Ungrouped channels are universal — they should be reachable even when an
/// `X-Account-Group` header is set for a different group.
#[tokio::test]
async fn dispatch_with_account_group_includes_ungrouped_channels() {
    let ungrouped_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Ungrouped!")),
        )
        .mount(&ungrouped_server)
        .await;

    // Only an ungrouped channel exists. Request with X-Account-Group: production
    // should still route to it because ungrouped channels are universal.
    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "ungrouped-channel",
        &ungrouped_server.uri(),
        1,
    )]);

    let mut headers = HeaderMap::new();
    headers.insert("x-account-group", "production".parse().unwrap());
    let body = chat_request_body("gpt-4", "Ungrouped fallback");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"], "Ungrouped!",
        "ungrouped channels should be reachable even with an account group header"
    );
}

/// When all channels belong to a non-matching group and no ungrouped channels
/// exist, dispatch should return 429 (all channels exhausted).
#[tokio::test]
async fn dispatch_with_non_matching_group_excludes_all_channels() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Should not reach")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config_with_group(
        "00000000-0000-0000-0000-000000000001",
        "staging-channel",
        &mock_server.uri(),
        1,
        "staging",
    )]);

    let mut headers = HeaderMap::new();
    headers.insert("x-account-group", "production".parse().unwrap());
    let body = chat_request_body("gpt-4", "No matching group");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        429,
        "dispatch should return 429 when no channels match the requested account group"
    );

    let received = mock_server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "upstream should not be called when no channels match the account group"
    );
}

// ── Per-channel custom header injection tests (P1.5) ────────────────────────

/// Build a `ChannelConfig` with custom headers.
fn channel_config_with_headers(
    id: &str,
    name: &str,
    base_url: &str,
    priority: u8,
    headers: HashMap<String, String>,
) -> ChannelConfig {
    let mut cfg = channel_config(id, name, base_url, priority);
    cfg.headers = Some(headers);
    cfg
}

/// Per-channel custom headers are forwarded to the upstream. Verify the mock
/// server receives the custom header.
#[tokio::test]
async fn dispatch_injects_custom_headers() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::header(
            "x-custom-header",
            "custom-value",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Headers OK!")),
        )
        .mount(&mock_server)
        .await;

    let mut headers = HashMap::new();
    headers.insert("x-custom-header".to_string(), "custom-value".to_string());

    let state = build_test_state(vec![channel_config_with_headers(
        "00000000-0000-0000-0000-000000000001",
        "custom-header-channel",
        &mock_server.uri(),
        1,
        headers,
    )]);

    let req_headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test custom headers");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &req_headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed — custom header should be forwarded"
    );
}

/// Denylisted headers (authorization, cookie, x-forwarded-for, etc.) must
/// NOT be forwarded. Verify that the upstream never sees them.
#[tokio::test]
async fn dispatch_skips_denylisted_custom_headers() {
    let mock_server = MockServer::start().await;

    // Mount a mock that expects requests WITHOUT the denylisted headers.
    // Use a non-conditional mock and verify the received request manually.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(chat_completion_response("OK")))
        .mount(&mock_server)
        .await;

    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "Bearer malicious".to_string());
    headers.insert("cookie".to_string(), "session=stolen".to_string());
    headers.insert("x-forwarded-for".to_string(), "10.0.0.1".to_string());
    headers.insert("x-safe-header".to_string(), "safe-value".to_string());

    let state = build_test_state(vec![channel_config_with_headers(
        "00000000-0000-0000-0000-000000000001",
        "denylist-channel",
        &mock_server.uri(),
        1,
        headers,
    )]);

    let req_headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test denylist");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &req_headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(response_status(&response), 200);

    // Check the request that was received
    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let req = &received[0];

    // Denylisted headers must NOT be present (beyond what apply_auth sets)
    // The authorization header set by channel.headers should be overwritten
    // by apply_auth with the real API key, not "Bearer malicious"
    let auth = req
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    assert_ne!(
        auth,
        Some("Bearer malicious"),
        "Denylisted 'authorization' from channel.headers must not override provider auth"
    );

    // Cookie must not be forwarded from channel config
    assert!(
        !req.headers.contains_key("cookie"),
        "Denylisted 'cookie' header must not be forwarded"
    );

    // x-forwarded-for must not be forwarded
    assert!(
        !req.headers.contains_key("x-forwarded-for"),
        "Denylisted 'x-forwarded-for' header must not be forwarded"
    );

    // Non-denylisted header should be present
    assert_eq!(
        req.headers
            .get("x-safe-header")
            .and_then(|v| v.to_str().ok()),
        Some("safe-value"),
        "Non-denylisted custom header should be forwarded"
    );
}

// ── Per-channel retry limit tests (P1.7) ────────────────────────────────────

/// Build a `ChannelConfig` with a custom `max_retries`.
fn channel_config_with_max_retries(
    id: &str,
    name: &str,
    base_url: &str,
    priority: u8,
    max_retries: u32,
) -> ChannelConfig {
    let mut cfg = channel_config(id, name, base_url, priority);
    cfg.max_retries = Some(max_retries);
    cfg
}

/// Channel with `max_retries = 1` should be skipped after the first attempt.
/// When the only available channel has a low retry cap, dispatch should fail
/// faster (fewer total requests) than with the global default.
#[tokio::test]
async fn dispatch_per_channel_retry_limit_caps_attempts() {
    let mock_fail = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "Always fails", "type": "server_error"}
        })))
        .mount(&mock_fail)
        .await;

    // Channel with max_retries = 1 — should only be tried once before being
    // skipped. The global max_retries defaults to 3.
    let state = build_test_state(vec![channel_config_with_max_retries(
        "00000000-0000-0000-0000-000000000001",
        "low-retry-channel",
        &mock_fail.uri(),
        1,
        1,
    )]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test per-channel retry limit");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    // Should return 429 (all exhausted)
    assert_eq!(response_status(&response), 429);

    let received = mock_fail.received_requests().await.unwrap();
    // With max_retries = 1, only the first attempt uses this channel.
    // Subsequent attempts (attempt 2, 3) exceed the cap and skip it.
    // So the mock should be hit exactly 1 time, not 3.
    assert_eq!(
        received.len(),
        1,
        "Channel with max_retries=1 should only receive 1 request, got {}",
        received.len()
    );
}

/// Channel with high `max_retries` should be retried normally.
#[tokio::test]
async fn dispatch_per_channel_retry_limit_allows_normal_retries() {
    let mock_fail = MockServer::start().await;
    let mock_success = MockServer::start().await;

    // First channel with max_retries = 5, always fails
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "fail", "type": "server_error"}
        })))
        .mount(&mock_fail)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Success!")),
        )
        .mount(&mock_success)
        .await;

    let state = build_test_state(vec![
        channel_config_with_max_retries(
            "00000000-0000-0000-0000-000000000001",
            "high-retry-fail",
            &mock_fail.uri(),
            1,
            5,
        ),
        channel_config(
            "00000000-0000-0000-0000-000000000002",
            "success-channel",
            &mock_success.uri(),
            2,
        ),
    ]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test normal retries");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    // Should succeed on the second channel
    assert_eq!(response_status(&response), 200);
}

// ── Bootstrap retry for streaming tests (P0.2) ──────────────────────────────

/// Bootstrap retry: when the first SSE chunk from upstream contains an error,
/// the gateway should silently retry on the next channel instead of forwarding
/// the error to the client.
#[tokio::test]
async fn dispatch_bootstrap_retry_on_stream_error() {
    let mock_error = MockServer::start().await;
    let mock_success = MockServer::start().await;

    // First channel: returns 200 OK but the SSE body is an error event
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    r#"data: {"error":{"message":"Internal error","type":"server_error"}}

"#,
                ),
        )
        .mount(&mock_error)
        .await;

    // Second channel: returns 200 OK with a valid SSE stream
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Recovered!\"}}]}\n\ndata: [DONE]\n\n",
                ),
        )
        .mount(&mock_success)
        .await;

    let state = build_test_state_with_opts(
        vec![
            channel_config(
                "00000000-0000-0000-0000-000000000001",
                "error-stream-channel",
                &mock_error.uri(),
                1,
            ),
            channel_config(
                "00000000-0000-0000-0000-000000000002",
                "success-stream-channel",
                &mock_success.uri(),
                2,
            ),
        ],
        2, // stream_bootstrap_retries = 2
        HashMap::new(),
    );

    let headers = HeaderMap::new();
    let body = streaming_request_body("gpt-4", "Test bootstrap retry");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    // Should succeed — bootstrap retry detected the error in the first stream
    // and retried on the second channel
    assert_eq!(
        response_status(&response),
        200,
        "bootstrap retry should detect error in first stream and retry"
    );

    // Both servers should have been hit
    let error_requests = mock_error.received_requests().await.unwrap();
    assert_eq!(
        error_requests.len(),
        1,
        "error stream channel should have received 1 request"
    );

    let success_requests = mock_success.received_requests().await.unwrap();
    assert_eq!(
        success_requests.len(),
        1,
        "success stream channel should have received 1 request"
    );
}

/// Bootstrap retry disabled (stream_bootstrap_retries = 0): the error SSE
/// event from upstream is forwarded directly to the client.
#[tokio::test]
async fn dispatch_bootstrap_retry_disabled_forwards_error() {
    let mock_error = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    r#"data: {"error":{"message":"Internal error","type":"server_error"}}

"#,
                ),
        )
        .mount(&mock_error)
        .await;

    // stream_bootstrap_retries = 0 (disabled) — the error is forwarded
    let state = build_test_state_with_opts(
        vec![channel_config(
            "00000000-0000-0000-0000-000000000001",
            "error-stream-channel",
            &mock_error.uri(),
            1,
        )],
        0, // stream_bootstrap_retries = 0 (disabled)
        HashMap::new(),
    );

    let headers = HeaderMap::new();
    let body = streaming_request_body("gpt-4", "Test bootstrap disabled");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    // With bootstrap disabled, the 200 response with the error SSE is forwarded
    assert_eq!(
        response_status(&response),
        200,
        "with bootstrap disabled, the upstream response is forwarded directly"
    );

    // Only one request to the upstream — no retry
    let error_requests = mock_error.received_requests().await.unwrap();
    assert_eq!(
        error_requests.len(),
        1,
        "with bootstrap disabled, only 1 request to upstream"
    );
}

// ── Rate limit enforcement ─────────────────────────────────────────────────

/// Per-channel RPM limit is enforced: the first request succeeds but increments
/// the RPM counter. The second request (with a different body to bypass cache)
/// finds the channel at its RPM cap and is denied — with only one channel,
/// dispatch returns 429 (all exhausted).
#[tokio::test]
async fn dispatch_rate_limit_enforcement() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Rate limited!")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "rate-limited-channel",
        &mock_server.uri(),
        1,
    )]);

    // Set RPM limit to 1 — only one request per minute is allowed.
    let ch_uuid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    state.limits.rate_limiter.set_channel_rpm_limit(ch_uuid, 1);

    let headers = HeaderMap::new();
    let provider = openai_provider();

    // First request — should succeed (RPM counter is 0, limit is 1)
    let body1 = chat_request_body("gpt-4", "first request");
    let response1 =
        dispatch(&state, &headers, &body1, &provider, RequestFormat::OpenAIChat).await;
    assert_eq!(response_status(&response1), 200);

    // Second request with a different body (different cache key) — the channel
    // is now rate-limited (RPM counter is 1, limit is 1). With only one channel,
    // dispatch should return 429 (all channels exhausted).
    let body2 = chat_request_body("gpt-4", "second request");
    let response2 =
        dispatch(&state, &headers, &body2, &provider, RequestFormat::OpenAIChat).await;
    assert_eq!(
        response_status(&response2),
        429,
        "second request should be rate-limited with only one channel"
    );

    // The mock server should have been hit exactly once
    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        1,
        "upstream should receive exactly 1 request when the channel is rate-limited after the first"
    );
}

// ── In-flight request coalescing ────────────────────────────────────────────

/// Two identical concurrent requests should be coalesced: the first registers
/// an in-flight entry and dispatches upstream; the second waits for it to
/// complete, then serves the cached response. The upstream mock is hit exactly
/// once despite two client requests.
#[tokio::test]
async fn dispatch_in_flight_coalescing() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_string(chat_completion_response("Coalesced!")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "coalescing-channel",
        &mock_server.uri(),
        1,
    )]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "coalescing test");
    let provider = openai_provider();

    // Fire two identical requests concurrently. The second should coalesce
    // onto the first's in-flight entry, then serve from cache after the
    // first completes.
    let (response1, response2) = tokio::join!(
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat),
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat),
    );

    assert_eq!(response_status(&response1), 200);
    assert_eq!(response_status(&response2), 200);

    // The mock server should have been hit exactly once — the second request
    // was coalesced and served from cache.
    let received = mock_server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        1,
        "concurrent identical requests should be coalesced — upstream hit once, got {}",
        received.len()
    );
}

// ── Disabled channel is skipped ─────────────────────────────────────────────

/// When one channel is disabled and another is enabled, dispatch should route
/// exclusively to the enabled channel. The disabled channel's upstream should
/// receive zero requests.
#[tokio::test]
async fn dispatch_disabled_channel_is_skipped() {
    let mock_disabled = MockServer::start().await;
    let mock_enabled = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Disabled channel")),
        )
        .mount(&mock_disabled)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Enabled channel")),
        )
        .mount(&mock_enabled)
        .await;

    let state = build_test_state(vec![
        channel_config_disabled(
            "00000000-0000-0000-0000-000000000001",
            "disabled-channel",
            &mock_disabled.uri(),
            1,
        ),
        channel_config(
            "00000000-0000-0000-0000-000000000002",
            "enabled-channel",
            &mock_enabled.uri(),
            2,
        ),
    ]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test disabled channel skip");
    let provider = openai_provider();

    let response =
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat).await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Enabled channel",
        "should route to the enabled channel, not the disabled one"
    );

    // The disabled channel's server should not have received any requests
    let disabled_requests = mock_disabled.received_requests().await.unwrap();
    assert!(
        disabled_requests.is_empty(),
        "disabled channel should receive zero requests"
    );

    // The enabled channel's server should have received exactly one request
    let enabled_requests = mock_enabled.received_requests().await.unwrap();
    assert_eq!(
        enabled_requests.len(),
        1,
        "enabled channel should receive exactly one request"
    );
}

// ── Priority ordering ───────────────────────────────────────────────────────

/// Channels are grouped by priority tier (lower number = higher priority).
/// WeightedRandom only selects within a single tier, so a channel at priority 1
/// is always chosen over a channel at priority 2 when both are healthy.
#[tokio::test]
async fn dispatch_priority_ordering() {
    let mock_high = MockServer::start().await;
    let mock_low = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("High priority!")),
        )
        .mount(&mock_high)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Low priority!")),
        )
        .mount(&mock_low)
        .await;

    // Channel A: priority 1 (higher), Channel B: priority 2 (lower).
    // WeightedRandom groups by priority tier — since A is the only candidate
    // in tier 1, it is always selected first.
    let state = build_test_state(vec![
        channel_config(
            "00000000-0000-0000-0000-000000000001",
            "high-priority",
            &mock_high.uri(),
            1,
        ),
        channel_config(
            "00000000-0000-0000-0000-000000000002",
            "low-priority",
            &mock_low.uri(),
            2,
        ),
    ]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test priority ordering");
    let provider = openai_provider();

    let response =
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat).await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "High priority!",
        "should route to the higher-priority channel"
    );

    // The high-priority server should have been hit
    let high_requests = mock_high.received_requests().await.unwrap();
    assert_eq!(
        high_requests.len(),
        1,
        "high-priority channel should receive exactly one request"
    );

    // The low-priority server should NOT have been hit
    let low_requests = mock_low.received_requests().await.unwrap();
    assert!(
        low_requests.is_empty(),
        "low-priority channel should receive zero requests when high-priority succeeds"
    );
}

// ── Excluded models filter ──────────────────────────────────────────────────

/// A channel whose `excluded_models` list contains the requested model is
/// filtered out during channel selection. Dispatch falls through to the next
/// available channel that does not exclude the model.
#[tokio::test]
async fn dispatch_excluded_models_skips_channel() {
    let mock_excluded = MockServer::start().await;
    let mock_allowed = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Excluded channel")),
        )
        .mount(&mock_excluded)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Allowed channel")),
        )
        .mount(&mock_allowed)
        .await;

    // Channel A excludes "gpt-4" but has priority 1 (would be preferred if not excluded).
    // Channel B has no exclusions and priority 2.
    let mut channel_a = channel_config(
        "00000000-0000-0000-0000-000000000001",
        "excluded-models-channel",
        &mock_excluded.uri(),
        1,
    );
    channel_a.excluded_models = vec!["gpt-4".to_string()];

    let channel_b = channel_config(
        "00000000-0000-0000-0000-000000000002",
        "allowed-channel",
        &mock_allowed.uri(),
        2,
    );

    let state = build_test_state(vec![channel_a, channel_b]);

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test excluded models");
    let provider = openai_provider();

    let response =
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat).await;

    assert_eq!(response_status(&response), 200);
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Allowed channel",
        "should route to the channel without model exclusion"
    );

    // The excluded channel should not have received any requests
    let excluded_requests = mock_excluded.received_requests().await.unwrap();
    assert!(
        excluded_requests.is_empty(),
        "channel excluding gpt-4 should receive zero requests for that model"
    );

    // The allowed channel should have received exactly one request
    let allowed_requests = mock_allowed.received_requests().await.unwrap();
    assert_eq!(
        allowed_requests.len(),
        1,
        "channel without exclusion should receive exactly one request"
    );
}

// ── Model fallback chain (business flow) ───────────────────────────────────

/// When the primary model fails, dispatch falls back to the configured
/// alternative model. Uses two channels so the circuit breaker on the
/// failing channel does not block the fallback channel.
#[tokio::test]
async fn dispatch_model_fallback_chain() {
    let mock_fail = MockServer::start().await;
    let mock_success = MockServer::start().await;

    // Failing upstream — always returns 500.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "Model unavailable", "type": "server_error"}
        })))
        .mount(&mock_fail)
        .await;

    // Success upstream — always returns 200.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(chat_completion_response("Fallback success!")),
        )
        .mount(&mock_success)
        .await;

    let mut fallbacks = HashMap::new();
    fallbacks.insert(
        "gpt-4".to_string(),
        vec!["gpt-3.5-turbo".to_string()],
    );

    // Channel A (priority 1): excludes gpt-4 so it only handles fallback models.
    let mut channel_a = channel_config(
        "00000000-0000-0000-0000-000000000001",
        "fallback-channel",
        &mock_success.uri(),
        1,
    );
    channel_a.excluded_models = vec!["gpt-4".to_string()];

    // Channel B (priority 2): accepts gpt-4 but will fail.
    let channel_b = channel_config(
        "00000000-0000-0000-0000-000000000002",
        "primary-channel",
        &mock_fail.uri(),
        2,
    );

    let state = build_test_state_with_opts(
        vec![channel_a, channel_b],
        0,
        fallbacks,
    );

    let headers = HeaderMap::new();
    let body = chat_request_body("gpt-4", "Test model fallback");
    let provider = openai_provider();

    let response = dispatch(
        &state,
        &headers,
        &body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed by falling back from gpt-4 to gpt-3.5-turbo"
    );
    let json = response_json(response).await;
    assert_eq!(
        json["choices"][0]["message"]["content"],
        "Fallback success!",
        "response content should come from the fallback model attempt"
    );

    // Verify the fail upstream received exactly 1 request (for gpt-4).
    let fail_requests = mock_fail.received_requests().await.unwrap();
    assert_eq!(
        fail_requests.len(),
        1,
        "fail channel should receive exactly 1 request for gpt-4"
    );

    // Verify the success upstream received exactly 1 request (for gpt-3.5-turbo).
    let success_requests = mock_success.received_requests().await.unwrap();
    assert_eq!(
        success_requests.len(),
        1,
        "success channel should receive exactly 1 request for gpt-3.5-turbo fallback"
    );
}

// ── Virtual key billing: success charges ───────────────────────────────────

/// A successful request through a virtual key charges the key's budget.
/// After dispatch returns 200, the key's daily/monthly spend should be
/// non-zero.
#[tokio::test]
async fn dispatch_virtual_key_billing_success_charges() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response("Billed!")),
        )
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "billing-channel",
        &mock_server.uri(),
        1,
    )]);

    // Create a virtual key with budget limits so reserve_spend is triggered.
    let (vk, _plaintext) = state
        .billing
        .virtual_key_store
        .create(
            "test-billing-key".to_string(),
            Some(1000),  // daily_budget_cents
            Some(10000), // monthly_budget_cents
            None,        // allowed_models (all)
            vec![],      // denied_models
            vec![],      // allowed_ips
            None,        // rpm_limit
            None,        // tpm_limit
            None,        // expires_at
            None,        // group
        )
        .await;

    // Inject the virtual key ID via the header that the middleware normally sets.
    let mut headers = HeaderMap::new();
    headers.insert("x-virtual-key-id", vk.id.to_string().parse().unwrap());

    let body = chat_request_body("gpt-4", "Test billing");
    let provider = openai_provider();

    let response =
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat).await;

    assert_eq!(
        response_status(&response),
        200,
        "dispatch should succeed"
    );

    // After the successful dispatch, the key's spend should be non-zero.
    let fetched = state
        .billing
        .virtual_key_store
        .get(vk.id)
        .await
        .expect("virtual key should exist");
    assert!(
        fetched.spend.today.cents > 0,
        "daily spend should be non-zero after a successful billed request, got {}",
        fetched.spend.today.cents
    );
    assert!(
        fetched.spend.this_month.cents > 0,
        "monthly spend should be non-zero after a successful billed request, got {}",
        fetched.spend.this_month.cents
    );
}

// ── Virtual key bill: failure refunds ──────────────────────────────────────

/// When all channels are exhausted, the reserved budget is refunded. After
/// dispatch returns 429, the key's spend should be zero (reserved then
/// reconciled to 0).
#[tokio::test]
async fn dispatch_virtual_key_billing_failure_refunds() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "Internal server error", "type": "server_error"}
        })))
        .mount(&mock_server)
        .await;

    let state = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "fail-channel",
        &mock_server.uri(),
        1,
    )]);

    // Create a virtual key with a daily budget so reserve_spend is triggered.
    let (vk, _plaintext) = state
        .billing
        .virtual_key_store
        .create(
            "test-refund-key".to_string(),
            Some(1000),  // daily_budget_cents
            Some(10000), // monthly_budget_cents
            None,        // allowed_models (all)
            vec![],      // denied_models
            vec![],      // allowed_ips
            None,        // rpm_limit
            None,        // tpm_limit
            None,        // expires_at
            None,        // group
        )
        .await;

    let mut headers = HeaderMap::new();
    headers.insert("x-virtual-key-id", vk.id.to_string().parse().unwrap());

    let body = chat_request_body("gpt-4", "Test refund");
    let provider = openai_provider();

    let response =
        dispatch(&state, &headers, &body, &provider, RequestFormat::OpenAIChat).await;

    // All channels exhausted → 429
    assert_eq!(
        response_status(&response),
        429,
        "dispatch should return 429 when all channels fail"
    );

    // The reservation should have been refunded — spend back to zero.
    let fetched = state
        .billing
        .virtual_key_store
        .get(vk.id)
        .await
        .expect("virtual key should exist");
    assert_eq!(
        fetched.spend.today.cents, 0,
        "daily spend should be 0 after reservation refund on failure"
    );
    assert_eq!(
        fetched.spend.this_month.cents, 0,
        "monthly spend should be 0 after reservation refund on failure"
    );
    assert_eq!(
        fetched.spend.total_cents, 0,
        "total spend should be 0 after reservation refund on failure"
    );
}
