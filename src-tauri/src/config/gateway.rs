use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::auth::AuthConfig;
use super::security::SanitizerConfig;

/// Per-model pricing overrides. When present, these rates override the
/// channel-level `input_cost_per_mtok` / `output_cost_per_mtok` for the
/// matching model key.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    #[serde(default)]
    pub input_cost_per_mtok: Option<f64>,
    #[serde(default)]
    pub output_cost_per_mtok: Option<f64>,
}

/// Per-model retry configuration overrides.
/// Any field set to `None` falls back to the global gateway default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRetryConfig {
    /// Override the global max_retries for this model.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Override the global retry_base_ms for this model.
    #[serde(default)]
    pub retry_base_ms: Option<u64>,
    /// Override the global retry_max_ms for this model.
    #[serde(default)]
    pub retry_max_ms: Option<u64>,
}

/// TLS configuration for native HTTPS binding.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    /// Enable TLS/HTTPS binding.
    #[serde(default)]
    pub enable: bool,
    /// Path to PEM-encoded certificate file.
    #[serde(default)]
    pub cert: String,
    /// Path to PEM-encoded private key file.
    #[serde(default)]
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_circuit_breaker_minutes")]
    pub circuit_breaker_minutes: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub model_fallbacks: HashMap<String, Vec<String>>,
    /// Context window fallback models. When a request fails due to context
    /// length exceeded, the gateway tries these alternative models in order.
    /// Key = original model, Value = list of models with larger context windows.
    /// They are appended to the regular fallback chain during dispatch.
    #[serde(default)]
    pub context_window_fallbacks: HashMap<String, Vec<String>>,
    /// Gateway-level model aliases. The key (alias) is replaced by the value
    /// (canonical model) before channel selection and fallback resolution.
    /// This is different from per-channel model_mapping, which maps at the
    /// upstream level.
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    #[serde(default = "default_routing_strategy")]
    pub routing_strategy: crate::router::RoutingStrategyType,
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
    #[serde(default = "default_health_check_enabled")]
    pub health_check_enabled: bool,
    /// Bearer token for /api/* endpoints. None = no auth. Env MODELSWITCH_ADMIN_TOKEN takes precedence.
    #[serde(default)]
    pub admin_token: Option<String>,
    /// Additional admin tokens with role assignments for RBAC.
    /// When non-empty, role-based permission checks are enforced.
    #[serde(default)]
    pub admin_tokens: Vec<super::auth::AdminTokenConfig>,
    /// Directory containing web console static files (default None = disabled).
    /// When set, the gateway serves a web UI at `/` for browser-based management.
    #[serde(default)]
    pub web_console_dir: Option<String>,
    /// Maximum time (seconds) to wait for in-flight requests during shutdown.
    #[serde(default = "default_drain_timeout_secs")]
    pub drain_timeout_secs: u64,
    /// Per-request timeout in seconds for the dispatch retry loop (default 120).
    #[serde(default)]
    pub request_timeout_secs: Option<u64>,
    /// Streaming keepalive interval in seconds (emit SSE comments to prevent idle timeout).
    #[serde(default)]
    pub stream_keepalive_secs: Option<u64>,
    /// Request cache TTL in seconds (default 300).
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Maximum number of cached responses (default 1000).
    #[serde(default = "default_max_cache_entries")]
    pub max_cache_entries: usize,
    /// Cache mode: "on", "off", "readonly", or "writeonly" (default "on").
    #[serde(default = "default_cache_mode")]
    pub cache_mode: String,
    /// HTTP client timeout in seconds (default 300).
    #[serde(default = "default_http_timeout_secs")]
    pub http_timeout_secs: u64,
    /// Session affinity TTL in seconds (default 1800).
    #[serde(default = "default_affinity_ttl_secs")]
    pub affinity_ttl_secs: u64,
    /// Max dispatch log entries to keep in memory (default 1000).
    #[serde(default = "default_log_max_entries")]
    pub log_max_entries: usize,
    /// Quota poller interval in seconds (default 300).
    #[serde(default = "default_quota_poll_interval_secs")]
    pub quota_poll_interval_secs: u64,
    /// Maximum MCP tool-call loop iterations (default 5).
    #[serde(default = "default_mcp_max_iterations")]
    pub mcp_max_iterations: u32,
    /// Whether to auto-inject MCP tools into chat completion requests (default true).
    #[serde(default = "default_mcp_auto_inject")]
    pub mcp_auto_inject: bool,
    /// Privacy guardrail — redacts secrets from request bodies before forwarding.
    #[serde(default)]
    pub sanitizer: SanitizerConfig,
    /// Enable MCP Gateway Mode — expose /mcp endpoint for MCP clients (default true).
    #[serde(default = "default_mcp_gateway_enabled")]
    pub mcp_gateway_enabled: bool,
    /// Time-to-first-token (TTFT) timeout in seconds for streaming requests.
    /// If the first byte of a streaming response doesn't arrive within this
    /// duration, the request is aborted and retried on the next channel.
    /// None or 0 = no TTFT timeout (use only the overall request timeout).
    #[serde(default = "default_stream_ttft_timeout_secs")]
    pub stream_ttft_timeout_secs: Option<u64>,
    /// Number of HTTP client instances in the connection pool.
    /// Each client maintains a separate TCP connection per host.
    /// HTTP/2 limits concurrent streams to ~100 per connection.
    /// Pool size × 100 = max concurrent upstream requests.
    /// Default: 8 (800 concurrent streams). Increase for higher throughput.
    #[serde(default = "default_http_pool_size")]
    pub http_pool_size: usize,
    /// Per-provider budget limits. Key = provider name (e.g., "openai", "anthropic").
    #[serde(default)]
    pub provider_budgets: HashMap<String, crate::provider_budget::ProviderBudgetConfig>,
    /// Base delay in milliseconds for exponential retry backoff (default 100).
    #[serde(default = "default_retry_base_ms")]
    pub retry_base_ms: u64,
    /// Maximum delay in milliseconds for retry backoff (default 5000).
    #[serde(default = "default_retry_max_ms")]
    pub retry_max_ms: u64,
    /// Per-model retry overrides. Key = model name (supports exact match and
    /// wildcard patterns like "gpt-4*", same as model_fallbacks).
    /// Fields not specified fall back to the global defaults.
    #[serde(default)]
    pub model_retry_overrides: HashMap<String, ModelRetryConfig>,
    /// Allowed CORS origins for web console mode.
    /// When set, only these origins may access the gateway via browser.
    /// When empty or None, defaults to localhost-only in release builds,
    /// permissive in debug builds.
    #[serde(default)]
    pub allowed_origins: Option<Vec<String>>,
    /// Interval (seconds) for sending keepalive whitespace on non-streaming
    /// responses. 0 = disabled (default). Recommended: 10-15 for long
    /// inference times. When enabled, the gateway sends newline bytes as
    /// HTTP chunked transfer encoding while waiting for the upstream
    /// response, preventing client-side TCP timeouts.
    #[serde(default)]
    pub nonstream_keepalive_interval_secs: u64,
    /// Upstream response headers to forward to the client.
    /// If empty, defaults to the built-in passthrough list (rate limit headers, x-request-id).
    #[serde(default)]
    pub passthrough_headers: Vec<String>,
    /// Maximum log file size in MB before rotation (default 100).
    #[serde(default = "default_log_max_file_size_mb")]
    pub log_max_file_size_mb: u64,
    /// Maximum number of rotated log files to retain (default 5).
    #[serde(default = "default_log_max_files")]
    pub log_max_files: usize,
    /// Number of bootstrap retry attempts for streaming requests.
    /// When > 0, the gateway peeks at the first SSE chunk before forwarding
    /// to the client. If the first chunk indicates an upstream error (e.g.,
    /// error event in the stream), the request is silently retried on the
    /// next channel instead of forwarding the error to the client.
    /// Default: 0 (disabled).
    #[serde(default)]
    pub stream_bootstrap_retries: u32,
    /// When true, globally disables circuit breaker cooldown on failures.
    /// Useful for testing or deployments where cooling is not desired.
    #[serde(default)]
    pub disable_cooling: bool,
    /// When true, image generation endpoints return 404.
    /// When `"chat"`, image generation is only disabled for chat completions
    /// (not applicable yet since image endpoints are separate).
    /// Currently supports: false, true.
    #[serde(default)]
    pub disable_image_generation: bool,
    /// Named model groups for routing, access control, and organization.
    /// Key = group name, Value = list of model names in the group.
    /// Clients can request a group name (e.g., `"model": "reasoning"`) and the
    /// gateway resolves it to the first available model in the group.
    /// Group names can also be used in virtual key `allowed_models` to grant
    /// access to all models in the group.
    #[serde(default)]
    pub model_groups: HashMap<String, Vec<String>>,
    /// Per-model pricing overrides. Key = model name. When an entry exists,
    /// its `input_cost_per_mtok` / `output_cost_per_mtok` override the
    /// channel-level rates for cost calculation.
    #[serde(default)]
    pub model_pricing: HashMap<String, ModelPricing>,
    /// Per-model completion ratio multiplier. Default 1.0 (no adjustment).
    /// Example: {"gpt-4": 2.0} means output tokens cost 2x their base price.
    #[serde(default)]
    pub completion_ratios: HashMap<String, f64>,
    /// Group ratio for budgeting. Maps group name to ratio multiplier.
    /// Example: {"premium": 1.5, "economy": 0.5}
    #[serde(default)]
    pub group_ratios: HashMap<String, f64>,
    /// TLS configuration for native HTTPS binding.
    #[serde(default)]
    pub tls: TlsConfig,
    /// Notification/alert configuration for budget thresholds and channel events.
    #[serde(default)]
    pub notification: crate::notification::NotificationConfig,
    /// Rate-limiting algorithm for per-channel RPM/TPM enforcement.
    /// "sliding_window" (default) uses the existing rolling-window counters.
    /// "token_bucket" uses a burst-capable token bucket.
    #[serde(default)]
    pub rate_limit_algorithm: crate::proxy::rate_limiter::RateLimitAlgorithm,
    /// Enterprise authentication (LDAP/AD, OIDC SSO).
    #[serde(default)]
    pub auth: AuthConfig,
}

// ── Default value functions ───────────────────────────

pub(crate) fn default_port() -> u16 {
    8080
}
pub(crate) fn default_host() -> String {
    "127.0.0.1".to_string()
}
pub(crate) fn default_circuit_breaker_minutes() -> u64 {
    30
}
pub(crate) fn default_max_retries() -> u32 {
    3
}
pub(crate) fn default_routing_strategy() -> crate::router::RoutingStrategyType {
    crate::router::RoutingStrategyType::WeightedRandom
}
pub(crate) fn default_health_check_interval_secs() -> u64 {
    60
}
pub(crate) fn default_health_check_enabled() -> bool {
    true
}
pub(crate) fn default_drain_timeout_secs() -> u64 {
    30
}
pub(crate) fn default_cache_ttl_secs() -> u64 {
    300
}
pub(crate) fn default_max_cache_entries() -> usize {
    1000
}
pub(crate) fn default_cache_mode() -> String {
    "on".to_string()
}
pub(crate) fn default_http_timeout_secs() -> u64 {
    300
}
pub(crate) fn default_affinity_ttl_secs() -> u64 {
    1800
}
pub(crate) fn default_log_max_entries() -> usize {
    1000
}
pub(crate) fn default_quota_poll_interval_secs() -> u64 {
    60
}
pub(crate) fn default_mcp_max_iterations() -> u32 {
    5
}
pub(crate) fn default_mcp_auto_inject() -> bool {
    true
}
pub(crate) fn default_mcp_gateway_enabled() -> bool {
    true
}
pub(crate) fn default_stream_ttft_timeout_secs() -> Option<u64> {
    Some(30)
}
pub(crate) fn default_http_pool_size() -> usize {
    8
}
pub(crate) fn default_retry_base_ms() -> u64 {
    100
}
pub(crate) fn default_retry_max_ms() -> u64 {
    5000
}
pub(crate) fn default_log_max_file_size_mb() -> u64 {
    100
}
pub(crate) fn default_log_max_files() -> usize {
    5
}

impl GatewayConfig {
    /// Resolve a model name through groups: if the name matches a group,
    /// return the group's model list; otherwise return single-element vec.
    pub fn resolve_model_group(&self, model: &str) -> Vec<String> {
        if let Some(group) = self.model_groups.get(model) {
            return group.clone();
        }
        vec![model.to_string()]
    }

    /// Returns the effective passthrough headers: configured list if non-empty, otherwise built-in defaults.
    pub fn effective_passthrough_headers(&self) -> Vec<String> {
        if self.passthrough_headers.is_empty() {
            vec![
                "x-ratelimit-remaining".into(),
                "x-ratelimit-limit".into(),
                "x-ratelimit-reset".into(),
                "x-ratelimit-limit-requests".into(),
                "x-ratelimit-remaining-requests".into(),
                "x-ratelimit-reset-requests".into(),
                "x-ratelimit-limit-tokens".into(),
                "x-ratelimit-remaining-tokens".into(),
                "x-ratelimit-reset-tokens".into(),
                "anthropic-ratelimit-requests-limit".into(),
                "anthropic-ratelimit-requests-remaining".into(),
                "anthropic-ratelimit-requests-reset".into(),
                "anthropic-ratelimit-tokens-limit".into(),
                "anthropic-ratelimit-tokens-remaining".into(),
                "anthropic-ratelimit-tokens-reset".into(),
                "x-request-id".into(),
            ]
        } else {
            self.passthrough_headers.clone()
        }
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            circuit_breaker_minutes: default_circuit_breaker_minutes(),
            max_retries: default_max_retries(),
            model_fallbacks: HashMap::new(),
            context_window_fallbacks: HashMap::new(),
            model_aliases: HashMap::new(),
            routing_strategy: default_routing_strategy(),
            health_check_interval_secs: default_health_check_interval_secs(),
            health_check_enabled: default_health_check_enabled(),
            admin_token: None,
            admin_tokens: vec![],
            web_console_dir: None,
            drain_timeout_secs: default_drain_timeout_secs(),
            request_timeout_secs: None,
            stream_keepalive_secs: None,
            cache_ttl_secs: default_cache_ttl_secs(),
            max_cache_entries: default_max_cache_entries(),
            cache_mode: default_cache_mode(),
            http_timeout_secs: default_http_timeout_secs(),
            affinity_ttl_secs: default_affinity_ttl_secs(),
            log_max_entries: default_log_max_entries(),
            quota_poll_interval_secs: default_quota_poll_interval_secs(),
            mcp_max_iterations: default_mcp_max_iterations(),
            mcp_auto_inject: default_mcp_auto_inject(),
            mcp_gateway_enabled: default_mcp_gateway_enabled(),
            stream_ttft_timeout_secs: default_stream_ttft_timeout_secs(),
            http_pool_size: default_http_pool_size(),
            sanitizer: SanitizerConfig::default(),
            provider_budgets: HashMap::new(),
            retry_base_ms: default_retry_base_ms(),
            retry_max_ms: default_retry_max_ms(),
            model_retry_overrides: HashMap::new(),
            allowed_origins: None,
            nonstream_keepalive_interval_secs: 0,
            passthrough_headers: vec![],
            log_max_file_size_mb: default_log_max_file_size_mb(),
            log_max_files: default_log_max_files(),
            stream_bootstrap_retries: 0,
            disable_cooling: false,
            disable_image_generation: false,
            model_groups: HashMap::new(),
            model_pricing: HashMap::new(),
            completion_ratios: HashMap::new(),
            group_ratios: HashMap::new(),
            tls: TlsConfig::default(),
            notification: crate::notification::NotificationConfig::default(),
            rate_limit_algorithm: crate::proxy::rate_limiter::RateLimitAlgorithm::default(),
            auth: AuthConfig::default(),
        }
    }
}
