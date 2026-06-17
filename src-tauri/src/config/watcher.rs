use crate::channel::{manager::ChannelManager, Channel};
use crate::config::AppConfig;
use crate::mcp::McpManager;
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use notify::Watcher;
use std::path::PathBuf;
use std::sync::Arc;

/// Apply a config reload: update channels, MCP servers, rate limits, and payload rules.
/// Extracted from the watcher loop for testability.
pub(crate) async fn apply_config_reload(
    new_config: &AppConfig,
    channel_mgr: &ChannelManager,
    mcp_mgr: &McpManager,
    rate_limiter: &RateLimiter,
    payload_rules: &ChannelPayloadRules,
) {
    // Update channels atomically: build the complete new
    // channel list outside any locks, then swap it in with
    // a single write-lock acquisition. This prevents dispatch
    // from seeing a partially-updated channel list.
    let config_ids: std::collections::HashSet<uuid::Uuid> = new_config
        .channels
        .iter()
        .filter_map(|cc| uuid::Uuid::parse_str(&cc.id).ok())
        .collect();

    // Snapshot current channels once (single read lock)
    let current_channels = channel_mgr.list().await;
    let existing_by_id: std::collections::HashMap<uuid::Uuid, Channel> =
        current_channels.into_iter().map(|c| (c.id, c)).collect();

    // Build the full new channel list, preserving runtime
    // state for channels that already exist.
    let mut new_channels: Vec<Channel> = Vec::new();
    for cc in &new_config.channels {
        let id = uuid::Uuid::parse_str(&cc.id).unwrap_or_else(|_| uuid::Uuid::new_v4());
        if let Some(existing) = existing_by_id.get(&id) {
            // Update mutable fields only, preserve runtime state
            let mut updated = existing.clone();
            updated.weight = cc.weight;
            updated.priority = cc.priority;
            updated.enabled = cc.enabled;
            updated.base_url = cc.base_url.clone();
            updated.model_mapping = cc.model_mapping.clone();
            updated.cost_per_token = cc.cost_per_token;
            updated.input_cost_per_mtok = cc.input_cost_per_mtok;
            updated.output_cost_per_mtok = cc.output_cost_per_mtok;
            updated.cooldown_minutes = cc.cooldown_minutes;
            updated.name = cc.name.clone();
            updated.provider = crate::channel::Provider::from_str(&cc.provider);
            new_channels.push(updated);
        } else {
            // New channel — create it
            new_channels.push(Channel::from_config(cc));
        }
    }
    // Remove payload rules and log channels being removed (not in new config)
    for ch in existing_by_id.values() {
        if !config_ids.contains(&ch.id) {
            tracing::info!(
                channel = %ch.name,
                id = %ch.id,
                "Removing channel deleted from config"
            );
            payload_rules.remove(ch.id);
        }
    }

    // Atomically swap the entire channel list
    channel_mgr.replace_all(new_channels).await;
    tracing::info!("Config reload complete");

    // Reload MCP servers: preserve running servers that still exist,
    // stop removed servers, add new servers (not auto-started).
    mcp_mgr.reload_configs(&new_config.mcp_servers).await;
    tracing::info!("MCP servers reloaded");

    // Hot-reload rate limits and payload rules from channel configs
    for cc in &new_config.channels {
        let id = uuid::Uuid::parse_str(&cc.id).unwrap_or_else(|_| uuid::Uuid::new_v4());

        if let Some(rpm) = cc.rpm_limit {
            rate_limiter.set_channel_rpm_limit(id, rpm);
        }
        if let Some(tpm) = cc.tpm_limit {
            rate_limiter.set_channel_tpm_limit(id, tpm);
        }

        if let Some(ref rules) = cc.payload_rules {
            use crate::proxy::payload_rules::PayloadRules;
            payload_rules.add(
                id,
                PayloadRules {
                    defaults: rules.defaults.clone(),
                    overrides: rules.overrides.clone(),
                    strip: rules.strip.clone(),
                },
            );
        }
    }
    tracing::info!("Rate limits and payload rules reloaded");
}

/// Watch the config file for changes and reload channels and MCP servers when modified.
pub fn start_config_watcher(
    config_path: PathBuf,
    channel_mgr: Arc<ChannelManager>,
    mcp_mgr: Arc<McpManager>,
    rate_limiter: Arc<RateLimiter>,
    payload_rules: Arc<ChannelPayloadRules>,
) {
    crate::spawn_bg(async move {
        let mut notify = match notify::recommended_watcher(
            |res: Result<notify::Event, _>| match res {
                Ok(event) if event.kind.is_modify() || event.kind.is_create() => {
                    tracing::info!(?event.paths, "Config file change detected");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Config watch error");
                }
                _ => {}
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create config watcher, hot reload disabled");
                return;
            }
        };

        if let Some(parent) = config_path.parent() {
            if let Err(e) = notify.watch(parent, notify::RecursiveMode::NonRecursive) {
                tracing::warn!(error = %e, "Failed to watch config directory");
                return;
            }
        }

        let mut last_mod = std::fs::metadata(&config_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            // Check if the file was modified since we last checked
            if let Ok(metadata) = std::fs::metadata(&config_path) {
                if let Ok(modified) = metadata.modified() {
                    if modified > last_mod {
                        // Reload config
                        match AppConfig::load_from(config_path.clone()) {
                            Ok(new_config) => {
                                tracing::info!("Reloading config");
                                apply_config_reload(
                                    &new_config,
                                    &channel_mgr,
                                    &mcp_mgr,
                                    &rate_limiter,
                                    &payload_rules,
                                )
                                .await;
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to reload config");
                            }
                        }
                        last_mod = modified;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelStatus;
    use crate::config::{ChannelConfig, GatewayConfig};
    use crate::credential::file_store::FileCredentialStore;
    use crate::credential::SharedCredentialStore;
    use std::collections::HashMap;

    // -- Test helpers ---------------------------------------------------------

    fn make_credential_store() -> SharedCredentialStore {
        Arc::new(FileCredentialStore::new())
    }

    fn make_channel_config(id: &str, name: &str, base_url: &str, priority: u8) -> ChannelConfig {
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
            api_key: Some("sk-test".to_string()),
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
        }
    }

    fn make_app_config(channels: Vec<ChannelConfig>) -> AppConfig {
        AppConfig {
            gateway: GatewayConfig::default(),
            channels,
            mcp_servers: vec![],
        }
    }

    fn make_manager(config: &AppConfig) -> ChannelManager {
        ChannelManager::new(config, make_credential_store())
    }

    // -- TOML parsing tests ---------------------------------------------------

    #[test]
    fn toml_parsing_produces_correct_channel_configs() {
        let toml = r#"
[gateway]
port = 9090
circuit_breaker_minutes = 10

[[channels]]
id = "00000000-0000-0000-0000-000000000001"
name = "primary"
provider = "openai"
priority = 1
weight = 200
credential_type = "api_key"
credential_ref = "test-key"
api_key = "sk-abc123"
base_url = "https://api.openai.com/v1"
enabled = true

[channels.model_mapping]
"gpt-4" = "gpt-4-turbo"

[[channels]]
id = "00000000-0000-0000-0000-000000000002"
name = "fallback"
provider = "anthropic"
priority = 2
weight = 50
credential_type = "api_key"
credential_ref = "unused"
api_key = "sk-anthropic"
base_url = "https://api.anthropic.com"
enabled = false
"#;

        let config: AppConfig = toml::from_str(toml).expect("TOML should parse");

        assert_eq!(config.gateway.port, 9090);
        assert_eq!(config.gateway.circuit_breaker_minutes, 10);
        assert_eq!(config.channels.len(), 2);

        let ch1 = &config.channels[0];
        assert_eq!(ch1.name, "primary");
        assert_eq!(ch1.provider, "openai");
        assert_eq!(ch1.weight, 200);
        assert_eq!(ch1.api_key.as_deref(), Some("sk-abc123"));
        assert_eq!(
            ch1.model_mapping.get("gpt-4").map(|s| s.as_str()),
            Some("gpt-4-turbo")
        );

        let ch2 = &config.channels[1];
        assert_eq!(ch2.provider, "anthropic");
        assert!(!ch2.enabled);
    }

    #[test]
    fn toml_parsing_with_rate_limits() {
        let toml = r#"
[gateway]

[[channels]]
id = "00000000-0000-0000-0000-000000000001"
name = "rate-limited"
provider = "openai"
credential_type = "api_key"
credential_ref = "x"
base_url = "https://api.openai.com/v1"
enabled = true
rpm_limit = 30
tpm_limit = 10000
"#;

        let config: AppConfig = toml::from_str(toml).expect("TOML should parse");
        assert_eq!(config.channels[0].rpm_limit, Some(30));
        assert_eq!(config.channels[0].tpm_limit, Some(10000));
    }

    // -- Reload logic tests ---------------------------------------------------

    #[tokio::test]
    async fn reload_removes_channels_not_in_new_config() {
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Start with two channels
        let old_config = make_app_config(vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        assert_eq!(mgr.list().await.len(), 2);

        // Reload with only id1
        let new_config =
            make_app_config(vec![make_channel_config(id1, "alpha", "https://a.com", 1)]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let channels = mgr.list().await;
        assert_eq!(channels.len(), 1, "removed channel should be gone");
        assert_eq!(channels[0].name, "alpha");
    }

    #[tokio::test]
    async fn reload_preserves_runtime_state_for_existing_channels() {
        let id = "00000000-0000-0000-0000-000000000001";
        let uuid = uuid::Uuid::parse_str(id).unwrap();

        // Seed the manager with a channel that has runtime state
        let config = make_app_config(vec![make_channel_config(
            id,
            "original",
            "https://old.com",
            1,
        )]);
        let mgr = make_manager(&config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Simulate accumulated runtime state
        mgr.mark_circuit_open(uuid).await;
        let mut ch = mgr.get(uuid).await.unwrap();
        ch.avg_latency_ms = 250;
        ch.consecutive_failures = 3;
        mgr.update(uuid, ch).await;

        // Reload with updated config fields but same ID
        let new_config = make_app_config(vec![make_channel_config(
            id,
            "updated-name",
            "https://new-url.com",
            5,
        )]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let ch = mgr.get(uuid).await.unwrap();
        // Config fields updated
        assert_eq!(ch.name, "updated-name");
        assert_eq!(ch.base_url, "https://new-url.com");
        assert_eq!(ch.priority, 5);

        // Runtime state preserved
        assert_eq!(
            ch.status,
            ChannelStatus::CircuitOpen,
            "circuit breaker status should be preserved across reload"
        );
        assert_eq!(
            ch.consecutive_failures, 3,
            "failure count should be preserved"
        );
        assert_eq!(ch.avg_latency_ms, 250, "latency should be preserved");
    }

    #[tokio::test]
    async fn reload_adds_new_channels_from_config() {
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Start with one channel
        let old_config =
            make_app_config(vec![make_channel_config(id1, "alpha", "https://a.com", 1)]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Reload with an additional channel
        let new_config = make_app_config(vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let channels = mgr.list().await;
        assert_eq!(channels.len(), 2);

        let beta = channels
            .iter()
            .find(|c| c.name == "beta")
            .expect("new channel should exist");
        assert_eq!(
            beta.status,
            ChannelStatus::Healthy,
            "new channels should start with fresh Healthy status"
        );
        assert_eq!(beta.consecutive_failures, 0);
        assert_eq!(beta.avg_latency_ms, 0);
    }

    #[tokio::test]
    async fn reload_updates_mutable_fields_for_existing_channels() {
        let id = "00000000-0000-0000-0000-000000000001";

        let old_config = make_app_config(vec![make_channel_config(
            id,
            "original",
            "https://old.com",
            1,
        )]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Build a new config with changed fields
        let mut new_cc = make_channel_config(id, "renamed", "https://new.com", 3);
        new_cc.weight = 500;
        new_cc.enabled = false;
        let new_config = make_app_config(vec![new_cc]);

        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let uuid = uuid::Uuid::parse_str(id).unwrap();
        let ch = mgr.get(uuid).await.unwrap();
        assert_eq!(ch.name, "renamed");
        assert_eq!(ch.priority, 3);
        assert_eq!(ch.weight, 500);
        assert!(!ch.enabled);
        assert_eq!(ch.base_url, "https://new.com");
    }

    #[tokio::test]
    async fn reload_removes_payload_rules_for_deleted_channels() {
        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";
        let uuid2 = uuid::Uuid::parse_str(id2).unwrap();

        let old_config = make_app_config(vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Add payload rules for both channels
        use crate::proxy::payload_rules::PayloadRules;
        rules.add(
            uuid2,
            PayloadRules {
                defaults: HashMap::new(),
                overrides: HashMap::new(),
                strip: vec!["temperature".to_string()],
            },
        );
        assert!(rules.get(uuid2).is_some());

        // Reload with only id1 — id2 is removed
        let new_config =
            make_app_config(vec![make_channel_config(id1, "alpha", "https://a.com", 1)]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        assert!(
            rules.get(uuid2).is_none(),
            "payload rules for deleted channel should be removed"
        );
    }

    #[tokio::test]
    async fn reload_applies_rate_limits_from_config() {
        let id = "00000000-0000-0000-0000-000000000001";
        let uuid = uuid::Uuid::parse_str(id).unwrap();

        let old_config =
            make_app_config(vec![make_channel_config(id, "alpha", "https://a.com", 1)]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // New config with rate limits
        let mut new_cc = make_channel_config(id, "alpha", "https://a.com", 1);
        new_cc.rpm_limit = Some(20);
        new_cc.tpm_limit = Some(5000);
        let new_config = make_app_config(vec![new_cc]);

        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        assert_eq!(limiter.tpm_limit(uuid), Some(5000));
    }

    #[tokio::test]
    async fn reload_handles_empty_channel_list() {
        let id = "00000000-0000-0000-0000-000000000001";

        let old_config =
            make_app_config(vec![make_channel_config(id, "alpha", "https://a.com", 1)]);
        let mgr = make_manager(&old_config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Reload with empty config — all channels removed
        let new_config = make_app_config(vec![]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        assert!(
            mgr.list().await.is_empty(),
            "all channels should be removed with empty config"
        );
    }

    #[tokio::test]
    async fn reload_reloads_mcp_servers() {
        use crate::config::McpServerConfig;

        let mgr = make_manager(&make_app_config(vec![]));
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        let new_config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![McpServerConfig {
                id: "test-server".to_string(),
                name: "Test".to_string(),
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                enabled: true,
                expose_tools: true,
            }],
        };

        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let status = mcp.list_status().await;
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].0, "test-server");
    }

    #[tokio::test]
    async fn reload_preserves_half_open_status() {
        let id = "00000000-0000-0000-0000-000000000001";
        let uuid = uuid::Uuid::parse_str(id).unwrap();

        let config = make_app_config(vec![make_channel_config(id, "test", "https://a.com", 1)]);
        let mgr = make_manager(&config);
        let mcp = McpManager::new();
        let limiter = RateLimiter::new(None);
        let rules = ChannelPayloadRules::new();

        // Set channel to HalfOpen with some runtime state
        let mut ch = mgr.get(uuid).await.unwrap();
        ch.status = ChannelStatus::HalfOpen;
        ch.avg_latency_ms = 180;
        mgr.update(uuid, ch).await;

        // Reload — state should be preserved
        let new_config = make_app_config(vec![make_channel_config(
            id,
            "test-updated",
            "https://a.com",
            1,
        )]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let ch = mgr.get(uuid).await.unwrap();
        assert_eq!(ch.status, ChannelStatus::HalfOpen);
        assert_eq!(ch.avg_latency_ms, 180);
        assert_eq!(ch.name, "test-updated");
    }
}
