use crate::channel::manager::ChannelManager;
use crate::config::QuotaConfig;
use crate::quota::provider::PollContext;
use crate::quota::registry::QuotaProviderRegistry;
use crate::quota::SharedQuotaStore;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{self, Duration};

/// Background task that periodically polls provider billing/quota endpoints
/// using the QuotaProviderRegistry for strategy resolution.
pub async fn start_quota_poller(
    channel_mgr: Arc<ChannelManager>,
    quota_store: SharedQuotaStore,
    client: reqwest::Client,
    interval_secs: u64,
    registry: Arc<QuotaProviderRegistry>,
    quota_configs: HashMap<String, QuotaConfig>,
) {
    // Poll immediately on startup — don't wait for the first interval tick.
    poll_all_channels(
        &channel_mgr,
        &quota_store,
        &client,
        &registry,
        &quota_configs,
    )
    .await;

    let mut interval = time::interval(Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;
        poll_all_channels(
            &channel_mgr,
            &quota_store,
            &client,
            &registry,
            &quota_configs,
        )
        .await;
    }
}

async fn poll_all_channels(
    channel_mgr: &Arc<ChannelManager>,
    quota_store: &SharedQuotaStore,
    client: &reqwest::Client,
    registry: &Arc<QuotaProviderRegistry>,
    quota_configs: &HashMap<String, QuotaConfig>,
) {
    let channels = channel_mgr.list().await;

    for ch in &channels {
        if !ch.enabled {
            continue;
        }

        let api_key = match channel_mgr.get_credential(ch.id).await {
            Some(key) => key,
            None => {
                // Create an error entry so the frontend shows an actionable message
                // instead of a permanent "PENDING / Waiting for quota data..."
                let info = crate::quota::QuotaInfo {
                    error: Some(
                        "No API key configured — add api_key in channel config or re-create the channel".into(),
                    ),
                    ..crate::quota::QuotaInfo::new(
                        ch.id,
                        &ch.name,
                        ch.provider.as_str(),
                        "http_api",
                    )
                };
                quota_store.update(info).await;
                tracing::warn!(
                    channel = %ch.name,
                    "No credential found — quota entry created with error"
                );
                continue;
            }
        };

        let quota_config = quota_configs.get(&ch.id.to_string()).cloned();

        let ctx = PollContext {
            channel_id: ch.id,
            channel_name: ch.name.clone(),
            provider: ch.provider.as_str().to_string(),
            base_url: ch.base_url.clone(),
            credential: api_key,
            http_client: client.clone(),
            quota_config,
        };

        let result = registry.poll(&ctx).await;

        match result {
            Ok(mut info) => {
                tracing::info!(
                    channel = %ch.name,
                    balance = ?info.balance,
                    usage = ?info.usage,
                    "Quota polled"
                );
                info.updated_at = chrono::Utc::now();
                quota_store.update(info).await;
            }
            Err(crate::quota::QuotaError::Unsupported(reason)) => {
                // Still create an entry so the frontend shows the channel
                // with an explanatory error instead of "Waiting for data..."
                let info = crate::quota::QuotaInfo {
                    error: Some(reason.clone()),
                    ..crate::quota::QuotaInfo::new(
                        ch.id,
                        &ch.name,
                        ch.provider.as_str(),
                        "http_api",
                    )
                };
                quota_store.update(info).await;
                tracing::debug!(
                    channel = %ch.name,
                    reason = %reason,
                    "Quota not supported for channel"
                );
            }
            Err(e) => {
                tracing::debug!(
                    channel = %ch.name,
                    error = %e,
                    "Quota poll failed"
                );
                let info = crate::quota::QuotaInfo::new(
                    ch.id,
                    &ch.name,
                    ch.provider.as_str(),
                    "http_api",
                );
                quota_store
                    .update(crate::quota::QuotaInfo {
                        error: Some(e.to_string()),
                        ..info
                    })
                    .await;
            }
        }
    }
}
