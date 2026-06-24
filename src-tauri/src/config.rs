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
        fs::write(&config_path, content)?;
        Ok(())
    }

    pub fn config_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("modelswitch");
        Ok(dir.join("config.toml"))
    }
}
