use crate::channel::manager::ChannelManager;
use crate::config::AppConfig;
use notify::Watcher;
use std::path::PathBuf;
use std::sync::Arc;

/// Watch the config file for changes and reload channels when modified.
pub fn start_config_watcher(
    config_path: PathBuf,
    channel_mgr: Arc<ChannelManager>,
) {
    crate::spawn_bg(async move {
        let mut notify = match notify::recommended_watcher(|res: Result<notify::Event, _>| {
            match res {
                Ok(event) if event.kind.is_modify() || event.kind.is_create() => {
                    tracing::info!(?event.paths, "Config file change detected");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Config watch error");
                }
                _ => {}
            }
        }) {
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

        let mut last_mod = std::time::SystemTime::UNIX_EPOCH;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            // Check if the file was modified since we last checked
            if let Ok(metadata) = std::fs::metadata(&config_path) {
                if let Ok(modified) = metadata.modified() {
                    if modified > last_mod {
                        // Reload config
                        match AppConfig::load() {
                            Ok(new_config) => {
                                tracing::info!("Reloading config");
                                // Update channels: replace all channels from new config
                                // Collect config channel IDs for deletion detection
                                let config_ids: std::collections::HashSet<uuid::Uuid> = new_config.channels.iter()
                                    .filter_map(|cc| uuid::Uuid::parse_str(&cc.id).ok())
                                    .collect();

                                for cc in &new_config.channels {
                                    let id = uuid::Uuid::parse_str(&cc.id)
                                        .unwrap_or_else(|_| uuid::Uuid::new_v4());
                                    if let Some(existing) = channel_mgr.get(id).await {
                                        // Update mutable fields only, preserve runtime state
                                        let mut updated = existing;
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
                                        let _ = channel_mgr.update(id, updated).await;
                                    } else {
                                        // New channel — create it
                                        use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
                                        let cred_type = match cc.credential_type.as_str() {
                                            "web_session" => CredentialType::WebSession,
                                            _ => CredentialType::ApiKey,
                                        };
                                        let new_channel = Channel {
                                            id,
                                            name: cc.name.clone(),
                                            provider: Provider::from_str(&cc.provider),
                                            priority: cc.priority,
                                            weight: cc.weight,
                                            cost_per_token: cc.cost_per_token,
                                            input_cost_per_mtok: cc.input_cost_per_mtok,
                                            output_cost_per_mtok: cc.output_cost_per_mtok,
                                            credential: Credential {
                                                cred_type,
                                                key_ref: cc.credential_ref.clone(),
                                                api_key: cc.api_key.clone(),
                                                expires_at: None,
                                            },
                                            enabled: cc.enabled,
                                            status: ChannelStatus::Healthy,
                                            circuit_open_until: None,
                                            base_url: cc.base_url.clone(),
                                            model_mapping: cc.model_mapping.clone(),
                                            created_at: chrono::Utc::now(),
                                            updated_at: chrono::Utc::now(),
                                            avg_latency_ms: 0,
                                            consecutive_failures: 0,
                                            cooldown_minutes: cc.cooldown_minutes,
                                            rpm_limit: cc.rpm_limit,
                                            tpm_limit: cc.tpm_limit,
                                        };
                                        let _ = channel_mgr.create(new_channel).await;
                                    }
                                }

                                // Delete channels that were removed from config
                                let current_channels = channel_mgr.list().await;
                                for ch in &current_channels {
                                    if !config_ids.contains(&ch.id) {
                                        tracing::info!(channel = %ch.name, id = %ch.id, "Removing channel deleted from config");
                                        let _ = channel_mgr.delete(ch.id).await;
                                    }
                                }
                                tracing::info!("Config reload complete");
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
