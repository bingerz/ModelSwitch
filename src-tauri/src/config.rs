pub mod watcher;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Return the application config directory (`~/.config/modelswitch` on Linux, etc.).
pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modelswitch")
}

/// Per-channel payload rules configuration.
///
/// All field keys use dotted JSON path notation
/// (e.g., `"generationConfig.thinkingConfig.thinkingBudget"`).
/// Top-level keys like `"temperature"` are single-segment paths.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PayloadRulesConfig {
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub strip: Vec<String>,
    /// Optional per-model rules applied after the channel-level rules above.
    /// Each entry can match by model glob pattern and optional protocol.
    #[serde(default)]
    pub model_rules: Vec<crate::proxy::payload_rules::ModelPayloadRule>,
}

/// MCP server configuration for subprocess-based tool providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique identifier for this MCP server instance.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Command to execute (e.g., "npx", "node", "python").
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the subprocess.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Working directory for the subprocess.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Whether this server is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Whether tools from this server should be exposed to LLM clients.
    #[serde(default = "default_mcp_expose_tools")]
    pub expose_tools: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub gateway: GatewayConfig,
    pub channels: Vec<ChannelConfig>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
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
    pub admin_tokens: Vec<AdminTokenConfig>,
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
}

/// Config entry for a role-based admin token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTokenConfig {
    pub token: String,
    pub role: String,
}

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

/// Privacy guardrail configuration for the sanitizer middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizerConfig {
    /// Master switch. When false, the middleware is a no-op.
    #[serde(default = "default_sanitizer_enabled")]
    pub enabled: bool,
    /// Whether to actually redact matched secrets. When false, patterns are
    /// scanned for telemetry only without mutating the body.
    #[serde(default = "default_sanitizer_redact")]
    pub redact_secrets: bool,
    /// Scan SSE response streams for echoed secrets (default false for performance).
    #[serde(default)]
    pub scan_response: bool,
    /// User-supplied patterns in addition to the built-in catalog.
    #[serde(default)]
    pub custom_patterns: Vec<CustomPattern>,
}

impl Default for SanitizerConfig {
    fn default() -> Self {
        Self {
            enabled: default_sanitizer_enabled(),
            redact_secrets: default_sanitizer_redact(),
            scan_response: false,
            custom_patterns: vec![],
        }
    }
}

/// A user-defined sanitizer pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomPattern {
    /// Human-readable label used in logs.
    pub name: String,
    /// Regular expression source. Invalid regexes are silently skipped at compile time.
    pub pattern: String,
    /// Replacement text written in place of each match.
    #[serde(default = "default_custom_replacement")]
    pub replacement: String,
}

fn default_sanitizer_enabled() -> bool {
    true
}
fn default_sanitizer_redact() -> bool {
    true
}
fn default_custom_replacement() -> String {
    "[REDACTED]".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub cost_per_token: Option<f64>,
    #[serde(default)]
    pub input_cost_per_mtok: Option<f64>,
    #[serde(default)]
    pub output_cost_per_mtok: Option<f64>,
    pub credential_type: String,
    pub credential_ref: String,
    /// Inline API key (bypasses keyring/credential store lookup).
    #[serde(default)]
    pub api_key: Option<String>,
    pub base_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub model_mapping: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub cooldown_minutes: Option<u64>,
    #[serde(default)]
    pub rpm_limit: Option<u64>,
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    #[serde(default)]
    pub payload_rules: Option<PayloadRulesConfig>,
    #[serde(default)]
    pub quota: Option<QuotaConfig>,
    /// Optional account group tag for multi-account pool management.
    #[serde(default)]
    pub account_group: Option<String>,
    /// Maximum concurrent in-flight requests for this channel (None = no limit).
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// Additional API keys for round-robin rotation across a single channel.
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Glob patterns for models to exclude from this channel (e.g., ["*-preview", "*flash*"]).
    /// When non-empty, requests for matching models skip this channel.
    #[serde(default)]
    pub excluded_models: Vec<String>,
    /// Optional proxy URL for this channel (e.g., "socks5://host:port", "http://host:port").
    /// When set, requests to this channel's upstream use a dedicated reqwest client with this proxy.
    /// Use "direct" to explicitly bypass any global proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// Custom HTTP headers to inject into requests to this channel's upstream.
    /// Headers are set after forwarding original request headers (last-value-wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// Per-channel max retries override. If set, overrides the gateway-level max_retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Optional endpoint to fetch available models from (e.g., "https://api.openai.com/v1/models").
    /// When set, the model registry will periodically fetch and update the model list for this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_endpoint: Option<String>,
    /// Interval in seconds between model list refresh polls. Defaults to 300 (5 minutes).
    #[serde(default)]
    pub models_refresh_interval_secs: u64,
    /// User-defined tags for grouping and filtering channels.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Per-channel quota polling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaConfig {
    /// Strategy: "http_api" | "openai_compat" | "jsonpath" | "disabled"
    #[serde(default)]
    pub strategy: Option<String>,
    /// Custom billing endpoint (overrides default per-provider).
    #[serde(default)]
    pub balance_url: Option<String>,
    /// JSONPath to extract balance from response (e.g. "$.data.totalBalance").
    #[serde(default)]
    pub balance_path: Option<String>,
    /// JSONPath to extract limit from response (e.g. "$.data.hard_limit_usd").
    #[serde(default)]
    pub limit_path: Option<String>,
    /// JSONPath to extract usage from response (e.g. "$.data.total_usage").
    #[serde(default)]
    pub usage_path: Option<String>,
    /// Authorization header prefix (default: "Bearer").
    #[serde(default)]
    pub auth_prefix: Option<String>,
    /// Polling interval override in seconds.
    #[serde(default)]
    pub refresh_secs: Option<u64>,
}

fn default_port() -> u16 {
    8080
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_circuit_breaker_minutes() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    3
}
fn default_priority() -> u8 {
    1
}
fn default_weight() -> u32 {
    100
}
fn default_routing_strategy() -> crate::router::RoutingStrategyType {
    crate::router::RoutingStrategyType::WeightedRandom
}
fn default_health_check_interval_secs() -> u64 {
    60
}
fn default_health_check_enabled() -> bool {
    true
}
fn default_drain_timeout_secs() -> u64 {
    30
}
fn default_cache_ttl_secs() -> u64 {
    300
}
fn default_max_cache_entries() -> usize {
    1000
}
fn default_cache_mode() -> String {
    "on".to_string()
}
fn default_http_timeout_secs() -> u64 {
    300
}
fn default_affinity_ttl_secs() -> u64 {
    1800
}
fn default_log_max_entries() -> usize {
    1000
}
fn default_quota_poll_interval_secs() -> u64 {
    60
}
fn default_mcp_max_iterations() -> u32 {
    5
}
fn default_mcp_auto_inject() -> bool {
    true
}
fn default_mcp_gateway_enabled() -> bool {
    true
}
fn default_stream_ttft_timeout_secs() -> Option<u64> {
    Some(30)
}
fn default_http_pool_size() -> usize {
    8
}
fn default_retry_base_ms() -> u64 {
    100
}
fn default_retry_max_ms() -> u64 {
    5000
}
fn default_enabled() -> bool {
    true
}
fn default_mcp_expose_tools() -> bool {
    true
}
fn default_log_max_file_size_mb() -> u64 {
    100
}
fn default_log_max_files() -> usize {
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
            sanitizer: SanitizerConfig {
                enabled: default_sanitizer_enabled(),
                redact_secrets: default_sanitizer_redact(),
                scan_response: false,
                custom_patterns: vec![],
            },
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
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            let default_config = Self::default();
            default_config.save()?;
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load config from an explicit path (for --config flag).
    pub fn load_from(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            anyhow::bail!("Config file not found: {}", path.display());
        }
        let content = fs::read_to_string(&path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        // Write to a temp file then rename for atomicity — prevents a
        // partially written config if the process crashes mid-write.
        let tmp_path = config_path.with_extension("toml.tmp");
        fs::write(&tmp_path, &content)
            .and_then(|_| fs::rename(&tmp_path, &config_path))
            .map_err(|e| anyhow::anyhow!("Failed to save config atomically: {}", e))?;
        Ok(())
    }

    pub fn config_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("modelswitch");
        Ok(dir.join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Helper =====

    /// Build a minimal `ChannelConfig` with sensible defaults for round-trip tests.
    fn sample_channel(id: &str) -> ChannelConfig {
        ChannelConfig {
            id: id.to_string(),
            name: format!("Channel {}", id),
            provider: "openai".to_string(),
            priority: default_priority(),
            weight: default_weight(),
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential_type: "api_key".to_string(),
            credential_ref: "key-ref".to_string(),
            api_key: None,
            base_url: "https://api.openai.com".to_string(),
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

    // ===== Existing tests =====

    #[test]
    fn completion_ratios_default_empty() {
        let config = GatewayConfig::default();
        assert!(
            config.completion_ratios.is_empty(),
            "default completion_ratios should be empty"
        );
    }

    #[test]
    fn group_ratios_default_empty() {
        let config = GatewayConfig::default();
        assert!(
            config.group_ratios.is_empty(),
            "default group_ratios should be empty"
        );
    }

    #[test]
    fn completion_ratios_deserialized_from_toml() {
        let toml = r#"
            [gateway]
            [gateway.completion_ratios]
            gpt-4 = 2.0
            claude-3-opus = 1.5
        "#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml).expect("valid TOML");
        assert_eq!(parsed.gateway.completion_ratios.get("gpt-4"), Some(&2.0));
        assert_eq!(
            parsed.gateway.completion_ratios.get("claude-3-opus"),
            Some(&1.5)
        );
        // Unrelated model absent
        assert!(parsed.gateway.completion_ratios.get("unknown").is_none());
    }

    // ===== AppConfig round-trip =====

    #[test]
    fn app_config_round_trip_preserves_gateway_and_channels() {
        let mut config = AppConfig::default();
        config.gateway.port = 9090;
        config.gateway.host = "0.0.0.0".to_string();
        config.gateway.cache_mode = "off".to_string();
        config.channels = vec![sample_channel("ch-1"), sample_channel("ch-2")];

        let serialized = toml::to_string_pretty(&config).expect("serialize");
        let deserialized: AppConfig = toml::from_str(&serialized).expect("deserialize");

        assert_eq!(deserialized.gateway.port, 9090);
        assert_eq!(deserialized.gateway.host, "0.0.0.0");
        assert_eq!(deserialized.gateway.cache_mode, "off");
        assert_eq!(deserialized.channels.len(), 2);
        assert_eq!(deserialized.channels[0].id, "ch-1");
        assert_eq!(deserialized.channels[1].id, "ch-2");
    }

    #[test]
    fn app_config_invalid_toml_returns_error() {
        let bad_toml = "this is not valid toml at all\n[[[ }\n";
        let result: Result<AppConfig, toml::de::Error> = toml::from_str(bad_toml);
        assert!(result.is_err(), "invalid TOML should produce an error");
    }

    #[test]
    fn app_config_empty_channels_no_crash() {
        // Root-level keys must precede [gateway] table header in TOML.
        let toml_str = "channels = []\n\n[gateway]\n";
        let parsed: AppConfig = toml::from_str(toml_str).expect("valid TOML");
        assert!(parsed.channels.is_empty());
        assert!(parsed.mcp_servers.is_empty());
        // Defaults still applied
        assert_eq!(parsed.gateway.port, 8080);
        assert_eq!(parsed.gateway.host, "127.0.0.1");
    }

    // ===== ChannelConfig parsing =====

    #[test]
    fn channel_config_parses_all_fields_from_toml() {
        let toml_str = r#"
[[channels]]
id = "openai-1"
name = "OpenAI Primary"
provider = "openai"
priority = 3
weight = 250
cost_per_token = 0.000002
input_cost_per_mtok = 2.5
output_cost_per_mtok = 7.5
credential_type = "api_key"
credential_ref = "openai-key"
api_key = "sk-secret"
base_url = "https://api.openai.com"
enabled = true
cooldown_minutes = 10
rpm_limit = 600
tpm_limit = 200000
max_concurrent = 20
account_group = "prod"
max_retries = 5
models_refresh_interval_secs = 120
tags = ["premium", "fast"]
api_keys = ["sk-1", "sk-2"]
excluded_models = ["*-preview"]
proxy_url = "http://proxy:8080"

[channels.model_mapping]
"gpt-4" = "gpt-4-turbo"
"claude-3" = "claude-3-opus"

[channels.headers]
"x-custom-header" = "value"
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<ChannelConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let ch = &parsed.channels[0];

        assert_eq!(ch.id, "openai-1");
        assert_eq!(ch.name, "OpenAI Primary");
        assert_eq!(ch.provider, "openai");
        assert_eq!(ch.priority, 3);
        assert_eq!(ch.weight, 250);
        assert_eq!(ch.cost_per_token, Some(0.000002));
        assert_eq!(ch.input_cost_per_mtok, Some(2.5));
        assert_eq!(ch.output_cost_per_mtok, Some(7.5));
        assert_eq!(ch.credential_type, "api_key");
        assert_eq!(ch.credential_ref, "openai-key");
        assert_eq!(ch.api_key.as_deref(), Some("sk-secret"));
        assert_eq!(ch.base_url, "https://api.openai.com");
        assert!(ch.enabled);
        assert_eq!(ch.cooldown_minutes, Some(10));
        assert_eq!(ch.rpm_limit, Some(600));
        assert_eq!(ch.tpm_limit, Some(200000));
        assert_eq!(ch.max_concurrent, Some(20));
        assert_eq!(ch.account_group.as_deref(), Some("prod"));
        assert_eq!(ch.max_retries, Some(5));
        assert_eq!(ch.models_refresh_interval_secs, 120);
        assert_eq!(ch.tags, vec!["premium", "fast"]);
        assert_eq!(ch.api_keys, vec!["sk-1", "sk-2"]);
        assert_eq!(ch.excluded_models, vec!["*-preview"]);
        assert_eq!(ch.proxy_url.as_deref(), Some("http://proxy:8080"));
        assert_eq!(
            ch.model_mapping.get("gpt-4").map(|s| s.as_str()),
            Some("gpt-4-turbo")
        );
        assert_eq!(
            ch.model_mapping.get("claude-3").map(|s| s.as_str()),
            Some("claude-3-opus")
        );
        assert_eq!(
            ch.headers
                .as_ref()
                .unwrap()
                .get("x-custom-header")
                .map(|s| s.as_str()),
            Some("value")
        );
    }

    #[test]
    fn channel_config_applies_defaults_for_optional_fields() {
        let toml_str = r#"
[[channels]]
id = "min-1"
name = "Minimal"
provider = "anthropic"
credential_type = "api_key"
credential_ref = "ref"
base_url = "https://api.anthropic.com"
enabled = false
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<ChannelConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let ch = &parsed.channels[0];

        assert_eq!(ch.id, "min-1");
        assert_eq!(ch.priority, 1, "default_priority");
        assert_eq!(ch.weight, 100, "default_weight");
        assert!(ch.cost_per_token.is_none());
        assert!(ch.input_cost_per_mtok.is_none());
        assert!(ch.output_cost_per_mtok.is_none());
        assert!(ch.api_key.is_none());
        assert!(ch.model_mapping.is_empty());
        assert!(ch.cooldown_minutes.is_none());
        assert!(ch.rpm_limit.is_none());
        assert!(ch.tpm_limit.is_none());
        assert!(ch.payload_rules.is_none());
        assert!(ch.quota.is_none());
        assert!(ch.account_group.is_none());
        assert!(ch.max_concurrent.is_none());
        assert!(ch.api_keys.is_empty());
        assert!(ch.excluded_models.is_empty());
        assert!(ch.proxy_url.is_none());
        assert!(ch.headers.is_none());
        assert!(ch.max_retries.is_none());
        assert!(ch.models_endpoint.is_none());
        assert_eq!(ch.models_refresh_interval_secs, 0);
        assert!(ch.tags.is_empty());
        assert!(!ch.enabled, "enabled should honor the TOML value");
    }

    #[test]
    fn channel_config_parses_multiple_channels() {
        let toml_str = r#"
[[channels]]
id = "ch-a"
name = "Channel A"
provider = "openai"
credential_type = "api_key"
credential_ref = "ref-a"
base_url = "https://a.example.com"
enabled = true

[[channels]]
id = "ch-b"
name = "Channel B"
provider = "anthropic"
credential_type = "api_key"
credential_ref = "ref-b"
base_url = "https://b.example.com"
enabled = true

[[channels]]
id = "ch-c"
name = "Channel C"
provider = "gemini"
credential_type = "api_key"
credential_ref = "ref-c"
base_url = "https://c.example.com"
enabled = false
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<ChannelConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        assert_eq!(parsed.channels.len(), 3);
        assert_eq!(parsed.channels[0].id, "ch-a");
        assert_eq!(parsed.channels[1].provider, "anthropic");
        assert!(!parsed.channels[2].enabled);
    }

    #[test]
    fn channel_config_with_payload_rules_parses() {
        let toml_str = r#"
[[channels]]
id = "rules-1"
name = "Rules Channel"
provider = "openai"
credential_type = "api_key"
credential_ref = "ref"
base_url = "https://api.openai.com"
enabled = true

[channels.payload_rules]
strip = ["metadata", "user_agent"]
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<ChannelConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let ch = &parsed.channels[0];
        let rules = ch.payload_rules.as_ref().expect("payload_rules present");
        assert_eq!(rules.strip.len(), 2);
        assert!(rules.strip.contains(&"metadata".to_string()));
        assert!(rules.strip.contains(&"user_agent".to_string()));
    }

    #[test]
    fn channel_config_with_quota_config_parses() {
        let toml_str = r#"
[[channels]]
id = "quota-1"
name = "Quota Channel"
provider = "openai"
credential_type = "api_key"
credential_ref = "ref"
base_url = "https://api.openai.com"
enabled = true

[channels.quota]
strategy = "http_api"
balance_url = "https://api.openai.com/billing"
balance_path = "$.data.totalBalance"
limit_path = "$.data.hard_limit_usd"
usage_path = "$.data.total_usage"
auth_prefix = "Bearer"
refresh_secs = 120
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<ChannelConfig>,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let ch = &parsed.channels[0];
        let quota = ch.quota.as_ref().expect("quota present");
        assert_eq!(quota.strategy.as_deref(), Some("http_api"));
        assert_eq!(
            quota.balance_url.as_deref(),
            Some("https://api.openai.com/billing")
        );
        assert_eq!(quota.balance_path.as_deref(), Some("$.data.totalBalance"));
        assert_eq!(quota.limit_path.as_deref(), Some("$.data.hard_limit_usd"));
        assert_eq!(quota.usage_path.as_deref(), Some("$.data.total_usage"));
        assert_eq!(quota.auth_prefix.as_deref(), Some("Bearer"));
        assert_eq!(quota.refresh_secs, Some(120));
    }

    // ===== McpServerConfig parsing =====

    #[test]
    fn mcp_server_config_parses_with_all_fields() {
        let toml_str = r#"
channels = []

[gateway]

[[mcp_servers]]
id = "fs-1"
name = "Filesystem Server"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
enabled = true
expose_tools = true
cwd = "/tmp"

[mcp_servers.env]
NODE_PATH = "/usr/local/lib/node_modules"
"#;
        let parsed: AppConfig = toml::from_str(toml_str).expect("valid TOML");
        assert_eq!(parsed.mcp_servers.len(), 1);
        let srv = &parsed.mcp_servers[0];
        assert_eq!(srv.id, "fs-1");
        assert_eq!(srv.name, "Filesystem Server");
        assert_eq!(srv.command, "npx");
        assert_eq!(
            srv.args,
            vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );
        assert!(srv.enabled);
        assert!(srv.expose_tools);
        assert_eq!(srv.cwd.as_deref(), Some("/tmp"));
        assert_eq!(
            srv.env.get("NODE_PATH").map(|s| s.as_str()),
            Some("/usr/local/lib/node_modules")
        );
    }

    #[test]
    fn mcp_server_config_defaults_to_enabled_and_expose_tools() {
        let toml_str = r#"
channels = []

[gateway]

[[mcp_servers]]
id = "min-mcp"
name = "Minimal MCP"
command = "node"
"#;
        let parsed: AppConfig = toml::from_str(toml_str).expect("valid TOML");
        let srv = &parsed.mcp_servers[0];
        assert!(srv.enabled, "enabled should default to true");
        assert!(srv.expose_tools, "expose_tools should default to true");
        assert!(srv.args.is_empty());
        assert!(srv.env.is_empty());
        assert!(srv.cwd.is_none());
    }

    // ===== GatewayConfig defaults and parsing =====

    #[test]
    fn gateway_config_defaults_match_expected_values() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.circuit_breaker_minutes, 30);
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.health_check_interval_secs, 60);
        assert!(cfg.health_check_enabled);
        assert_eq!(cfg.drain_timeout_secs, 30);
        assert_eq!(cfg.cache_ttl_secs, 300);
        assert_eq!(cfg.max_cache_entries, 1000);
        assert_eq!(cfg.cache_mode, "on");
        assert_eq!(cfg.http_timeout_secs, 300);
        assert_eq!(cfg.affinity_ttl_secs, 1800);
        assert_eq!(cfg.log_max_entries, 1000);
        assert_eq!(cfg.quota_poll_interval_secs, 60);
        assert_eq!(cfg.mcp_max_iterations, 5);
        assert!(cfg.mcp_auto_inject);
        assert!(cfg.mcp_gateway_enabled);
        assert_eq!(cfg.stream_ttft_timeout_secs, Some(30));
        assert_eq!(cfg.http_pool_size, 8);
        assert_eq!(cfg.retry_base_ms, 100);
        assert_eq!(cfg.retry_max_ms, 5000);
        assert_eq!(cfg.log_max_file_size_mb, 100);
        assert_eq!(cfg.log_max_files, 5);
        assert!(!cfg.disable_cooling);
        assert!(!cfg.disable_image_generation);
        assert!(cfg.passthrough_headers.is_empty());
        assert!(cfg.sanitizer.enabled);
        assert!(cfg.sanitizer.redact_secrets);
        assert!(!cfg.sanitizer.scan_response);
    }

    #[test]
    fn gateway_config_with_tls_settings_parses() {
        let toml_str = r#"
[gateway.tls]
enable = true
cert = "/path/to/cert.pem"
key = "/path/to/key.pem"
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        assert!(parsed.gateway.tls.enable);
        assert_eq!(parsed.gateway.tls.cert, "/path/to/cert.pem");
        assert_eq!(parsed.gateway.tls.key, "/path/to/key.pem");
    }

    #[test]
    fn gateway_config_with_model_groups_parses() {
        let toml_str = r#"
[gateway.model_groups]
reasoning = ["o1", "claude-3-opus", "gemini-pro"]
fast = ["gpt-4o-mini", "claude-3-haiku"]
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let groups = &parsed.gateway.model_groups;
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get("reasoning").unwrap().len(), 3);
        assert!(groups.get("reasoning").unwrap().contains(&"o1".to_string()));
        assert_eq!(groups.get("fast").unwrap().len(), 2);
    }

    #[test]
    fn gateway_config_with_model_pricing_parses() {
        let toml_str = r#"
[gateway.model_pricing.gpt-4]
input_cost_per_mtok = 2.5
output_cost_per_mtok = 7.5

[gateway.model_pricing.claude-3-opus]
input_cost_per_mtok = 15.0
output_cost_per_mtok = 75.0
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let pricing = &parsed.gateway.model_pricing;
        assert_eq!(pricing.len(), 2);
        let gpt4 = pricing.get("gpt-4").unwrap();
        assert_eq!(gpt4.input_cost_per_mtok, Some(2.5));
        assert_eq!(gpt4.output_cost_per_mtok, Some(7.5));
        let claude = pricing.get("claude-3-opus").unwrap();
        assert_eq!(claude.input_cost_per_mtok, Some(15.0));
        assert_eq!(claude.output_cost_per_mtok, Some(75.0));
        assert!(pricing.get("unknown-model").is_none());
    }

    #[test]
    fn gateway_config_with_model_aliases_parses() {
        let toml_str = r#"
[gateway.model_aliases]
"gpt-4" = "gpt-4-turbo-preview"
"claude" = "claude-3-opus"
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let aliases = &parsed.gateway.model_aliases;
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases.get("gpt-4").unwrap(), "gpt-4-turbo-preview");
        assert_eq!(aliases.get("claude").unwrap(), "claude-3-opus");
    }

    #[test]
    fn gateway_config_with_provider_budgets_parses() {
        let toml_str = r#"
[gateway.provider_budgets.openai]
daily_budget_cents = 5000
monthly_budget_cents = 150000

[gateway.provider_budgets.anthropic]
daily_budget_cents = 3000
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let budgets = &parsed.gateway.provider_budgets;
        assert_eq!(budgets.len(), 2);
        let openai = budgets.get("openai").unwrap();
        assert_eq!(openai.daily_budget_cents, Some(5000));
        assert_eq!(openai.monthly_budget_cents, Some(150000));
        let anthropic = budgets.get("anthropic").unwrap();
        assert_eq!(anthropic.daily_budget_cents, Some(3000));
        assert!(anthropic.monthly_budget_cents.is_none());
    }

    #[test]
    fn gateway_config_with_sanitizer_custom_patterns_parses() {
        let toml_str = r#"
[[gateway.sanitizer.custom_patterns]]
name = "SSN"
pattern = "\\d{3}-\\d{2}-\\d{4}"

[[gateway.sanitizer.custom_patterns]]
name = "Credit Card"
pattern = "\\d{16}"
replacement = "[CENSORED]"
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let patterns = &parsed.gateway.sanitizer.custom_patterns;
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].name, "SSN");
        assert_eq!(patterns[0].pattern, "\\d{3}-\\d{2}-\\d{4}");
        assert_eq!(patterns[0].replacement, "[REDACTED]", "default replacement");
        assert_eq!(patterns[1].name, "Credit Card");
        assert_eq!(patterns[1].replacement, "[CENSORED]", "custom replacement");
    }

    #[test]
    fn gateway_config_with_model_retry_overrides_parses() {
        let toml_str = r#"
[gateway.model_retry_overrides."gpt-4*"]
max_retries = 10
retry_base_ms = 200
"#;
        #[derive(Deserialize)]
        struct Wrapper {
            gateway: GatewayConfig,
        }
        let parsed: Wrapper = toml::from_str(toml_str).expect("valid TOML");
        let overrides = &parsed.gateway.model_retry_overrides;
        assert_eq!(overrides.len(), 1);
        let gpt4 = overrides.get("gpt-4*").unwrap();
        assert_eq!(gpt4.max_retries, Some(10));
        assert_eq!(gpt4.retry_base_ms, Some(200));
        assert!(
            gpt4.retry_max_ms.is_none(),
            "retry_max_ms should default to None"
        );
    }

    // ===== resolve_model_group method =====

    #[test]
    fn resolve_model_group_returns_group_when_present() {
        let mut cfg = GatewayConfig::default();
        cfg.model_groups.insert(
            "reasoning".to_string(),
            vec!["o1".to_string(), "claude-3-opus".to_string()],
        );
        let resolved = cfg.resolve_model_group("reasoning");
        assert_eq!(resolved, vec!["o1", "claude-3-opus"]);
    }

    #[test]
    fn resolve_model_group_returns_single_when_absent() {
        let cfg = GatewayConfig::default();
        let resolved = cfg.resolve_model_group("gpt-4");
        assert_eq!(resolved, vec!["gpt-4"]);
    }

    // ===== effective_passthrough_headers method =====

    #[test]
    fn effective_passthrough_headers_returns_defaults_when_empty() {
        let cfg = GatewayConfig::default();
        let headers = cfg.effective_passthrough_headers();
        assert!(!headers.is_empty(), "should return built-in defaults");
        assert!(headers.contains(&"x-request-id".to_string()));
        assert!(headers.contains(&"x-ratelimit-remaining".to_string()));
        assert!(
            headers.iter().any(|h| h.contains("anthropic")),
            "should contain anthropic rate-limit headers"
        );
    }

    #[test]
    fn effective_passthrough_headers_returns_configured_when_set() {
        let mut cfg = GatewayConfig::default();
        cfg.passthrough_headers = vec!["x-custom".into(), "x-forwarded-for".into()];
        let headers = cfg.effective_passthrough_headers();
        assert_eq!(headers, vec!["x-custom", "x-forwarded-for"]);
        // Should NOT contain built-in defaults when custom list is set
        assert!(!headers.contains(&"x-request-id".to_string()));
    }

    // ===== ModelPricing =====

    #[test]
    fn model_pricing_defaults_to_none_for_missing_costs() {
        // Partial — only input cost
        let toml_str = "input_cost_per_mtok = 1.0\n";
        let parsed: ModelPricing = toml::from_str(toml_str).expect("valid TOML");
        assert_eq!(parsed.input_cost_per_mtok, Some(1.0));
        assert!(parsed.output_cost_per_mtok.is_none());

        // Empty — both default to None
        let empty: ModelPricing = toml::from_str("").expect("valid TOML");
        assert!(empty.input_cost_per_mtok.is_none());
        assert!(empty.output_cost_per_mtok.is_none());
    }

    #[test]
    fn model_pricing_round_trip_preserves_values() {
        let pricing = ModelPricing {
            input_cost_per_mtok: Some(3.5),
            output_cost_per_mtok: Some(10.0),
        };
        let serialized = toml::to_string(&pricing).expect("serialize");
        let deserialized: ModelPricing = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.input_cost_per_mtok, Some(3.5));
        assert_eq!(deserialized.output_cost_per_mtok, Some(10.0));
    }

    // ===== PayloadRulesConfig =====

    #[test]
    fn payload_rules_config_round_trip_preserves_strip() {
        let toml_str = r#"strip = ["metadata", "generationConfig.thinkingConfig.thinkingBudget"]
"#;
        let parsed: PayloadRulesConfig = toml::from_str(toml_str).expect("valid TOML");
        assert_eq!(parsed.strip.len(), 2);
        assert!(parsed.defaults.is_empty());
        assert!(parsed.overrides.is_empty());

        let serialized = toml::to_string(&parsed).expect("serialize");
        let reparsed: PayloadRulesConfig = toml::from_str(&serialized).expect("reparse");
        assert_eq!(reparsed.strip, parsed.strip);
    }

    // ===== ModelRetryConfig =====

    #[test]
    fn model_retry_config_defaults_to_none() {
        let cfg = ModelRetryConfig::default();
        assert!(cfg.max_retries.is_none());
        assert!(cfg.retry_base_ms.is_none());
        assert!(cfg.retry_max_ms.is_none());
    }

    // ===== TlsConfig =====

    #[test]
    fn tls_config_defaults_are_disabled_and_empty() {
        let tls = TlsConfig::default();
        assert!(!tls.enable);
        assert!(tls.cert.is_empty());
        assert!(tls.key.is_empty());
    }

    // ===== SanitizerConfig =====

    #[test]
    fn sanitizer_config_defaults_to_enabled_with_redaction() {
        let s = SanitizerConfig::default();
        assert!(s.enabled);
        assert!(s.redact_secrets);
        assert!(!s.scan_response);
        assert!(s.custom_patterns.is_empty());
    }
}
