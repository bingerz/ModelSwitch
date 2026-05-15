pub mod manager;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    OpenAI,
    Anthropic,
    DeepSeek,
    Gemini,
    OpenRouter,
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
        true
    }

    pub fn map_model(&self, requested_model: &str) -> String {
        self.model_mapping
            .get(requested_model)
            .cloned()
            .unwrap_or_else(|| requested_model.to_string())
    }

    /// Calculate cost from real token counts.
    /// Uses input_cost_per_mtok/output_cost_per_mtok if set, falls back to cost_per_token.
    pub fn calculate_cost(&self, input_tokens: Option<u64>, output_tokens: Option<u64>) -> Option<f64> {
        if input_tokens.is_none() && output_tokens.is_none() {
            return None;
        }

        let in_tok = input_tokens.unwrap_or(0) as f64;
        let out_tok = output_tokens.unwrap_or(0) as f64;

        if self.input_cost_per_mtok.is_some() || self.output_cost_per_mtok.is_some() {
            let in_rate = self.input_cost_per_mtok.unwrap_or(0.0);
            let out_rate = self.output_cost_per_mtok.unwrap_or(0.0);
            Some(in_tok / 1_000_000.0 * in_rate + out_tok / 1_000_000.0 * out_rate)
        } else if let Some(rate_per_1k) = self.cost_per_token {
            Some((in_tok + out_tok) / 1000.0 * rate_per_1k)
        } else {
            None
        }
    }

    pub fn mark_circuit_open(&mut self, duration_mins: u64) {
        self.status = ChannelStatus::CircuitOpen;
        self.circuit_open_until = Some(Utc::now() + chrono::Duration::minutes(duration_mins as i64));
        self.updated_at = Utc::now();
    }

    pub fn recover_if_expired(&mut self) {
        if self.status == ChannelStatus::CircuitOpen {
            if let Some(until) = self.circuit_open_until {
                if Utc::now() >= until {
                    self.status = ChannelStatus::Healthy;
                    self.circuit_open_until = None;
                    self.updated_at = Utc::now();
                }
            }
        }
    }
}

pub type SharedChannels = Arc<RwLock<Vec<Channel>>>;
