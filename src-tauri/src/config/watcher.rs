use crate::channel::{manager::ChannelManager, Channel, CredentialType};
use crate::config::{AppConfig, ChannelConfig};
use crate::mcp::McpManager;
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use notify::Watcher;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Compute which channels were added, removed, or changed.
#[derive(Debug)]
struct ChannelDiff {
    added: Vec<ChannelConfig>,
    removed: Vec<String>, // channel IDs
    updated: Vec<ChannelConfig>,
}

/// Compute the diff between old and new channel configs.
fn diff_channels(old: &[ChannelConfig], new: &[ChannelConfig]) -> ChannelDiff {
    let old_map: HashMap<&str, &ChannelConfig> = old.iter().map(|c| (c.id.as_str(), c)).collect();
    let new_map: HashMap<&str, &ChannelConfig> = new.iter().map(|c| (c.id.as_str(), c)).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut updated = Vec::new();

    for c in new {
        match old_map.get(c.id.as_str()) {
            None => added.push(c.clone()),
            Some(old_c) if config_changed(old_c, c) => updated.push(c.clone()),
            _ => {} // unchanged
        }
    }

    for c in old {
        if !new_map.contains_key(c.id.as_str()) {
            removed.push(c.id.clone());
        }
    }

    ChannelDiff {
        added,
        removed,
        updated,
    }
}

/// Check if a channel's config has meaningfully changed.
/// Compares fields that affect routing behavior.
fn config_changed(old: &ChannelConfig, new: &ChannelConfig) -> bool {
    old.name != new.name
        || old.provider != new.provider
        || old.priority != new.priority
        || old.weight != new.weight
        || old.enabled != new.enabled
        || old.base_url != new.base_url
        || old.model_mapping != new.model_mapping
        || old.api_keys != new.api_keys
        || old.excluded_models != new.excluded_models
        || old.rpm_limit != new.rpm_limit
        || old.tpm_limit != new.tpm_limit
        || old.proxy_url != new.proxy_url
        || old.credential_ref != new.credential_ref
        || old.api_key != new.api_key
        || old.max_concurrent != new.max_concurrent
        || old.headers != new.headers
        || old.max_retries != new.max_retries
        || old.models_endpoint != new.models_endpoint
        || old.models_refresh_interval_secs != new.models_refresh_interval_secs
}

/// Convert a runtime `Channel` back into a `ChannelConfig` for diffing.
/// Only fields used by `config_changed` need to be accurate; runtime-only
/// fields (payload_rules, quota) are set to `None`.
fn channel_to_config(ch: &Channel) -> ChannelConfig {
    ChannelConfig {
        id: ch.id.to_string(),
        name: ch.name.clone(),
        provider: ch.provider.as_str().to_string(),
        priority: ch.priority,
        weight: ch.weight,
        cost_per_token: ch.cost_per_token,
        input_cost_per_mtok: ch.input_cost_per_mtok,
        output_cost_per_mtok: ch.output_cost_per_mtok,
        credential_type: match ch.credential.cred_type {
            CredentialType::ApiKey => "api_key".to_string(),
            CredentialType::WebSession => "web_session".to_string(),
        },
        credential_ref: ch.credential.key_ref.clone(),
        api_key: ch.credential.api_key.clone(),
        base_url: ch.base_url.clone(),
        enabled: ch.enabled,
        model_mapping: ch.model_mapping.clone(),
        cooldown_minutes: ch.cooldown_minutes,
        rpm_limit: ch.rpm_limit,
        tpm_limit: ch.tpm_limit,
        payload_rules: None,
        quota: None,
        account_group: ch.account_group.clone(),
        max_concurrent: ch.max_concurrent,
        api_keys: ch.api_keys.clone(),
        excluded_models: ch.excluded_models.clone(),
        proxy_url: ch.proxy_url.clone(),
        headers: if ch.headers.is_empty() {
            None
        } else {
            Some(ch.headers.clone())
        },
        max_retries: ch.max_retries,
        models_endpoint: ch.models_endpoint.clone(),
        models_refresh_interval_secs: ch.models_refresh_interval_secs,
        tags: ch.tags.clone(),
    }
}

/// Apply a config reload: update channels, MCP servers, rate limits, and payload rules.
/// Uses diff-based updates so unchanged channels preserve their runtime state
/// (latency stats, circuit breaker state, etc.).
/// Extracted from the watcher loop for testability.
pub(crate) async fn apply_config_reload(
    new_config: &AppConfig,
    channel_mgr: &ChannelManager,
    mcp_mgr: &McpManager,
    rate_limiter: &RateLimiter,
    payload_rules: &ChannelPayloadRules,
) {
    // Snapshot current channels and convert to configs for diffing
    let current_channels = channel_mgr.list().await;
    let old_configs: Vec<ChannelConfig> = current_channels.iter().map(channel_to_config).collect();

    // Compute diff — only touched channels will be modified
    let diff = diff_channels(&old_configs, &new_config.channels);

    // Added channels: create with fresh runtime state
    for cc in &diff.added {
        let ch = Channel::from_config(cc);
        channel_mgr.create(ch).await;
    }

    // Removed channels: delete and clean up payload rules
    for id_str in &diff.removed {
        if let Ok(uuid) = uuid::Uuid::parse_str(id_str) {
            tracing::info!(channel_id = %id_str, "Removing channel deleted from config");
            channel_mgr.delete(uuid).await;
            payload_rules.remove(uuid);
        }
    }

    // Updated channels: replace config (resets runtime state for that channel)
    for cc in &diff.updated {
        let ch = Channel::from_config(cc);
        let uuid = ch.id;
        channel_mgr.update(uuid, ch).await;
    }

    // Unchanged channels: do nothing — runtime state is preserved

    tracing::info!(
        added = diff.added.len(),
        removed = diff.removed.len(),
        updated = diff.updated.len(),
        "Config reloaded"
    );

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
            if !rules.model_rules.is_empty() {
                payload_rules.set_model_rules(id, rules.model_rules.clone());
            }
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
            excluded_models: vec![],
            proxy_url: None,
            headers: None,
            max_retries: None,
            models_endpoint: None,
            models_refresh_interval_secs: 0,
            tags: vec![],
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
    async fn reload_preserves_runtime_state_for_unchanged_channels() {
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

        // Reload with identical config — channel is unchanged
        let new_config = make_app_config(vec![make_channel_config(
            id,
            "original",
            "https://old.com",
            1,
        )]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let ch = mgr.get(uuid).await.unwrap();

        // Runtime state preserved because config was unchanged
        assert_eq!(
            ch.status,
            ChannelStatus::CircuitOpen,
            "circuit breaker status should be preserved for unchanged channels"
        );
        assert_eq!(
            ch.consecutive_failures, 3,
            "failure count should be preserved for unchanged channels"
        );
        assert_eq!(
            ch.avg_latency_ms, 250,
            "latency should be preserved for unchanged channels"
        );
    }

    #[tokio::test]
    async fn reload_resets_runtime_state_for_updated_channels() {
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

        // Reload with changed config — channel is updated, runtime state resets
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

        // Runtime state reset because config changed
        assert_eq!(
            ch.status,
            ChannelStatus::Healthy,
            "circuit breaker status should reset for updated channels"
        );
        assert_eq!(
            ch.consecutive_failures, 0,
            "failure count should reset for updated channels"
        );
        assert_eq!(
            ch.avg_latency_ms, 0,
            "latency should reset for updated channels"
        );
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
    async fn reload_preserves_half_open_status_for_unchanged_channels() {
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

        // Reload with identical config — state should be preserved
        let new_config = make_app_config(vec![make_channel_config(id, "test", "https://a.com", 1)]);
        apply_config_reload(&new_config, &mgr, &mcp, &limiter, &rules).await;

        let ch = mgr.get(uuid).await.unwrap();
        assert_eq!(ch.status, ChannelStatus::HalfOpen);
        assert_eq!(ch.avg_latency_ms, 180);
        assert_eq!(ch.name, "test");
    }

    // -- Diff function unit tests --------------------------------------------

    #[test]
    fn diff_channels_detects_added() {
        let id1 = "chan-1";
        let id2 = "chan-2";
        let old = vec![make_channel_config(id1, "alpha", "https://a.com", 1)];
        let new = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ];

        let diff = diff_channels(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].id, id2);
        assert!(diff.removed.is_empty());
        assert!(diff.updated.is_empty());
    }

    #[test]
    fn diff_channels_detects_removed() {
        let id1 = "chan-1";
        let id2 = "chan-2";
        let old = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ];
        let new = vec![make_channel_config(id1, "alpha", "https://a.com", 1)];

        let diff = diff_channels(&old, &new);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], id2);
        assert!(diff.updated.is_empty());
    }

    #[test]
    fn diff_channels_detects_updated() {
        let id = "chan-1";
        let old = vec![make_channel_config(id, "alpha", "https://a.com", 1)];
        let new = vec![make_channel_config(id, "alpha-renamed", "https://a.com", 1)];

        let diff = diff_channels(&old, &new);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.updated.len(), 1);
        assert_eq!(diff.updated[0].name, "alpha-renamed");
    }

    #[test]
    fn diff_channels_detects_unchanged() {
        let id1 = "chan-1";
        let id2 = "chan-2";
        let old = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ];
        // Same configs — no changes
        let new = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
        ];

        let diff = diff_channels(&old, &new);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.updated.is_empty());
    }

    #[test]
    fn diff_channels_handles_empty_old() {
        let new = vec![make_channel_config("chan-1", "alpha", "https://a.com", 1)];

        let diff = diff_channels(&[], &new);
        assert_eq!(diff.added.len(), 1);
        assert!(diff.removed.is_empty());
        assert!(diff.updated.is_empty());
    }

    #[test]
    fn diff_channels_handles_empty_new() {
        let old = vec![make_channel_config("chan-1", "alpha", "https://a.com", 1)];

        let diff = diff_channels(&old, &[]);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert!(diff.updated.is_empty());
    }

    #[test]
    fn diff_channels_detects_mixed_changes() {
        let id1 = "chan-1"; // unchanged
        let id2 = "chan-2"; // updated
        let id3 = "chan-3"; // removed
        let id4 = "chan-4"; // added

        let old = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1),
            make_channel_config(id2, "beta", "https://b.com", 2),
            make_channel_config(id3, "gamma", "https://c.com", 3),
        ];
        let new = vec![
            make_channel_config(id1, "alpha", "https://a.com", 1), // unchanged
            make_channel_config(id2, "beta-v2", "https://b.com", 2), // name changed
            make_channel_config(id4, "delta", "https://d.com", 4), // new
        ];

        let diff = diff_channels(&old, &new);
        assert_eq!(diff.added.len(), 1, "one channel added");
        assert_eq!(diff.added[0].id, id4);
        assert_eq!(diff.removed.len(), 1, "one channel removed");
        assert_eq!(diff.removed[0], id3);
        assert_eq!(diff.updated.len(), 1, "one channel updated");
        assert_eq!(diff.updated[0].id, id2);
    }

    #[test]
    fn config_changed_returns_false_for_identical_configs() {
        let cc = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        assert!(!config_changed(&cc, &cc));
    }

    #[test]
    fn config_changed_detects_name_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.name = "beta".to_string();
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_priority_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.priority = 5;
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_weight_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.weight = 200;
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_enabled_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.enabled = false;
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_base_url_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.base_url = "https://b.com".to_string();
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_api_key_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.api_key = Some("sk-different".to_string());
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_rpm_limit_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.rpm_limit = Some(60);
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_proxy_url_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.proxy_url = Some("socks5://proxy:1080".to_string());
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_model_mapping_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.model_mapping
            .insert("gpt-4".to_string(), "gpt-4-turbo".to_string());
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_api_keys_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.api_keys.push("sk-extra".to_string());
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_excluded_models_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.excluded_models.push("*-preview".to_string());
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_max_concurrent_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.max_concurrent = Some(10);
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_credential_ref_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.credential_ref = "different-key".to_string();
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_provider_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.provider = "anthropic".to_string();
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_detects_tpm_limit_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.tpm_limit = Some(10000);
        assert!(config_changed(&old, &new));
    }

    #[test]
    fn config_changed_ignores_payload_rules_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.payload_rules = Some(crate::config::PayloadRulesConfig {
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec!["temperature".to_string()],
            model_rules: vec![],
        });
        // payload_rules is not compared by config_changed
        assert!(!config_changed(&old, &new));
    }

    #[test]
    fn config_changed_ignores_quota_change() {
        let old = make_channel_config("chan-1", "alpha", "https://a.com", 1);
        let mut new = old.clone();
        new.quota = Some(crate::config::QuotaConfig {
            strategy: Some("http_api".to_string()),
            balance_url: None,
            balance_path: None,
            limit_path: None,
            usage_path: None,
            auth_prefix: None,
            refresh_secs: None,
        });
        // quota is not compared by config_changed
        assert!(!config_changed(&old, &new));
    }
}
