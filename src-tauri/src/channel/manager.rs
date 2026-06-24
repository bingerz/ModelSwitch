use crate::channel::{Channel, ChannelStatus, CredentialType, SharedChannels};
use crate::config::{AppConfig, ChannelConfig, GatewayConfig};
use crate::credential::SharedCredentialStore;
use parking_lot::{Mutex, RwLock as StdRwLock};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct ChannelManager {
    channels: SharedChannels,
    circuit_breaker_minutes: u64,
    gateway_config: GatewayConfig,
    credential_store: SharedCredentialStore,
    /// Per-channel rotation counter for multi-key round-robin selection.
    key_indices: Mutex<HashMap<Uuid, usize>>,
    /// When true, globally disables circuit breaker cooldown on failures.
    disable_cooling: bool,
}

impl ChannelManager {
    pub fn new(config: &AppConfig, credential_store: SharedCredentialStore) -> Self {
        let channels: HashMap<Uuid, Arc<StdRwLock<Channel>>> = config
            .channels
            .iter()
            .map(|c| {
                let ch = Channel::from_config(c);
                (ch.id, Arc::new(StdRwLock::new(ch)))
            })
            .collect();

        Self {
            channels: Arc::new(RwLock::new(channels)),
            circuit_breaker_minutes: config.gateway.circuit_breaker_minutes,
            gateway_config: config.gateway.clone(),
            credential_store,
            key_indices: Mutex::new(HashMap::new()),
            disable_cooling: config.gateway.disable_cooling,
        }
    }

    /// Set the global disable-cooling override at runtime.
    pub fn set_disable_cooling(&mut self, value: bool) {
        self.disable_cooling = value;
    }

    pub fn channels(&self) -> SharedChannels {
        Arc::clone(&self.channels)
    }

    pub async fn list(&self) -> Vec<Channel> {
        let channels = self.channels.read().await;
        channels
            .values()
            .filter_map(|ch_arc| {
                let ch = ch_arc.read();
                let mut c = ch.clone();
                c.recover_if_expired();
                Some(c)
            })
            .collect()
    }

    pub async fn get(&self, id: Uuid) -> Option<Channel> {
        let channels = self.channels.read().await;
        channels.get(&id).map(|ch_arc| {
            let ch = ch_arc.read();
            ch.clone()
        })
    }

    pub async fn create(&self, channel: Channel) -> Channel {
        let mut channels = self.channels.write().await;
        channels.insert(channel.id, Arc::new(StdRwLock::new(channel.clone())));
        channel
    }

    pub async fn update(&self, id: Uuid, updated: Channel) -> Option<Channel> {
        let mut channels = self.channels.write().await;
        if channels.contains_key(&id) {
            channels.insert(id, Arc::new(StdRwLock::new(updated.clone())));
            Some(updated)
        } else {
            None
        }
    }

    pub async fn delete(&self, id: Uuid) -> bool {
        let mut channels = self.channels.write().await;
        channels.remove(&id).is_some()
    }

    pub async fn mark_circuit_open(&self, id: Uuid) {
        // Read outer lock only briefly to clone the Arc, then drop it before
        // acquiring the inner write lock. This ensures circuit-breaker writes
        // do not block routing reads of other channels.
        let ch_arc = {
            let channels = self.channels.read().await;
            channels.get(&id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let mut ch = ch_arc.write();
            ch.consecutive_failures += 1;
            if self.disable_cooling {
                // Track the failure, but skip cooldown entirely.
                return;
            }
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
        if self.disable_cooling {
            return;
        }
        let duration_mins = retry_after_secs
            .map(|s| (s / 60).max(1))
            .unwrap_or(self.circuit_breaker_minutes);

        let ch_arc = {
            let channels = self.channels.read().await;
            channels.get(&id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let mut ch = ch_arc.write();
            ch.mark_circuit_open(duration_mins);
        }
    }

    /// Record a per-model rate-limit cooldown on a specific channel.
    /// This allows the router to skip this channel for the rate-limited model
    /// while still routing other models to it.
    pub async fn mark_model_rate_limited(
        &self,
        id: Uuid,
        model: &str,
        retry_after_secs: Option<u64>,
    ) {
        if self.disable_cooling {
            return;
        }
        let ch_arc = {
            let channels = self.channels.read().await;
            channels.get(&id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let mut ch = ch_arc.write();
            ch.mark_model_rate_limited(model, retry_after_secs);
        }
    }

    /// Clean expired model cooldowns across all channels.
    /// Call periodically to prevent the cooldown maps from growing unbounded.
    pub async fn clean_expired_model_cooldowns(&self) {
        let channels = self.channels.read().await;
        for ch_arc in channels.values() {
            let mut ch = ch_arc.write();
            ch.clean_expired_model_cooldowns();
        }
    }

    pub async fn get_credential(&self, id: Uuid) -> Option<String> {
        // Snapshot the fields we need under the inner read lock, then release.
        // The credential-store lookup (keyring/file IO) happens outside any
        // channel lock so a slow keyring does not stall routing.
        let (api_keys, primary_key, key_ref) = {
            let channels = self.channels.read().await;
            let ch_arc = channels.get(&id)?;
            let ch = ch_arc.read();
            (
                ch.api_keys.clone(),
                ch.credential.api_key.clone(),
                ch.credential.key_ref.clone(),
            )
        };

        // Multi-key rotation: when additional keys are configured, rotate
        // through [credential.api_key, ...api_keys] round-robin.
        if !api_keys.is_empty() {
            let all_keys = {
                let mut keys = Vec::with_capacity(1 + api_keys.len());
                if let Some(ref key) = primary_key {
                    keys.push(key.clone());
                }
                keys.extend(api_keys);
                keys
            };
            if all_keys.is_empty() {
                // Fall through to credential store lookup below
            } else {
                let idx = {
                    let mut counters = self.key_indices.lock();
                    let entry = counters.entry(id).or_insert(0);
                    let current = *entry;
                    *entry = (current + 1) % all_keys.len();
                    current
                };
                return Some(all_keys[idx].clone());
            }
        }

        // Priority 1: inline api_key from config
        if let Some(ref key) = primary_key {
            return Some(key.clone());
        }

        // Priority 2: lookup from credential store (keyring/file)
        let service = "modelswitch";
        let username = &key_ref;

        self.credential_store.get(service, username).ok().flatten()
    }

    /// Persist current channels to config file. Best-effort: logs errors but does not propagate.
    pub async fn persist(&self) {
        let channel_configs: Vec<ChannelConfig> = {
            let channels = self.channels.read().await;
            channels
                .values()
                .filter_map(|ch_arc| {
                    let c = ch_arc.read();
                    Some(ChannelConfig {
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
                        api_keys: c.api_keys.clone(),
                        excluded_models: c.excluded_models.clone(),
                        proxy_url: c.proxy_url.clone(),
                        headers: if c.headers.is_empty() {
                            None
                        } else {
                            Some(c.headers.clone())
                        },
                        max_retries: c.max_retries,
                        models_endpoint: c.models_endpoint.clone(),
                        models_refresh_interval_secs: c.models_refresh_interval_secs,
                    })
                })
                .collect()
        };

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
        let ch_arc = {
            let channels = self.channels.read().await;
            channels.get(&id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let mut ch = ch_arc.write();
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
        let ch_arc = {
            let channels = self.channels.read().await;
            channels.get(&id).map(Arc::clone)
        };
        if let Some(ch_arc) = ch_arc {
            let mut ch = ch_arc.write();
            ch.status = ChannelStatus::Healthy;
            ch.circuit_open_until = None;
            ch.consecutive_failures = 0;
            ch.updated_at = chrono::Utc::now();
        }
    }

    /// Atomically replace the entire channel list with a new set of channels.
    /// This acquires the write lock exactly once and swaps the whole map,
    /// so dispatch never sees a partially-updated list.
    pub async fn replace_all(&self, new_channels: Vec<Channel>) {
        let mut channels = self.channels.write().await;
        let new_map: HashMap<Uuid, Arc<StdRwLock<Channel>>> = new_channels
            .into_iter()
            .map(|c| (c.id, Arc::new(StdRwLock::new(c))))
            .collect();
        *channels = new_map;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
    use crate::credential::file_store::FileCredentialStore;
    use chrono::{Duration, Utc};

    // -- Test helpers ---------------------------------------------------------

    fn make_test_manager() -> ChannelManager {
        let config = AppConfig {
            gateway: GatewayConfig {
                circuit_breaker_minutes: 5,
                ..Default::default()
            },
            ..Default::default()
        };
        let credential_store: SharedCredentialStore = Arc::new(FileCredentialStore::new());
        ChannelManager::new(&config, credential_store)
    }

    fn make_test_channel(id: Uuid, name: &str) -> Channel {
        Channel {
            id,
            name: name.to_string(),
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
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: "https://api.openai.com/v1".to_string(),
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
        }
    }

    // -- CRUD tests -----------------------------------------------------------

    #[tokio::test]
    async fn create_adds_channel_to_list() {
        let manager = make_test_manager();
        let channel = make_test_channel(Uuid::new_v4(), "test-create");
        let created = manager.create(channel).await;

        let list = manager.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);
        assert_eq!(list[0].name, "test-create");
    }

    #[tokio::test]
    async fn delete_removes_channel() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();
        let channel = make_test_channel(id, "test-delete");
        manager.create(channel).await;

        assert!(manager.delete(id).await);

        let list = manager.list().await;
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn update_modifies_channel() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();
        let channel = make_test_channel(id, "original");
        manager.create(channel).await;

        let mut updated = manager.get(id).await.unwrap();
        updated.name = "updated".to_string();
        updated.priority = 5;
        manager.update(id, updated).await;

        let fetched = manager.get(id).await.unwrap();
        assert_eq!(fetched.name, "updated");
        assert_eq!(fetched.priority, 5);
    }

    #[tokio::test]
    async fn list_returns_all_channels() {
        let manager = make_test_manager();
        for i in 0..3 {
            let ch = make_test_channel(Uuid::new_v4(), &format!("ch-{i}"));
            manager.create(ch).await;
        }

        let list = manager.list().await;
        assert_eq!(list.len(), 3);
    }

    // -- Circuit breaker tests ------------------------------------------------

    #[tokio::test]
    async fn circuit_recovers_after_cooldown() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();
        let channel = make_test_channel(id, "cb-recovery");
        manager.create(channel).await;

        // Open the circuit.
        manager.mark_circuit_open(id).await;
        let ch = manager.get(id).await.unwrap();
        assert_eq!(ch.status, ChannelStatus::CircuitOpen);

        // Simulate cooldown expiry by rewinding circuit_open_until into the
        // past, then persist via update().
        let mut ch = manager.get(id).await.unwrap();
        ch.circuit_open_until = Some(Utc::now() - Duration::minutes(1));
        manager.update(id, ch).await;

        // list() calls recover_if_expired() on every channel, transitioning
        // CircuitOpen -> HalfOpen when the cooldown has elapsed.
        let list = manager.list().await;
        let recovered = list.iter().find(|c| c.id == id).unwrap();
        assert_eq!(recovered.status, ChannelStatus::HalfOpen);
        assert!(recovered.circuit_open_until.is_none());
    }

    // -- Credential rotation tests --------------------------------------------

    #[tokio::test]
    async fn get_credential_rotates_keys() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();

        let mut channel = make_test_channel(id, "key-rotation");
        channel.credential.api_key = Some("key1".to_string());
        channel.api_keys = vec!["key2".to_string(), "key3".to_string()];
        manager.create(channel).await;

        // all_keys() = [key1, key2, key3]; round-robin index advances each call.
        let k1 = manager.get_credential(id).await.unwrap();
        let k2 = manager.get_credential(id).await.unwrap();
        let k3 = manager.get_credential(id).await.unwrap();
        let k4 = manager.get_credential(id).await.unwrap();

        assert_eq!(k1, "key1");
        assert_eq!(k2, "key2");
        assert_eq!(k3, "key3");
        assert_eq!(k4, "key1");
    }

    #[tokio::test]
    async fn get_credential_returns_primary_when_no_rotation_keys() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();

        // make_test_channel sets credential.api_key = Some("sk-test") and
        // api_keys = vec![] — no rotation keys configured.
        let channel = make_test_channel(id, "single-key");
        manager.create(channel).await;

        let k1 = manager.get_credential(id).await.unwrap();
        let k2 = manager.get_credential(id).await.unwrap();

        assert_eq!(k1, "sk-test");
        assert_eq!(k2, "sk-test");
    }
}
