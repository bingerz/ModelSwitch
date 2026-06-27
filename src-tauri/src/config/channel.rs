use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

// ── Default value functions ───────────────────────────

pub(crate) fn default_priority() -> u8 {
    1
}
pub(crate) fn default_weight() -> u32 {
    100
}
