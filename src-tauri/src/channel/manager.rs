use crate::channel::{Channel, ChannelStatus, CredentialType, SharedChannels};
use crate::config::{AppConfig, ChannelConfig, GatewayConfig};
use crate::credential::SharedCredentialStore;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct ChannelManager {
    channels: SharedChannels,
    circuit_breaker_minutes: u64,
    gateway_config: GatewayConfig,
    credential_store: SharedCredentialStore,
}

impl ChannelManager {
    pub fn new(config: &AppConfig, credential_store: SharedCredentialStore) -> Self {
        let channels: Vec<Channel> = config.channels.iter().map(Channel::from_config).collect();

        Self {
            channels: Arc::new(RwLock::new(channels)),
            circuit_breaker_minutes: config.gateway.circuit_breaker_minutes,
            gateway_config: config.gateway.clone(),
            credential_store,
        }
    }

    pub fn channels(&self) -> SharedChannels {
        Arc::clone(&self.channels)
    }

    pub async fn list(&self) -> Vec<Channel> {
        let channels = self.channels.read().await;
        channels
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.recover_if_expired();
                c
            })
            .collect()
    }

    pub async fn get(&self, id: Uuid) -> Option<Channel> {
        let channels = self.channels.read().await;
        channels.iter().find(|c| c.id == id).cloned()
    }

    pub async fn create(&self, channel: Channel) -> Channel {
        let mut channels = self.channels.write().await;
        channels.push(channel.clone());
        channel
    }

    pub async fn update(&self, id: Uuid, updated: Channel) -> Option<Channel> {
        let mut channels = self.channels.write().await;
        if let Some(idx) = channels.iter().position(|c| c.id == id) {
            channels[idx] = updated.clone();
            Some(updated)
        } else {
            None
        }
    }

    pub async fn delete(&self, id: Uuid) -> bool {
        let mut channels = self.channels.write().await;
        let before = channels.len();
        channels.retain(|c| c.id != id);
        channels.len() < before
    }

    pub async fn mark_circuit_open(&self, id: Uuid) {
        let mut channels = self.channels.write().await;
        if let Some(ch) = channels.iter_mut().find(|c| c.id == id) {
            ch.consecutive_failures += 1;
            let base_minutes = ch.cooldown_minutes.unwrap_or(self.circuit_breaker_minutes);
            // Progressive backoff: 1x, 2x, 4x, 8x... capped at 30 min
            let backoff = (base_minutes as u32)
                .saturating_mul(2u32.saturating_pow(ch.consecutive_failures.saturating_sub(1)));
            let duration = (backoff as u64).min(30);
            ch.mark_circuit_open(duration);
        }
    }

    /// Mark circuit open with an optional retry-after duration (in seconds).
    pub async fn mark_circuit_open_with_retry(&self, id: Uuid, retry_after_secs: Option<u64>) {
        let duration_mins = retry_after_secs
            .map(|s| (s / 60).max(1))
            .unwrap_or(self.circuit_breaker_minutes);
        let mut channels = self.channels.write().await;
        if let Some(ch) = channels.iter_mut().find(|c| c.id == id) {
            ch.mark_circuit_open(duration_mins);
        }
    }

    pub async fn get_credential(&self, id: Uuid) -> Option<String> {
        let channels = self.channels.read().await;
        let ch = channels.iter().find(|c| c.id == id)?;

        // Priority 1: inline api_key from config
        if let Some(ref key) = ch.credential.api_key {
            return Some(key.clone());
        }

        // Priority 2: lookup from credential store (keyring/file)
        let service = "modelswitch";
        let username = &ch.credential.key_ref;

        self.credential_store.get(service, username).ok().flatten()
    }

    /// Persist current channels to config file. Best-effort: logs errors but does not propagate.
    pub async fn persist(&self) {
        let channels = self.channels.read().await;
        let channel_configs: Vec<ChannelConfig> = channels
            .iter()
            .map(|c| ChannelConfig {
                id: c.id.to_string(),
                name: c.name.clone(),
                provider: c.provider.as_str().to_string(),
                priority: c.priority,
                weight: c.weight,
                cost_per_token: c.cost_per_token,
                input_cost_per_mtok: c.input_cost_per_mtok,
                output_cost_per_mtok: c.output_cost_per_mtok,
                credential_type: match c.credential.cred_type {
                    CredentialType::ApiKey => "api_key".to_string(),
                    CredentialType::WebSession => "web_session".to_string(),
                },
                credential_ref: c.credential.key_ref.clone(),
                api_key: c.credential.api_key.clone(),
                base_url: c.base_url.clone(),
                enabled: c.enabled,
                model_mapping: c.model_mapping.clone(),
                cooldown_minutes: c.cooldown_minutes,
                rpm_limit: c.rpm_limit,
                tpm_limit: c.tpm_limit,
                account_group: c.account_group.clone(),
                payload_rules: None,
                quota: None,
                max_concurrent: c.max_concurrent,
            })
            .collect();
        drop(channels);

        // Preserve existing MCP server configs — only channels are being persisted.
        let existing_mcp_servers = AppConfig::load().map(|c| c.mcp_servers).unwrap_or_default();

        let config = AppConfig {
            gateway: self.gateway_config.clone(),
            channels: channel_configs,
            mcp_servers: existing_mcp_servers,
        };

        if let Err(e) = config.save() {
            tracing::error!("Failed to persist config: {}", e);
        }
    }

    /// Record a latency sample for a channel and reset its consecutive failure count.
    pub async fn record_latency(&self, id: Uuid, latency_ms: u64) {
        let mut channels = self.channels.write().await;
        if let Some(ch) = channels.iter_mut().find(|c| c.id == id) {
            // Exponential moving average (alpha = 0.3)
            if ch.avg_latency_ms == 0 {
                ch.avg_latency_ms = latency_ms;
            } else {
                ch.avg_latency_ms =
                    (ch.avg_latency_ms as f64 * 0.7 + latency_ms as f64 * 0.3) as u64;
            }
            ch.consecutive_failures = 0;
            // Promote HalfOpen to Healthy after a successful dispatch.
            ch.recover_to_healthy();
        }
    }

    /// Force-recover a channel from circuit-open state.
    pub async fn force_recover(&self, id: Uuid) {
        let mut channels = self.channels.write().await;
        if let Some(ch) = channels.iter_mut().find(|c| c.id == id) {
            ch.status = ChannelStatus::Healthy;
            ch.circuit_open_until = None;
            ch.consecutive_failures = 0;
            ch.updated_at = chrono::Utc::now();
        }
    }
}
