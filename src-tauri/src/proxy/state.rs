use crate::admin::audit::AuditLog;
use crate::channel::manager::ChannelManager;
use crate::credential::SharedCredentialStore;
use crate::guardrails::GuardrailsChecker;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::ModelRegistry;
use crate::notification::NotificationService;
use crate::proxy::cache::{InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::{KeyRateLimiter, RateLimiter};
use crate::quota::{RedemptionCodeStore, SharedQuotaStore};
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
    /// Named model groups for routing and access control.
    /// Key = group name, Value = list of model names in the group.
    pub model_groups: HashMap<String, Vec<String>>,
    /// Per-model pricing overrides. Key = model name.
    pub model_pricing: HashMap<String, crate::config::ModelPricing>,
    /// Per-model completion ratio multiplier (default 1.0). Adjusts output
    /// token cost relative to input. Example: {"gpt-4": 2.0}.
    pub completion_ratios: HashMap<String, f64>,
    /// Group ratio multipliers for budgeting. Key = group name, Value = ratio.
    /// Example: {"premium": 1.5, "economy": 0.5}
    pub group_ratios: HashMap<String, f64>,
}

/// Router state (session affinity, active request tracking, latency tracking).
pub struct RouterState {
    pub session_affinity: SessionAffinity,
    pub active_requests: Arc<ActiveRequests>,
    pub latency_tracker: Arc<crate::router::latency_tracker::LatencyTracker>,
    pub cooldown_tracker: Arc<crate::router::cooldown::CooldownTracker>,
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
    pub key_rate_limiter: Arc<KeyRateLimiter>,
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
    /// Additional admin tokens with associated roles for RBAC.
    /// Each tuple is (token, Role). The legacy `admin_token` is always SuperAdmin.
    pub admin_roles: Vec<(String, crate::middleware::rbac::Role)>,
    /// Runtime-updatable sanitizer configuration wrapped in `Arc<RwLock<...>>`
    /// so the admin API can mutate it without restarting the gateway.
    pub sanitizer_config: Arc<StdRwLock<crate::config::SanitizerConfig>>,
    pub allowed_origins: Option<Vec<String>>,
    pub trust_forwarded_headers: bool,
    /// Whether open-proxy mode (no virtual key auth) is explicitly allowed
    /// on non-loopback bind addresses.
    pub allow_open_proxy: bool,
}

/// Shared application state for the proxy.
pub struct AppState {
    pub channel_mgr: Arc<ChannelManager>,
    pub credential_store: SharedCredentialStore,
    pub logger: Arc<DispatchLogger>,
    pub audit_log: Arc<AuditLog>,
    pub http_pool: crate::http_pool::HttpPool,
    pub model_registry: Arc<StdRwLock<ModelRegistry>>,
    pub gateway: ProxyParams,
    pub router: RouterState,
    pub cache: CacheState,
    pub limits: LimitsState,
    pub billing: BillingState,
    pub mcp: McpState,
    pub security: SecurityState,
    /// Content moderation guardrails checker (runtime-updatable).
    pub guardrails: Arc<GuardrailsChecker>,
    /// Redemption code store for credit grants.
    pub redemption_codes: Arc<RedemptionCodeStore>,
    /// Notification service for budget and channel alerts.
    pub notifications: Arc<NotificationService>,
    /// Runtime-updatable completion ratios (mirrors the startup value from
    /// [`ProxyParams::completion_ratios`] but mutable at runtime via admin API).
    pub completion_ratios: Arc<parking_lot::RwLock<HashMap<String, f64>>>,
    /// Runtime-updatable routing strategy (mirrors the startup value from
    /// [`ProxyParams::routing_strategy`] but mutable at runtime via admin API).
    pub routing_strategy: Arc<parking_lot::RwLock<crate::router::RoutingStrategyType>>,
    /// LDAP/AD authentication configuration. None = LDAP disabled.
    pub ldap_config: Option<crate::config::LdapConfig>,
    /// OIDC SSO configuration. None = OIDC disabled.
    pub oidc_config: Option<crate::config::OidcConfig>,
    /// Random secret for stateless OIDC CSRF state token signing.
    pub oidc_state_secret: String,
    pub started_at: std::time::Instant,
}
