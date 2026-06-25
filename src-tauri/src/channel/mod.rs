pub mod manager;

use crate::config::ChannelConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    OpenAI,
    Anthropic,
    DeepSeek,
    Gemini,
    OpenRouter,
    Ollama,
    Mistral,
    Groq,
    Together,
    Cohere,
    XAI,
    SiliconFlow,
    Yi,
    Moonshot,
    Zhipu,
    Custom(String),
}

impl Serialize for Provider {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Provider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Provider::from_str(&s))
    }
}

impl Provider {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => Provider::OpenAI,
            "anthropic" => Provider::Anthropic,
            "deepseek" => Provider::DeepSeek,
            "gemini" => Provider::Gemini,
            "openrouter" => Provider::OpenRouter,
            "ollama" => Provider::Ollama,
            "mistral" => Provider::Mistral,
            "groq" => Provider::Groq,
            "together" => Provider::Together,
            "cohere" => Provider::Cohere,
            "xai" => Provider::XAI,
            "siliconflow" => Provider::SiliconFlow,
            "yi" => Provider::Yi,
            "moonshot" => Provider::Moonshot,
            "zhipu" => Provider::Zhipu,
            other => Provider::Custom(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::DeepSeek => "deepseek",
            Provider::Gemini => "gemini",
            Provider::OpenRouter => "openrouter",
            Provider::Ollama => "ollama",
            Provider::Mistral => "mistral",
            Provider::Groq => "groq",
            Provider::Together => "together",
            Provider::Cohere => "cohere",
            Provider::XAI => "xai",
            Provider::SiliconFlow => "siliconflow",
            Provider::Yi => "yi",
            Provider::Moonshot => "moonshot",
            Provider::Zhipu => "zhipu",
            Provider::Custom(s) => s,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    ApiKey,
    WebSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub cred_type: CredentialType,
    /// A reference key stored in system keyring
    pub key_ref: String,
    /// Inline API key from config (bypasses keyring lookup)
    pub api_key: Option<String>,
    /// For web sessions, when does the cookie expire
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatus {
    Healthy,
    /// Circuit was open, cooldown expired — limited probing allowed.
    /// Transition to Healthy on success, back to CircuitOpen on failure.
    HalfOpen,
    CircuitOpen,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Uuid,
    pub name: String,
    pub provider: Provider,
    pub priority: u8,
    pub weight: u32,
    pub cost_per_token: Option<f64>,
    /// Cost per million input tokens (overrides cost_per_token if set)
    #[serde(default)]
    pub input_cost_per_mtok: Option<f64>,
    /// Cost per million output tokens (overrides cost_per_token if set)
    #[serde(default)]
    pub output_cost_per_mtok: Option<f64>,
    pub credential: Credential,
    pub enabled: bool,
    pub status: ChannelStatus,
    pub circuit_open_until: Option<DateTime<Utc>>,
    pub base_url: String,
    pub model_mapping: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Rolling average latency in milliseconds (populated by health checks and dispatches)
    #[serde(default)]
    pub avg_latency_ms: u64,
    /// Consecutive failure count for progressive backoff
    #[serde(default)]
    pub consecutive_failures: u32,
    /// Per-channel cooldown override in minutes (None = use global default)
    #[serde(default)]
    pub cooldown_minutes: Option<u64>,
    /// Per-channel RPM limit (None = default 60)
    #[serde(default)]
    pub rpm_limit: Option<u64>,
    /// Per-channel TPM limit (None = no limit)
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    /// Optional account group tag for multi-account pool management.
    #[serde(default)]
    pub account_group: Option<String>,
    /// Maximum concurrent in-flight requests for this channel (None = no limit).
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// Additional API keys for rotation. The primary key lives in
    /// `credential.api_key`. When `api_keys` is non-empty, requests
    /// rotate through `[credential.api_key, ...api_keys]` round-robin.
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Glob patterns for models this channel should NOT serve.
    /// Uses simple wildcard matching: `*` matches any sequence, `?` matches one char.
    #[serde(default)]
    pub excluded_models: Vec<String>,
    /// Per-model rate-limit cooldowns: model name -> expiry time.
    /// When a model gets a 429 from upstream, it enters cooldown for this
    /// channel specifically — other models on the same channel remain available.
    #[serde(default)]
    pub model_cooldowns: HashMap<String, DateTime<Utc>>,
    /// Optional proxy URL for this channel (e.g., "socks5://host:port", "http://host:port").
    /// When set, requests to this channel's upstream use a dedicated reqwest client with this proxy.
    /// Use "direct" to explicitly bypass any global proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// Custom HTTP headers injected into upstream requests.
    pub headers: HashMap<String, String>,
    /// Per-channel max retries override.
    pub max_retries: Option<u32>,
    /// Optional endpoint for periodic model list refresh.
    pub models_endpoint: Option<String>,
    /// Interval between model list refreshes in seconds.
    pub models_refresh_interval_secs: u64,
    /// User-defined tags for grouping and filtering channels.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Channel {
    pub fn is_available(&self) -> bool {
        if !self.enabled || self.status == ChannelStatus::Disabled {
            return false;
        }
        if self.status == ChannelStatus::CircuitOpen {
            if let Some(until) = self.circuit_open_until {
                return Utc::now() >= until;
            }
            return false;
        }
        // Healthy and HalfOpen are available.
        // Router prefers Healthy over HalfOpen to limit probe traffic.
        true
    }

    pub fn map_model(&self, requested_model: &str) -> String {
        self.model_mapping
            .get(requested_model)
            .cloned()
            .unwrap_or_else(|| requested_model.to_string())
    }

    /// Return all available API keys for this channel, combining the primary key
    /// (`credential.api_key`) with additional rotation keys (`api_keys`).
    /// Returns an empty vec when no keys are configured.
    pub fn all_keys(&self) -> Vec<String> {
        let mut keys = Vec::with_capacity(1 + self.api_keys.len());
        if let Some(ref key) = self.credential.api_key {
            keys.push(key.clone());
        }
        keys.extend(self.api_keys.clone());
        keys
    }

    /// Check if a model is excluded by this channel's exclusion patterns.
    /// Returns `true` if the model matches any excluded pattern.
    pub fn is_model_excluded(&self, model: &str) -> bool {
        if self.excluded_models.is_empty() {
            return false;
        }
        self.excluded_models
            .iter()
            .any(|pattern| matches_glob(pattern, model))
    }

    /// Mark a model as rate-limited on this channel with progressive backoff.
    /// Cooldown duration: derived from `retry_after_secs` when provided,
    /// otherwise a fixed 2-minute default. Capped at 30 minutes.
    pub fn mark_model_rate_limited(&mut self, model: &str, retry_after_secs: Option<u64>) {
        let duration_mins = retry_after_secs
            .map(|s| ((s + 59) / 60).max(1).min(30))
            .unwrap_or(2);
        let expiry = Utc::now() + chrono::Duration::minutes(duration_mins as i64);
        self.model_cooldowns.insert(model.to_string(), expiry);
    }

    /// Check if a model is currently in rate-limit cooldown on this channel.
    pub fn is_model_in_cooldown(&self, model: &str) -> bool {
        if let Some(&expiry) = self.model_cooldowns.get(model) {
            if Utc::now() < expiry {
                return true;
            }
        }
        false
    }

    /// Remove expired model cooldowns to prevent the map from growing unbounded.
    pub fn clean_expired_model_cooldowns(&mut self) {
        let now = Utc::now();
        self.model_cooldowns.retain(|_, expiry| *expiry > now);
    }

    /// Calculate cost from real token counts.
    /// Uses input_cost_per_mtok/output_cost_per_mtok if set, falls back to cost_per_token.
    pub fn calculate_cost(
        &self,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Option<f64> {
        if input_tokens.is_none() && output_tokens.is_none() {
            return None;
        }

        let in_tok = input_tokens.unwrap_or(0) as f64;
        let out_tok = output_tokens.unwrap_or(0) as f64;

        if self.input_cost_per_mtok.is_some() || self.output_cost_per_mtok.is_some() {
            let in_rate = self.input_cost_per_mtok.unwrap_or(0.0);
            let out_rate = self.output_cost_per_mtok.unwrap_or(0.0);
            Some(in_tok / 1_000_000.0 * in_rate + out_tok / 1_000_000.0 * out_rate)
        } else {
            self.cost_per_token
                .map(|rate_per_1k| (in_tok + out_tok) / 1000.0 * rate_per_1k)
        }
    }

    pub fn mark_circuit_open(&mut self, duration_mins: u64) {
        self.status = ChannelStatus::CircuitOpen;
        self.circuit_open_until =
            Some(Utc::now() + chrono::Duration::minutes(duration_mins as i64));
        self.updated_at = Utc::now();
    }

    pub fn recover_if_expired(&mut self) {
        if self.status == ChannelStatus::CircuitOpen {
            if let Some(until) = self.circuit_open_until {
                if Utc::now() >= until {
                    // Transition to HalfOpen — limited probing until a successful
                    // dispatch confirms the upstream is healthy again.
                    self.status = ChannelStatus::HalfOpen;
                    self.circuit_open_until = None;
                    self.updated_at = Utc::now();
                }
            }
        }
    }

    /// Promote a HalfOpen channel to Healthy after a successful dispatch.
    /// Resets failure counters.
    pub fn recover_to_healthy(&mut self) {
        if self.status == ChannelStatus::HalfOpen {
            self.status = ChannelStatus::Healthy;
            self.consecutive_failures = 0;
            self.updated_at = Utc::now();
        }
    }

    /// Create a new Channel from a ChannelConfig with fresh runtime state.
    pub fn from_config(c: &ChannelConfig) -> Self {
        let cred_type = match c.credential_type.as_str() {
            "web_session" => crate::channel::CredentialType::WebSession,
            _ => crate::channel::CredentialType::ApiKey,
        };
        Channel {
            id: Uuid::parse_str(&c.id).unwrap_or_else(|_| Uuid::new_v4()),
            name: c.name.clone(),
            provider: Provider::from_str(&c.provider),
            priority: c.priority,
            weight: c.weight,
            cost_per_token: c.cost_per_token,
            input_cost_per_mtok: c.input_cost_per_mtok,
            output_cost_per_mtok: c.output_cost_per_mtok,
            credential: crate::channel::Credential {
                cred_type,
                key_ref: c.credential_ref.clone(),
                api_key: c.api_key.clone(),
                expires_at: None,
            },
            enabled: c.enabled,
            status: crate::channel::ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: c.base_url.clone(),
            model_mapping: c.model_mapping.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: c.cooldown_minutes,
            rpm_limit: c.rpm_limit,
            tpm_limit: c.tpm_limit,
            account_group: c.account_group.clone(),
            max_concurrent: c.max_concurrent,
            api_keys: c.api_keys.clone(),
            excluded_models: c.excluded_models.clone(),
            model_cooldowns: HashMap::new(),
            proxy_url: c.proxy_url.clone(),
            headers: c.headers.clone().unwrap_or_default(),
            max_retries: c.max_retries,
            models_endpoint: c.models_endpoint.clone(),
            models_refresh_interval_secs: c.models_refresh_interval_secs,
            tags: c.tags.clone(),
        }
    }
}

impl From<&ChannelConfig> for Channel {
    fn from(c: &ChannelConfig) -> Self {
        Channel::from_config(c)
    }
}

/// Result of an automated connectivity test for a single channel.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelTestResult {
    pub channel_id: Uuid,
    pub channel_name: String,
    pub success: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub tested_at: DateTime<Utc>,
}

/// Shared channel storage with per-channel locking.
///
/// Outer `tokio::sync::RwLock<HashMap<Uuid, _>>` is held only briefly for
/// HashMap lookups and iteration. Each channel value is wrapped in an inner
/// `parking_lot::RwLock<Channel>` so that circuit-breaker mutations, latency
/// updates, and other per-channel writes do not block routing reads of other
/// channels. The inner parking_lot lock is safe because no `.await` is held
/// while the guard is live.
pub type SharedChannels =
    Arc<tokio::sync::RwLock<HashMap<Uuid, Arc<parking_lot::RwLock<Channel>>>>>;

/// Simple glob pattern matching: `*` matches any sequence, `?` matches one char.
/// Case-sensitive. No regex — intentionally simple.
pub(crate) fn matches_glob(pattern: &str, text: &str) -> bool {
    fn match_helper(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            b'*' => {
                // Try matching zero or more characters
                if match_helper(&p[1..], t) {
                    return true;
                }
                if !t.is_empty() && match_helper(p, &t[1..]) {
                    return true;
                }
                false
            }
            b'?' => !t.is_empty() && match_helper(&p[1..], &t[1..]),
            c => !t.is_empty() && t[0] == c && match_helper(&p[1..], &t[1..]),
        }
    }
    match_helper(pattern.as_bytes(), text.as_bytes())
}

/// Perform a basic connectivity check against a channel's base_url.
///
/// Tries `/models` then `/health` with a 5-second timeout. Any HTTP response
/// (even non-2xx) means the server is reachable — we do not validate auth here.
async fn check_channel_connectivity(base_url: &str) -> Result<(), String> {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let candidates = [format!("{base}/models"), format!("{base}/health")];
    let mut last_err = String::new();
    for url in &candidates {
        match client.get(url).send().await {
            Ok(_) => return Ok(()),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(format!("connectivity check failed: {last_err}"))
}

impl manager::ChannelManager {
    /// Run a lightweight connectivity test on a single channel.
    ///
    /// Performs an HTTP check against the channel's `base_url` and updates
    /// the channel's circuit-breaker status based on the result.
    pub async fn run_channel_test(&self, id: Uuid) -> ChannelTestResult {
        let channel = match self.get(id).await {
            Some(c) => c,
            None => {
                return ChannelTestResult {
                    channel_id: id,
                    channel_name: String::new(),
                    success: false,
                    latency_ms: None,
                    error: Some("channel not found".to_string()),
                    tested_at: Utc::now(),
                };
            }
        };

        let start = std::time::Instant::now();
        let outcome = check_channel_connectivity(&channel.base_url).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match outcome {
            Ok(()) => {
                let mut updated = channel.clone();
                updated.status = ChannelStatus::Healthy;
                updated.consecutive_failures = 0;
                updated.updated_at = Utc::now();
                self.update(id, updated).await;

                ChannelTestResult {
                    channel_id: id,
                    channel_name: channel.name.clone(),
                    success: true,
                    latency_ms: Some(latency_ms),
                    error: None,
                    tested_at: Utc::now(),
                }
            }
            Err(e) => {
                self.mark_circuit_open(id).await;
                ChannelTestResult {
                    channel_id: id,
                    channel_name: channel.name.clone(),
                    success: false,
                    latency_ms: Some(latency_ms),
                    error: Some(e),
                    tested_at: Utc::now(),
                }
            }
        }
    }

    /// Run connectivity tests on all enabled channels sequentially.
    pub async fn run_all_tests(&self) -> Vec<ChannelTestResult> {
        let channels = self.list().await;
        let mut results = Vec::with_capacity(channels.len());
        for ch in channels {
            if ch.enabled {
                results.push(self.run_channel_test(ch.id).await);
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelStatus, Credential, CredentialType};
    use chrono::Utc;

    /// Build a minimal Channel with default fields for testing.
    fn test_channel() -> Channel {
        Channel {
            id: Uuid::new_v4(),
            name: "test-channel".to_string(),
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
            tags: vec![],
        }
    }

    #[test]
    fn test_excluded_models_empty_allows_all() {
        let ch = test_channel();
        assert!(!ch.is_model_excluded("gpt-4"));
        assert!(!ch.is_model_excluded("anything"));
    }

    #[test]
    fn test_excluded_models_wildcard() {
        let ch = Channel {
            excluded_models: vec!["*-preview".to_string(), "*flash*".to_string()],
            ..test_channel()
        };
        assert!(ch.is_model_excluded("gemini-2.5-preview"));
        assert!(ch.is_model_excluded("gemini-flash"));
        assert!(ch.is_model_excluded("gemini-2.5-flash-preview"));
        assert!(!ch.is_model_excluded("gpt-4"));
    }

    #[test]
    fn test_excluded_models_exact() {
        let ch = Channel {
            excluded_models: vec!["gpt-3.5-turbo".to_string()],
            ..test_channel()
        };
        assert!(ch.is_model_excluded("gpt-3.5-turbo"));
        assert!(!ch.is_model_excluded("gpt-4"));
        assert!(!ch.is_model_excluded("gpt-3.5-turbo-16k"));
    }

    #[test]
    fn test_matches_glob_question_mark() {
        assert!(matches_glob("gpt-?a", "gpt-4a"));
        assert!(!matches_glob("gpt-?a", "gpt-4ab"));
        assert!(!matches_glob("gpt-?a", "gpt-a"));
    }

    #[test]
    fn test_matches_glob_star() {
        assert!(matches_glob("*", "anything"));
        assert!(matches_glob("gemini-*", "gemini-2.5-flash"));
        assert!(!matches_glob("gemini-*", "claude-3"));
        assert!(matches_glob("*-preview", "gemini-2.5-preview"));
    }

    #[test]
    fn test_model_cooldown_blocks_specific_model() {
        let mut ch = test_channel();
        ch.mark_model_rate_limited("gpt-4", Some(120)); // 2 minutes
        assert!(ch.is_model_in_cooldown("gpt-4"));
        assert!(!ch.is_model_in_cooldown("gpt-3.5-turbo"));
    }

    #[test]
    fn test_model_cooldown_expires() {
        let mut ch = test_channel();
        // Set cooldown with already-expired timestamp
        let expiry = Utc::now() - chrono::Duration::seconds(1);
        ch.model_cooldowns.insert("gpt-4".to_string(), expiry);
        assert!(!ch.is_model_in_cooldown("gpt-4"));
    }

    #[test]
    fn test_clean_expired_model_cooldowns() {
        let mut ch = test_channel();
        let past = Utc::now() - chrono::Duration::seconds(1);
        let future = Utc::now() + chrono::Duration::minutes(5);
        ch.model_cooldowns.insert("expired-model".to_string(), past);
        ch.model_cooldowns
            .insert("active-model".to_string(), future);
        ch.clean_expired_model_cooldowns();
        assert!(!ch.model_cooldowns.contains_key("expired-model"));
        assert!(ch.model_cooldowns.contains_key("active-model"));
    }

    // -- Channel auto-test helpers -------------------------------------------

    fn make_test_manager() -> manager::ChannelManager {
        use crate::config::{AppConfig, GatewayConfig};
        use crate::credential::file_store::FileCredentialStore;

        let config = AppConfig {
            gateway: GatewayConfig {
                circuit_breaker_minutes: 5,
                ..Default::default()
            },
            ..Default::default()
        };
        let credential_store: crate::credential::SharedCredentialStore =
            Arc::new(FileCredentialStore::new());
        manager::ChannelManager::new(&config, credential_store)
    }

    // -- Channel auto-test tests ---------------------------------------------

    #[tokio::test]
    async fn channel_test_returns_error_for_unknown_channel() {
        let manager = make_test_manager();
        let unknown_id = Uuid::new_v4();

        let result = manager.run_channel_test(unknown_id).await;

        assert!(!result.success);
        assert_eq!(result.channel_id, unknown_id);
        assert!(result.latency_ms.is_none());
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not found")),
            "error should mention 'not found': {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn channel_test_updates_status_on_success() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Mock HTTP server — any GET responds 200 OK.
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let manager = make_test_manager();
        let id = Uuid::new_v4();
        let mut channel = test_channel();
        channel.id = id;
        channel.base_url = mock_server.uri();
        channel.status = ChannelStatus::HalfOpen;
        manager.create(channel).await;

        let result = manager.run_channel_test(id).await;
        assert!(
            result.success,
            "test should succeed: {:?}",
            result.error
        );
        assert!(result.latency_ms.is_some());

        let updated = manager.get(id).await.expect("channel exists");
        assert_eq!(updated.status, ChannelStatus::Healthy);
        assert_eq!(updated.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn channel_test_updates_status_on_failure() {
        let manager = make_test_manager();
        let id = Uuid::new_v4();
        let mut channel = test_channel();
        channel.id = id;
        // Port 1 on loopback — connection refused immediately.
        channel.base_url = "http://127.0.0.1:1".to_string();
        channel.status = ChannelStatus::Healthy;
        manager.create(channel).await;

        let result = manager.run_channel_test(id).await;

        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.latency_ms.is_some());

        let updated = manager.get(id).await.expect("channel exists");
        assert_eq!(updated.status, ChannelStatus::CircuitOpen);
    }
}
