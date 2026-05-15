pub mod probe;

use crate::channel::manager::ChannelManager;
use std::sync::Arc;
use std::time::Duration;

/// Start the background health checker loop.
/// Periodically probes all enabled channels and updates their status.
pub fn start_health_checker(
    channel_mgr: Arc<ChannelManager>,
    interval_secs: u64,
    http_client: reqwest::Client,
) {
    crate::spawn_bg(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;

            let channels = channel_mgr.list().await;
            for ch in &channels {
                if !ch.enabled {
                    continue;
                }

                let start = std::time::Instant::now();
                let api_key = channel_mgr.get_credential(ch.id).await;
                let healthy = probe::probe_channel(&http_client, &ch, api_key.as_deref()).await;
                let latency = start.elapsed().as_millis() as u64;

                if healthy {
                    tracing::debug!(channel = %ch.name, latency_ms = latency, "Health check passed");
                    let _ = channel_mgr.record_latency(ch.id, latency).await;

                    // If channel was circuit-open, force recover since probe succeeded
                    if ch.status == crate::channel::ChannelStatus::CircuitOpen {
                        tracing::info!(channel = %ch.name, "Recovering circuit-open channel via health check");
                        channel_mgr.force_recover(ch.id).await;
                    }
                } else {
                    tracing::warn!(channel = %ch.name, "Health check failed");
                    // Only mark circuit-open if it was healthy (don't extend existing circuit-open)
                    if ch.status == crate::channel::ChannelStatus::Healthy {
                        channel_mgr.mark_circuit_open(ch.id).await;
                    }
                }
            }
        }
    });
}
