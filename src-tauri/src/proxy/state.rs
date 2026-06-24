use crate::channel::manager::ChannelManager;
use crate::credential::SharedCredentialStore;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::ModelRegistry;
use crate::proxy::cache::{InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::quota::SharedQuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::virtual_key::SharedVirtualKeyStore;
use parking_lot::RwLock as StdRwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Proxy parameters (timeouts, retries, fallback strategy).
pub struct ProxyParams {
    pub request_timeout_secs: Option<u64>,
    pub stream_keepalive_secs: Option<u64>,
    pub stream_ttft_timeout_secs: Option<u64>,
    pub max_retries: u32,
    pub model_fallbacks: HashMap<String, Vec<String>>,
    pub context_window_fallbacks: HashMap<String, Vec<String>>,
    pub model_aliases: HashMap<String, String>,
    pub routing_strategy: crate::router::RoutingStrategyType,
    pub retry_base_ms: u64,
    pub retry_max_ms: u64,
    pub model_retry_overrides: HashMap<String, crate::config::ModelRetryConfig>,
    pub nonstream_keepalive_interval_secs: u64,
    /// Upstream response headers to forward to the client.
    /// When empty, the built-in default passthrough list is used.
    pub passthrough_headers: Vec<String>,
    /// Number of bootstrap retry attempts for streaming requests (default 0).
    pub stream_bootstrap_retries: u32,
    /// When true, image generation endpoints return 404.
    pub disable_image_generation: bool,
}

/// Router state (session affinity, active request tracking, latency tracking).
pub struct RouterState {
    pub session_affinity: SessionAffinity,
    pub active_requests: Arc<ActiveRequests>,
    pub latency_tracker: Arc<crate::router::latency_tracker::LatencyTracker>,
}

/// Cache state (request cache + coalescing).
pub struct CacheState {
    pub request_cache: Arc<RequestCache>,
    pub in_flight: Arc<InFlightRequests>,
}

/// Limits state (rate limiter + payload rules).
pub struct LimitsState {
    pub payload_rules: Arc<ChannelPayloadRules>,
    pub rate_limiter: Arc<RateLimiter>,
}

/// Billing state (quota + virtual key + provider budget tracking).
pub struct BillingState {
    pub quota_store: SharedQuotaStore,
    pub virtual_key_store: SharedVirtualKeyStore,
    pub provider_budgets: crate::provider_budget::SharedProviderBudgetStore,
}

/// MCP integration state.
pub struct McpState {
    pub mcp_manager: Arc<McpManager>,
    pub mcp_max_iterations: u32,
    pub mcp_auto_inject: bool,
    pub mcp_gateway_enabled: bool,
}

/// Security state (auth + sanitizer + CORS).
pub struct SecurityState {
    pub admin_token: Option<String>,
    pub sanitizer_config: crate::config::SanitizerConfig,
    pub allowed_origins: Option<Vec<String>>,
}

/// Shared application state for the proxy.
pub struct AppState {
    pub channel_mgr: Arc<ChannelManager>,
    pub credential_store: SharedCredentialStore,
    pub logger: Arc<DispatchLogger>,
    pub http_pool: crate::http_pool::HttpPool,
    pub model_registry: Arc<StdRwLock<ModelRegistry>>,
    pub gateway: ProxyParams,
    pub router: RouterState,
    pub cache: CacheState,
    pub limits: LimitsState,
    pub billing: BillingState,
    pub mcp: McpState,
    pub security: SecurityState,
    pub started_at: std::time::Instant,
}
