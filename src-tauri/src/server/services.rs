//! Background service spawning: persistence, cleanup, config watcher, model discovery.

use std::sync::Arc;

use crate::channel::manager::ChannelManager;
use crate::config::{self, AppConfig};
use crate::health;
use crate::model_registry::{refresh_from_endpoint, ModelRegistry};
use crate::proxy::AppState;
use crate::quota;
use crate::quota::registry::QuotaProviderRegistry;
use crate::spawn_bg;

/// Spawn tasks that load persisted state and periodically save it to disk.
pub(super) fn spawn_persistence_tasks(state: &Arc<AppState>) {
    // Load persisted dispatch logs at startup
    let boot_logger = Arc::clone(&state.logger);
    spawn_bg(async move {
        boot_logger.load_from_file().await;
    });

    // Load persisted audit log at startup (administrative history survives restarts)
    let boot_audit = Arc::clone(&state.audit_log);
    spawn_bg(async move {
        boot_audit.load_from_file().await;
    });

    // Load persisted quota data (token usage survives restarts)
    {
        let boot_quota = Arc::clone(&state.billing.quota_store);
        spawn_bg(async move {
            boot_quota.load_from_file().await;
        });
    }

    // Periodic quota persistence (every 10s)
    {
        let persist_quota = Arc::clone(&state.billing.quota_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                persist_quota.persist_to_file().await;
            }
        });
    }

    // Load persisted virtual keys (so keys + spend survive restarts)
    {
        let boot_vk = Arc::clone(&state.billing.virtual_key_store);
        spawn_bg(async move {
            if let Err(e) = boot_vk.load().await {
                tracing::warn!(error = %e, "Failed to load virtual keys");
            } else {
                let count = boot_vk.list().await.len();
                tracing::info!(count, "Loaded virtual keys from disk");
            }
        });
    }

    // Periodic virtual key persistence (every 10s)
    {
        let persist_vk = Arc::clone(&state.billing.virtual_key_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Err(e) = persist_vk.persist().await {
                    tracing::warn!(error = %e, "Failed to persist virtual keys");
                }
            }
        });
    }

    // Load persisted provider budget spend (so limits + spend survive restarts)
    {
        let boot_pb = Arc::clone(&state.billing.provider_budgets);
        spawn_bg(async move {
            if let Err(e) = boot_pb.load().await {
                tracing::warn!(error = %e, "Failed to load provider budgets");
            } else {
                tracing::info!("Loaded provider budget spend from disk");
            }
        });
    }

    // Periodic provider budget persistence (every 10s)
    {
        let persist_pb = Arc::clone(&state.billing.provider_budgets);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Err(e) = persist_pb.persist().await {
                    tracing::warn!(error = %e, "Failed to persist provider budgets");
                }
            }
        });
    }
}

/// Spawn background services: health checker, quota poller, session affinity cleanup, cache sweep.
pub(super) fn spawn_background_services(state: &Arc<AppState>, config: &AppConfig) {
    // Start background health probe (P2-7: simplified periodic connectivity check).
    // When health_check_interval_secs > 0, spawns a lightweight probe that
    // verifies each channel's base URL is reachable without sending API requests.
    if config.gateway.health_check_enabled && config.gateway.health_check_interval_secs > 0 {
        let probe_state = Arc::clone(state);
        let probe_interval = config.gateway.health_check_interval_secs;
        spawn_bg(async move {
            health::run_periodic_probe(probe_state, probe_interval).await;
        });
    }

    // Start background quota poller
    {
        let qp_mgr = Arc::clone(&state.channel_mgr);
        let qp_store = Arc::clone(&state.billing.quota_store);
        let qp_client = state.http_pool.first().clone();
        let qp_interval = config.gateway.quota_poll_interval_secs;
        let qp_registry = Arc::new(QuotaProviderRegistry::new(
            quota::collectors::default_registry(),
        ));
        // Build per-channel quota config map
        let qp_configs: std::collections::HashMap<String, config::QuotaConfig> = config
            .channels
            .iter()
            .filter_map(|ch| ch.quota.as_ref().map(|q| (ch.id.clone(), q.clone())))
            .collect();
        spawn_bg(async move {
            quota::poller::start_quota_poller(
                qp_mgr,
                qp_store,
                qp_client,
                qp_interval,
                qp_registry,
                qp_configs,
            )
            .await;
        });
    }

    // Periodic session affinity cleanup
    {
        let affinity_cleanup = state.router.session_affinity.clone();
        spawn_bg(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                affinity_cleanup.cleanup().await;
            }
        });
    }

    // Periodic cache sweep — bulk-evict expired entries every 60s so that
    // `get()` only needs a lazy per-key TTL check.
    {
        let sweep_cache = Arc::clone(&state.cache.request_cache);
        spawn_bg(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                sweep_cache.sweep_expired();
            }
        });
    }

    // Periodic cleanup of expired per-model cooldowns (every 5 minutes)
    {
        let channel_mgr_cleanup = Arc::clone(&state.channel_mgr);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await; // Skip first immediate tick
            loop {
                interval.tick().await;
                channel_mgr_cleanup.clean_expired_model_cooldowns().await;
            }
        });
    }

    // Start background model discovery for channels with models_endpoint configured
    {
        let channel_mgr = Arc::clone(&state.channel_mgr);
        let registry = Arc::clone(&state.model_registry);
        spawn_bg(async move {
            run_model_discovery(channel_mgr, registry).await;
        });
    }
}

/// Start the hot config reload watcher.
pub(super) fn spawn_config_watcher(
    state: &Arc<AppState>,
    watcher_config_path: &Option<std::path::PathBuf>,
) {
    // Start hot config reload watcher
    let watcher_path = watcher_config_path
        .clone()
        .or_else(|| AppConfig::config_path().ok());
    if let Some(path) = watcher_path {
        config::watcher::start_config_watcher(
            path,
            Arc::clone(&state.channel_mgr),
            Arc::clone(&state.mcp.mcp_manager),
            Arc::clone(&state.limits.rate_limiter),
            Arc::clone(&state.limits.payload_rules),
        );
    }
}

/// Background model discovery for channels with `models_endpoint` configured.
///
/// Scans all channels on startup, finds those with a configured endpoint, and
/// spawns a per-channel tokio task that periodically fetches available models
/// and updates the shared registry.
async fn run_model_discovery(
    channel_mgr: Arc<ChannelManager>,
    registry: Arc<parking_lot::RwLock<ModelRegistry>>,
) {
    use uuid::Uuid;

    // Collect channels with models_endpoint
    let configs: Vec<(Uuid, String, u64)> = {
        let channels = channel_mgr.channels();
        let guard = channels.read().await;
        guard
            .values()
            .filter_map(|ch_arc| {
                let ch = ch_arc.read();
                ch.models_endpoint
                    .as_ref()
                    .map(|ep| (ch.id, ep.clone(), ch.models_refresh_interval_secs))
            })
            .collect()
    };

    for (channel_id, endpoint, interval_secs) in configs {
        let mgr = Arc::clone(&channel_mgr);
        let reg = Arc::clone(&registry);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            // First tick completes immediately, subsequent ticks wait for the interval
            loop {
                ticker.tick().await;
                let api_key = mgr.get_credential(channel_id).await;
                if let Some(key) = api_key {
                    match refresh_from_endpoint(&endpoint, &key).await {
                        Ok(models) => {
                            tracing::info!(
                                channel_id = %channel_id,
                                count = models.len(),
                                "Discovered {} models from endpoint {}",
                                models.len(),
                                endpoint,
                            );
                            reg.write().update_models(models, &endpoint);
                        }
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %channel_id,
                                error = %e,
                                "Failed to refresh models from endpoint {}",
                                endpoint,
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        channel_id = %channel_id,
                        "No API key found for model discovery",
                    );
                }
            }
        });
    }
}
