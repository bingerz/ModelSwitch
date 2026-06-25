//! Shared test helpers for constructing `AppState` and related test fixtures.
//!
//! This module is only compiled under `#[cfg(test)]`. It extracts the
//! `build_test_state()` pattern from `dispatch_tests.rs` so that every
//! test module across the crate can construct a realistic `AppState`
//! without duplicating boilerplate.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::channel::manager::ChannelManager;
use crate::config::{AppConfig, ChannelConfig, GatewayConfig, SanitizerConfig};
use crate::credential::{create_credential_store, SharedCredentialStore};
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::ModelRegistry;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};
use crate::quota::QuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::virtual_key::VirtualKeyStore;

/// Build a `ChannelConfig` pointing at the given base URL.
pub(crate) fn channel_config(id: &str, name: &str, base_url: &str, priority: u8) -> ChannelConfig {
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

/// Build a minimal `AppConfig` with the given channels.
pub(crate) fn test_config(channels: Vec<ChannelConfig>) -> AppConfig {
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
pub(crate) fn build_test_state(channel_configs: Vec<ChannelConfig>) -> Arc<AppState> {
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
        audit_log: Arc::new(crate::admin::audit::AuditLog::with_default_capacity()),
        http_pool,
        model_registry: Arc::new(parking_lot::RwLock::new(ModelRegistry::new())),
        gateway: ProxyParams {
            request_timeout_secs: Some(30),
            stream_keepalive_secs: None,
            stream_ttft_timeout_secs: Some(30),
            max_retries: config.gateway.max_retries,
            model_fallbacks: HashMap::new(),
            context_window_fallbacks: HashMap::new(),
            model_aliases: HashMap::new(),
            routing_strategy: crate::router::RoutingStrategyType::WeightedRandom,
            retry_base_ms: config.gateway.retry_base_ms,
            retry_max_ms: config.gateway.retry_max_ms,
            model_retry_overrides: HashMap::new(),
            nonstream_keepalive_interval_secs: 0,
            passthrough_headers: vec![],
            stream_bootstrap_retries: 0,
            disable_image_generation: false,
            model_groups: HashMap::new(),
            model_pricing: HashMap::new(),
            completion_ratios: HashMap::new(),
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
            allowed_origins: None,
        },
        guardrails: Arc::new(crate::guardrails::GuardrailsChecker::new(
            crate::guardrails::GuardrailsConfig::default(),
        )),
        redemption_codes: Arc::new(crate::quota::RedemptionCodeStore::new()),
        notifications: Arc::new(crate::notification::NotificationService::new(
            crate::notification::NotificationConfig::default(),
        )),
        completion_ratios: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        started_at: std::time::Instant::now(),
    })
}

/// Extract the HTTP status code from an axum Response.
pub(crate) fn response_status(response: &axum::response::Response) -> u16 {
    response.status().as_u16()
}

/// Buffer the axum Response body and parse as JSON.
pub(crate) async fn response_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    serde_json::from_slice(&bytes).expect("response body is not valid JSON")
}
