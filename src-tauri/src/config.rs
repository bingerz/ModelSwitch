pub mod watcher;

mod auth;
mod channel;
mod gateway;
mod mcp;
mod security;

// Re-export everything for backward compatibility — every existing
// `use crate::config::Foo` continues to work unchanged.
pub use auth::*;
pub use channel::*;
pub use gateway::*;
pub use mcp::*;
pub use security::*;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Return the application config directory (`~/.config/modelswitch` on Linux, etc.).
pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modelswitch")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub gateway: GatewayConfig,
    pub channels: Vec<ChannelConfig>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
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
    use super::auth::default_oidc_scopes;
    use super::channel::{default_priority, default_weight};
    use std::collections::HashMap;

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

    // ===== AuthConfig / LdapConfig / OidcConfig =====

    #[test]
    fn auth_config_defaults_to_disabled() {
        let auth = AuthConfig::default();
        assert!(auth.ldap.is_none());
        assert!(auth.oidc.is_none());
    }

    #[test]
    fn gateway_config_includes_auth_field_by_default() {
        let gw = GatewayConfig::default();
        assert!(gw.auth.ldap.is_none());
        assert!(gw.auth.oidc.is_none());
    }

    #[test]
    fn build_bind_dn_substitutes_username() {
        let cfg = LdapConfig {
            url: "ldap://dc01.corp.local:389".into(),
            bind_dn_template: "cn={username},ou=users,dc=corp,dc=local".into(),
            starttls: false,
            default_group: "ldap".into(),
            timeout_secs: 10,
        };
        let dn = cfg.build_bind_dn("alice");
        assert_eq!(dn, "cn=alice,ou=users,dc=corp,dc=local");
    }

    #[test]
    fn build_bind_dn_upn_style() {
        let cfg = LdapConfig {
            url: "ldaps://dc01.corp.local:636".into(),
            bind_dn_template: "{username}@corp.local".into(),
            starttls: true,
            default_group: "ad".into(),
            timeout_secs: 10,
        };
        let dn = cfg.build_bind_dn("bob");
        assert_eq!(dn, "bob@corp.local");
    }

    #[test]
    fn build_bind_dn_escapes_comma_injection() {
        let cfg = LdapConfig {
            url: "ldap://localhost:389".into(),
            bind_dn_template: "cn={username},dc=corp,dc=local".into(),
            starttls: false,
            default_group: "ldap".into(),
            timeout_secs: 10,
        };
        // Attacker tries to inject a second DN component
        let dn = cfg.build_bind_dn("alice,dc=evil");
        // The comma inside the username must be escaped
        assert_eq!(dn, "cn=alice\\,dc=evil,dc=corp,dc=local");
    }

    #[test]
    fn build_bind_dn_escapes_all_special_chars() {
        let cfg = LdapConfig {
            url: "ldap://localhost:389".into(),
            bind_dn_template: "cn={username},dc=corp,dc=local".into(),
            starttls: false,
            default_group: "ldap".into(),
            timeout_secs: 10,
        };
        let dn = cfg.build_bind_dn(r#"a+b"c\d<e>f;g"#);
        assert_eq!(dn, r#"cn=a\+b\"c\\d\<e\>f\;g,dc=corp,dc=local"#);
    }

    #[test]
    fn build_bind_dn_escapes_leading_and_trailing_space() {
        let cfg = LdapConfig {
            url: "ldap://localhost:389".into(),
            bind_dn_template: "cn={username},dc=corp,dc=local".into(),
            starttls: false,
            default_group: "ldap".into(),
            timeout_secs: 10,
        };
        let dn = cfg.build_bind_dn(" alice ");
        assert_eq!(dn, r"cn=\ alice\ ,dc=corp,dc=local");
    }

    #[test]
    fn build_bind_dn_escapes_leading_hash() {
        let cfg = LdapConfig {
            url: "ldap://localhost:389".into(),
            bind_dn_template: "cn={username},dc=corp,dc=local".into(),
            starttls: false,
            default_group: "ldap".into(),
            timeout_secs: 10,
        };
        let dn = cfg.build_bind_dn("#admin");
        assert_eq!(dn, r"cn=\#admin,dc=corp,dc=local");
    }

    #[test]
    fn ldap_config_serde_roundtrip() {
        let cfg = LdapConfig {
            url: "ldap://dc01.corp.local:389".into(),
            bind_dn_template: "cn={username},ou=users,dc=corp,dc=local".into(),
            starttls: true,
            default_group: "corp".into(),
            timeout_secs: 15,
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: LdapConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.url, cfg.url);
        assert_eq!(parsed.bind_dn_template, cfg.bind_dn_template);
        assert!(parsed.starttls);
        assert_eq!(parsed.timeout_secs, 15);
    }

    #[test]
    fn oidc_config_defaults_scopes() {
        let scopes = default_oidc_scopes();
        assert_eq!(scopes, vec!["openid", "email", "profile"]);
    }
}
