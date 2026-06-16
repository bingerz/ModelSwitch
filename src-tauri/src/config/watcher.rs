use crate::channel::{manager::ChannelManager, Channel};
use crate::config::AppConfig;
use crate::mcp::McpManager;
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use notify::Watcher;
use std::path::PathBuf;
use std::sync::Arc;

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
                                    let id = uuid::Uuid::parse_str(&cc.id)
                                        .unwrap_or_else(|_| uuid::Uuid::new_v4());
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
                                        updated.provider =
                                            crate::channel::Provider::from_str(&cc.provider);
                                        new_channels.push(updated);
                                    } else {
                                        // New channel — create it
                                        new_channels.push(Channel::from_config(cc));
                                    }
                                }
                                // Log any channels being removed (not in new config)
                                for ch in existing_by_id.values() {
                                    if !config_ids.contains(&ch.id) {
                                        tracing::info!(
                                            channel = %ch.name,
                                            id = %ch.id,
                                            "Removing channel deleted from config"
                                        );
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
                                    let id = uuid::Uuid::parse_str(&cc.id)
                                        .unwrap_or_else(|_| uuid::Uuid::new_v4());

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
