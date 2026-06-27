pub mod probe;

use crate::channel::manager::ChannelManager;
use crate::channel::{Channel, ChannelStatus};
use std::sync::Arc;
use std::time::Duration;

/// Probe a single channel and update its status in the channel manager.
/// When the probe succeeds, latency is recorded and circuit-open channels
/// are force-recovered. When it fails, healthy channels are tripped to
/// circuit-open.
///
/// This forms the thorough (API-specific) probe pathway together with
/// [`start_health_checker`] and [`probe::probe_channel`]. The production
/// server currently wires only the lighter [`run_periodic_probe`] (which
/// uses `probe_connectivity`). This thorough pathway is kept — and its
/// tests exercise real provider-specific probing logic — so it can be
/// swapped in via `start_health_checker` when deeper health checks are
/// needed. Marked dead_code because no production caller invokes it yet.
#[allow(dead_code)]
pub(crate) async fn check_channel_health(
    channel_mgr: &ChannelManager,
    http_client: &reqwest::Client,
    channel: &Channel,
) {
    let start = std::time::Instant::now();
    let api_key = channel_mgr.get_credential(channel.id).await;
    let healthy = probe::probe_channel(http_client, channel, api_key.as_deref()).await;
    let latency = start.elapsed().as_millis() as u64;

    if healthy {
        tracing::debug!(
            channel = %channel.name,
            latency_ms = latency,
            "Health check passed"
        );
        let _ = channel_mgr.record_latency(channel.id, latency).await;

        if channel.status == ChannelStatus::CircuitOpen {
            tracing::info!(
                channel = %channel.name,
                "Recovering circuit-open channel via health check"
            );
            channel_mgr.force_recover(channel.id).await;
        }
    } else {
        tracing::warn!(channel = %channel.name, "Health check failed");
        if channel.status == ChannelStatus::Healthy {
            channel_mgr.mark_circuit_open(channel.id).await;
        }
    }
}

/// Start the background health checker loop.
/// Periodically probes all enabled channels and updates their status.
///
/// This is the more thorough probe that sends provider-specific API requests
/// to validate credentials and endpoint reachability. For a simpler
/// connectivity-only check, use [`run_periodic_probe`] instead.
///
/// Not currently wired into server startup (server uses `run_periodic_probe`).
/// Kept as a complete, tested alternative for deeper health checks.
#[allow(dead_code)]
pub fn start_health_checker(
    channel_mgr: Arc<ChannelManager>,
    interval_secs: u64,
    http_client: reqwest::Client,
) {
    crate::spawn_bg(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;

            let channels = channel_mgr.list().await;

            // Probe all enabled channels concurrently
            let futures: Vec<_> = channels
                .iter()
                .filter(|ch| ch.enabled)
                .map(|ch| {
                    let http_client = http_client.clone();
                    let channel_mgr = Arc::clone(&channel_mgr);
                    async move {
                        check_channel_health(&channel_mgr, &http_client, ch).await;
                    }
                })
                .collect();

            futures::future::join_all(futures).await;
        }
    });
}

/// Run a periodic connectivity probe against all enabled channels.
///
/// This is a simplified health check (P2-7) that only verifies each channel's
/// base URL is reachable via a simple HTTP GET — no API-specific endpoints,
/// no credentials, no model requests. Suitable as a lightweight liveness check
/// that runs alongside (or instead of) the more thorough [`start_health_checker`].
///
/// When `interval_secs` is 0, the function returns immediately (disabled).
///
/// When a previously circuit-open channel responds to the connectivity probe,
/// it is force-recovered. When a healthy channel fails connectivity, its
/// circuit breaker is opened.
pub async fn run_periodic_probe(state: Arc<crate::proxy::AppState>, interval_secs: u64) {
    if interval_secs == 0 {
        tracing::debug!("Periodic connectivity probe disabled (interval_secs = 0)");
        return;
    }

    tracing::info!(interval_secs, "Starting periodic connectivity probe");
    // PooledClient derefs to reqwest::Client; .clone() on the derefed target
    // produces an owned reqwest::Client suitable for moving into the loop.
    let http_client: reqwest::Client = state.http_pool.first().clone();
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        ticker.tick().await;

        let channels = state.channel_mgr.list().await;
        let channel_mgr = Arc::clone(&state.channel_mgr);

        // Probe all enabled channels concurrently
        let futures: Vec<_> = channels
            .iter()
            .filter(|ch| ch.enabled)
            .map(|ch| {
                let http_client = http_client.clone();
                let channel_mgr = Arc::clone(&channel_mgr);
                let channel = ch.clone();
                async move {
                    let healthy = probe::probe_connectivity(&http_client, &channel).await;
                    if healthy {
                        if channel.status == ChannelStatus::CircuitOpen {
                            tracing::info!(
                                channel = %channel.name,
                                "Connectivity probe succeeded — recovering channel"
                            );
                            channel_mgr.force_recover(channel.id).await;
                        }
                    } else if channel.status == ChannelStatus::Healthy {
                        tracing::warn!(
                            channel = %channel.name,
                            "Connectivity probe failed — opening circuit"
                        );
                        channel_mgr.mark_circuit_open(channel.id).await;
                    }
                }
            })
            .collect();

        futures::future::join_all(futures).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, Credential, CredentialType, Provider};
    use crate::config::{AppConfig, GatewayConfig};
    use crate::credential::file_store::FileCredentialStore;
    use crate::credential::SharedCredentialStore;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // -- Test helpers ---------------------------------------------------------

    fn make_credential_store() -> SharedCredentialStore {
        Arc::new(FileCredentialStore::new())
    }

    fn make_channel(base_url: &str, status: ChannelStatus) -> Channel {
        Channel {
            id: Uuid::new_v4(),
            name: "health-test".to_string(),
            provider: Provider::OpenAI,
            priority: 1,
            weight: 1,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: "test".to_string(),
                api_key: Some("sk-test".to_string()),
                expires_at: None,
            },
            enabled: true,
            status,
            circuit_open_until: None,
            base_url: base_url.to_string(),
            model_mapping: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
            api_keys: vec![],
            excluded_models: vec![],
            model_cooldowns: HashMap::new(),
            proxy_url: None,
            headers: HashMap::new(),
            max_retries: None,
            models_endpoint: None,
            models_refresh_interval_secs: 300,
            tags: vec![],
        }
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }

    // -- Health check interaction tests ---------------------------------------

    #[tokio::test]
    async fn healthy_channel_stays_healthy_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());

        let channel = make_channel(&server.uri(), ChannelStatus::Healthy);
        let id = channel.id;
        mgr.create(channel).await;

        check_channel_health(&mgr, &http_client(), &mgr.get(id).await.unwrap()).await;

        let ch = mgr.get(id).await.unwrap();
        assert_eq!(
            ch.status,
            ChannelStatus::Healthy,
            "healthy channel should remain healthy after successful probe"
        );
        assert_eq!(ch.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn healthy_channel_transitions_to_circuit_open_on_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());

        let channel = make_channel(&server.uri(), ChannelStatus::Healthy);
        let id = channel.id;
        mgr.create(channel).await;

        check_channel_health(&mgr, &http_client(), &mgr.get(id).await.unwrap()).await;

        let ch = mgr.get(id).await.unwrap();
        assert_eq!(
            ch.status,
            ChannelStatus::CircuitOpen,
            "healthy channel should be tripped to CircuitOpen on probe failure"
        );
        assert!(ch.circuit_open_until.is_some());
    }

    #[tokio::test]
    async fn circuit_open_channel_recovers_on_successful_probe() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());

        let channel = make_channel(&server.uri(), ChannelStatus::CircuitOpen);
        let id = channel.id;
        mgr.create(channel).await;

        check_channel_health(&mgr, &http_client(), &mgr.get(id).await.unwrap()).await;

        let ch = mgr.get(id).await.unwrap();
        assert_eq!(
            ch.status,
            ChannelStatus::Healthy,
            "circuit-open channel should recover to Healthy on successful probe"
        );
        assert!(ch.circuit_open_until.is_none());
        assert_eq!(ch.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn half_open_channel_stays_half_open_on_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());

        // HalfOpen channel — should NOT be tripped to CircuitOpen
        // (only Healthy channels are tripped on probe failure)
        let channel = make_channel(&server.uri(), ChannelStatus::HalfOpen);
        let id = channel.id;
        mgr.create(channel).await;

        check_channel_health(&mgr, &http_client(), &mgr.get(id).await.unwrap()).await;

        let ch = mgr.get(id).await.unwrap();
        assert_eq!(
            ch.status,
            ChannelStatus::HalfOpen,
            "HalfOpen channel should not change status on probe failure \
             (only Healthy channels are tripped)"
        );
    }

    #[tokio::test]
    async fn connection_error_trips_healthy_channel() {
        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());

        // Channel pointing to a dead port
        let channel = make_channel("http://127.0.0.1:1", ChannelStatus::Healthy);
        let id = channel.id;
        mgr.create(channel).await;

        check_channel_health(&mgr, &http_client(), &mgr.get(id).await.unwrap()).await;

        let ch = mgr.get(id).await.unwrap();
        assert_eq!(
            ch.status,
            ChannelStatus::CircuitOpen,
            "connection error should trip healthy channel"
        );
    }

    #[tokio::test]
    async fn parallel_health_checks_complete_for_multiple_channels() {
        // Two mock servers — one healthy, one unhealthy
        let ok_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ok_server)
            .await;

        let fail_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&fail_server)
            .await;

        let config = AppConfig {
            gateway: GatewayConfig::default(),
            channels: vec![],
            mcp_servers: vec![],
        };
        let mgr = ChannelManager::new(&config, make_credential_store());
        let client = http_client();

        let ch1 = make_channel(&ok_server.uri(), ChannelStatus::Healthy);
        let ch2 = make_channel(&fail_server.uri(), ChannelStatus::Healthy);
        let id1 = ch1.id;
        let id2 = ch2.id;
        mgr.create(ch1).await;
        mgr.create(ch2).await;

        // Run both checks concurrently (simulating the parallel join_all)
        let ch1_snapshot = mgr.get(id1).await.unwrap();
        let ch2_snapshot = mgr.get(id2).await.unwrap();
        let f1 = check_channel_health(&mgr, &client, &ch1_snapshot);
        let f2 = check_channel_health(&mgr, &client, &ch2_snapshot);
        tokio::join!(f1, f2);

        let result1 = mgr.get(id1).await.unwrap();
        let result2 = mgr.get(id2).await.unwrap();
        assert_eq!(result1.status, ChannelStatus::Healthy);
        assert_eq!(result2.status, ChannelStatus::CircuitOpen);
    }
}
