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
        }
    }
}

impl From<&ChannelConfig> for Channel {
    fn from(c: &ChannelConfig) -> Self {
        Channel::from_config(c)
    }
}

/// Shared channel storage with per-channel locking.
///
/// Outer `tokio::sync::RwLock<HashMap<Uuid, _>>` is held only briefly for
/// HashMap lookups and iteration. Each channel value is wrapped in an inner
/// `std::sync::RwLock<Channel>` so that circuit-breaker mutations, latency
/// updates, and other per-channel writes do not block routing reads of other
/// channels. The inner std lock is safe because no `.await` is held while the
/// guard is live.
pub type SharedChannels =
    Arc<tokio::sync::RwLock<HashMap<Uuid, Arc<std::sync::RwLock<Channel>>>>>;
