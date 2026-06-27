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
    /// Explicitly allow open-proxy mode (no virtual key auth) on non-loopback binds.
    /// Defaults to false — the gateway refuses to start in open-proxy mode on
    /// non-loopback addresses unless this is set to true.
    #[serde(default)]
    pub allow_open_proxy: bool,
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
            allow_open_proxy: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default value tests ───────────────────────────────

    #[test]
    fn gateway_config_default_has_correct_port() {
        assert_eq!(GatewayConfig::default().port, 8080);
    }

    #[test]
    fn gateway_config_default_has_correct_host() {
        assert_eq!(GatewayConfig::default().host, "127.0.0.1");
    }

    #[test]
    fn gateway_config_default_circuit_breaker_minutes() {
        assert_eq!(GatewayConfig::default().circuit_breaker_minutes, 30);
    }

    #[test]
    fn gateway_config_default_max_retries() {
        assert_eq!(GatewayConfig::default().max_retries, 3);
    }

    #[test]
    fn gateway_config_default_cache_ttl() {
        assert_eq!(GatewayConfig::default().cache_ttl_secs, 300);
    }

    #[test]
    fn gateway_config_default_cache_mode() {
        assert_eq!(GatewayConfig::default().cache_mode, "on");
    }

    #[test]
    fn gateway_config_default_http_timeout() {
        assert_eq!(GatewayConfig::default().http_timeout_secs, 300);
    }

    #[test]
    fn gateway_config_default_http_pool_size() {
        assert_eq!(GatewayConfig::default().http_pool_size, 8);
    }

    #[test]
    fn gateway_config_default_log_max_entries() {
        assert_eq!(GatewayConfig::default().log_max_entries, 1000);
    }

    #[test]
    fn gateway_config_default_mcp_settings() {
        let c = GatewayConfig::default();
        assert_eq!(c.mcp_max_iterations, 5);
        assert!(c.mcp_auto_inject);
        assert!(c.mcp_gateway_enabled);
    }

    #[test]
    fn gateway_config_default_stream_ttft_timeout() {
        assert_eq!(GatewayConfig::default().stream_ttft_timeout_secs, Some(30));
    }

    #[test]
    fn gateway_config_default_retry_settings() {
        let c = GatewayConfig::default();
        assert_eq!(c.retry_base_ms, 100);
        assert_eq!(c.retry_max_ms, 5000);
    }

    #[test]
    fn gateway_config_default_log_rotation() {
        let c = GatewayConfig::default();
        assert_eq!(c.log_max_file_size_mb, 100);
        assert_eq!(c.log_max_files, 5);
    }

    #[test]
    fn gateway_config_default_quota_poll_interval() {
        assert_eq!(GatewayConfig::default().quota_poll_interval_secs, 60);
    }

    #[test]
    fn gateway_config_default_drain_timeout() {
        assert_eq!(GatewayConfig::default().drain_timeout_secs, 30);
    }

    #[test]
    fn gateway_config_default_affinity_ttl() {
        assert_eq!(GatewayConfig::default().affinity_ttl_secs, 1800);
    }

    // ── resolve_model_group tests ─────────────────────────

    #[test]
    fn resolve_model_group_returns_single_for_unknown() {
        let c = GatewayConfig::default();
        assert_eq!(c.resolve_model_group("gpt-4"), vec!["gpt-4"]);
    }

    #[test]
    fn resolve_model_group_returns_group_members() {
        let mut c = GatewayConfig::default();
        c.model_groups.insert(
            "reasoning".to_string(),
            vec!["o1".to_string(), "o3".to_string()],
        );
        assert_eq!(c.resolve_model_group("reasoning"), vec!["o1", "o3"]);
    }

    #[test]
    fn resolve_model_group_empty_group_returns_empty() {
        let mut c = GatewayConfig::default();
        c.model_groups.insert("empty".to_string(), vec![]);
        assert!(c.resolve_model_group("empty").is_empty());
    }

    // ── effective_passthrough_headers tests ───────────────

    #[test]
    fn effective_passthrough_headers_defaults_when_empty() {
        let c = GatewayConfig::default();
        let headers = c.effective_passthrough_headers();
        assert!(headers.contains(&"x-request-id".to_string()));
        assert!(headers.contains(&"x-ratelimit-remaining".to_string()));
        assert!(!headers.is_empty());
    }

    #[test]
    fn effective_passthrough_headers_uses_configured_when_set() {
        let c = GatewayConfig {
            passthrough_headers: vec!["x-custom".to_string()],
            ..Default::default()
        };
        let headers = c.effective_passthrough_headers();
        assert_eq!(headers, vec!["x-custom"]);
        // Should NOT contain defaults when custom list is set
        assert!(!headers.contains(&"x-request-id".to_string()));
    }

    // ── Serde deserialization tests ───────────────────────

    #[test]
    fn gateway_config_deserializes_from_minimal_json() {
        let json = r#"{}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.port, 8080);
        assert_eq!(c.host, "127.0.0.1");
        assert!(c.health_check_enabled);
    }

    #[test]
    fn gateway_config_deserializes_overrides() {
        let json = r#"{"port": 9090, "host": "0.0.0.0", "max_retries": 5, "cache_mode": "off"}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.port, 9090);
        assert_eq!(c.host, "0.0.0.0");
        assert_eq!(c.max_retries, 5);
        assert_eq!(c.cache_mode, "off");
    }

    #[test]
    fn gateway_config_deserializes_model_pricing() {
        let json = r#"{"model_pricing": {"gpt-4": {"input_cost_per_mtok": 10.0, "output_cost_per_mtok": 30.0}}}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        let pricing = c.model_pricing.get("gpt-4").unwrap();
        assert_eq!(pricing.input_cost_per_mtok, Some(10.0));
        assert_eq!(pricing.output_cost_per_mtok, Some(30.0));
    }

    #[test]
    fn gateway_config_deserializes_model_groups() {
        let json = r#"{"model_groups": {"fast": ["gpt-4o-mini", "claude-3-haiku"]}}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            c.resolve_model_group("fast"),
            vec!["gpt-4o-mini", "claude-3-haiku"]
        );
    }

    #[test]
    fn gateway_config_deserializes_tls_config() {
        let json = r#"{"tls": {"enable": true, "cert": "/path/cert.pem", "key": "/path/key.pem"}}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert!(c.tls.enable);
        assert_eq!(c.tls.cert, "/path/cert.pem");
    }

    #[test]
    fn gateway_config_deserializes_completion_ratios() {
        let json = r#"{"completion_ratios": {"gpt-4": 2.0, "claude-3": 1.5}}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.completion_ratios.get("gpt-4"), Some(&2.0));
        assert_eq!(c.completion_ratios.get("claude-3"), Some(&1.5));
    }

    #[test]
    fn gateway_config_deserializes_model_aliases() {
        let json = r#"{"model_aliases": {"gpt4": "gpt-4-turbo"}}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            c.model_aliases.get("gpt4"),
            Some(&"gpt-4-turbo".to_string())
        );
    }

    #[test]
    fn gateway_config_deserializes_disabled_features() {
        let json = r#"{"disable_cooling": true, "disable_image_generation": true}"#;
        let c: GatewayConfig = serde_json::from_str(json).unwrap();
        assert!(c.disable_cooling);
        assert!(c.disable_image_generation);
    }

    // ── TlsConfig default test ────────────────────────────

    #[test]
    fn tls_config_default_is_disabled() {
        let tls = TlsConfig::default();
        assert!(!tls.enable);
        assert!(tls.cert.is_empty());
        assert!(tls.key.is_empty());
    }

    // ── ModelPricing default test ─────────────────────────

    #[test]
    fn model_pricing_default_is_none() {
        let p = ModelPricing::default();
        assert!(p.input_cost_per_mtok.is_none());
        assert!(p.output_cost_per_mtok.is_none());
    }
}
