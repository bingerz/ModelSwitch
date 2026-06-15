pub mod collectors;
pub mod poller;
pub mod provider;
pub mod registry;
#[cfg(feature = "tauri")]
pub mod webview_scrape;

use crate::config::app_config_dir;
use crate::persisted_store::PersistedStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Error types for quota polling.
#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("auth failed: {0}")]
    AuthFailed(String),
    #[error("network: {0}")]
    Network(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("session expired")]
    SessionExpired,
}

/// A key-value detail item (e.g. "daily_usage" = "1.23").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaItem {
    pub label: String,
    pub value: String,
}

/// A grouped usage window (e.g. 5-hour or 7-day utilization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaGroup {
    pub window: String,
    pub utilization_pct: Option<f64>,
    pub resets_at: Option<String>,
}

/// Normalized quota information for a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaInfo {
    pub channel_id: Uuid,
    pub channel_name: String,
    pub provider: String,

    // Standardized balance fields
    pub balance: Option<f64>,
    pub limit: Option<f64>,
    pub usage: Option<f64>,
    pub remaining_tokens: Option<u64>,
    pub remaining_requests: Option<u64>,

    // Plan / subscription
    pub plan_status: Option<String>,
    pub expires_at: Option<String>,

    // Structured details
    pub items: Vec<QuotaItem>,
    pub groups: Vec<QuotaGroup>,
    pub compact_text: Option<String>,

    // Passive rate-limit data
    pub rate_limit_remaining_req: Option<u64>,
    pub rate_limit_limit_req: Option<u64>,
    pub rate_limit_remaining_tok: Option<u64>,
    pub rate_limit_limit_tok: Option<u64>,
    pub rate_limit_updated_at: Option<chrono::DateTime<chrono::Utc>>,

    // Accumulated token usage (passive from API response usage field)
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub total_cache_hit_tokens: Option<u64>,
    pub total_cache_miss_tokens: Option<u64>,
    pub total_requests_counted: Option<u64>,
    pub total_estimated_cost: Option<f64>,

    // Meta
    pub source: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
}

impl QuotaInfo {
    /// Create a minimal QuotaInfo with defaults.
    pub fn new(channel_id: Uuid, channel_name: &str, provider: &str, source: &str) -> Self {
        Self {
            channel_id,
            channel_name: channel_name.to_string(),
            provider: provider.to_string(),
            balance: None,
            limit: None,
            usage: None,
            remaining_tokens: None,
            remaining_requests: None,
            plan_status: None,
            expires_at: None,
            items: Vec::new(),
            groups: Vec::new(),
            compact_text: None,
            rate_limit_remaining_req: None,
            rate_limit_limit_req: None,
            rate_limit_remaining_tok: None,
            rate_limit_limit_tok: None,
            rate_limit_updated_at: None,
            total_input_tokens: None,
            total_output_tokens: None,
            total_cache_hit_tokens: None,
            total_cache_miss_tokens: None,
            total_requests_counted: None,
            total_estimated_cost: None,
            source: source.to_string(),
            updated_at: chrono::Utc::now(),
            error: None,
        }
    }

    /// Update rate-limit fields from passive response headers.
    pub fn update_from_headers(
        &mut self,
        remaining_req: Option<u64>,
        limit_req: Option<u64>,
        remaining_tok: Option<u64>,
        limit_tok: Option<u64>,
    ) {
        if remaining_req.is_some() {
            self.rate_limit_remaining_req = remaining_req;
        }
        if limit_req.is_some() {
            self.rate_limit_limit_req = limit_req;
        }
        if remaining_tok.is_some() {
            self.rate_limit_remaining_tok = remaining_tok;
        }
        if limit_tok.is_some() {
            self.rate_limit_limit_tok = limit_tok;
        }
        self.rate_limit_updated_at = Some(chrono::Utc::now());
        if self.source.is_empty() || self.source == "http_api" {
            self.source = "response_header".to_string();
        }
        self.updated_at = chrono::Utc::now();
    }
}

/// Shared store for quota information across all channels.
#[derive(Debug)]
pub struct QuotaStore {
    store: PersistedStore<Uuid, QuotaInfo>,
}

impl Default for QuotaStore {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaStore {
    pub fn new() -> Self {
        Self {
            store: PersistedStore::new(Self::store_path()),
        }
    }

    /// Insert or replace a QuotaInfo, preserving accumulated token usage
    /// from the existing entry (since the poller refreshes balance/quota
    /// data but does not track per-request token counts).
    pub async fn update(&self, info: QuotaInfo) {
        let mut quotas = self.store.write().await;
        let merged = if let Some(existing) = quotas.get(&info.channel_id) {
            QuotaInfo {
                // Preserve accumulated token usage from passive tracking
                total_input_tokens: existing.total_input_tokens,
                total_output_tokens: existing.total_output_tokens,
                total_cache_hit_tokens: existing.total_cache_hit_tokens,
                total_cache_miss_tokens: existing.total_cache_miss_tokens,
                total_requests_counted: existing.total_requests_counted,
                total_estimated_cost: existing.total_estimated_cost,
                // Preserve passive rate-limit data if poller doesn't supply it
                rate_limit_remaining_req: info
                    .rate_limit_remaining_req
                    .or(existing.rate_limit_remaining_req),
                rate_limit_limit_req: info.rate_limit_limit_req.or(existing.rate_limit_limit_req),
                rate_limit_remaining_tok: info
                    .rate_limit_remaining_tok
                    .or(existing.rate_limit_remaining_tok),
                rate_limit_limit_tok: info.rate_limit_limit_tok.or(existing.rate_limit_limit_tok),
                rate_limit_updated_at: info
                    .rate_limit_updated_at
                    .or(existing.rate_limit_updated_at),
                ..info
            }
        } else {
            info
        };
        quotas.insert(merged.channel_id, merged);
    }

    /// Update only the rate-limit fields (from passive header extraction).
    pub async fn update_rate_limits(
        &self,
        channel_id: Uuid,
        remaining_req: Option<u64>,
        limit_req: Option<u64>,
        remaining_tok: Option<u64>,
        limit_tok: Option<u64>,
    ) {
        let mut quotas = self.store.write().await;
        if let Some(info) = quotas.get_mut(&channel_id) {
            info.update_from_headers(remaining_req, limit_req, remaining_tok, limit_tok);
        } else {
            // Create a stub entry so the frontend still has data
            let mut info = QuotaInfo::new(channel_id, "", "", "response_header");
            info.update_from_headers(remaining_req, limit_req, remaining_tok, limit_tok);
            quotas.insert(channel_id, info);
        }
    }

    pub async fn get(&self, channel_id: Uuid) -> Option<QuotaInfo> {
        let quotas = self.store.read().await;
        quotas.get(&channel_id).cloned()
    }

    pub async fn list(&self) -> Vec<QuotaInfo> {
        let quotas = self.store.read().await;
        quotas.values().cloned().collect()
    }

    /// Remove the quota entry for a channel (e.g. when the channel is deleted).
    pub async fn delete(&self, channel_id: Uuid) {
        let mut quotas = self.store.write().await;
        quotas.remove(&channel_id);
    }

    /// Return the file path for persisted quota data.
    fn store_path() -> std::path::PathBuf {
        app_config_dir().join("quota_store.json")
    }

    /// Persist all quota data to disk (JSON). Best-effort — errors are logged.
    pub async fn persist_to_file(&self) {
        self.store.persist().await;
    }

    /// Load persisted quota data from disk. Merges into the current store —
    /// existing entries are NOT overwritten (the poller will refresh them).
    pub async fn load_from_file(&self) {
        self.store.load().await;
    }

    /// Synchronous persist for shutdown path (no async runtime needed).
    pub fn persist_sync(&self) {
        self.store.persist_sync();
    }

    /// Accumulate token usage from an API response into a channel's QuotaInfo.
    pub async fn accumulate_usage(
        &self,
        channel_id: Uuid,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_hit_tokens: Option<u64>,
        cache_miss_tokens: Option<u64>,
        estimated_cost: Option<f64>,
    ) {
        let mut quotas = self.store.write().await;
        if let Some(info) = quotas.get_mut(&channel_id) {
            if let Some(v) = input_tokens {
                info.total_input_tokens = Some(info.total_input_tokens.unwrap_or(0) + v);
            }
            if let Some(v) = output_tokens {
                info.total_output_tokens = Some(info.total_output_tokens.unwrap_or(0) + v);
            }
            if let Some(v) = cache_hit_tokens {
                info.total_cache_hit_tokens = Some(info.total_cache_hit_tokens.unwrap_or(0) + v);
            }
            if let Some(v) = cache_miss_tokens {
                info.total_cache_miss_tokens = Some(info.total_cache_miss_tokens.unwrap_or(0) + v);
            }
            info.total_requests_counted = Some(info.total_requests_counted.unwrap_or(0) + 1);
            if let Some(v) = estimated_cost {
                info.total_estimated_cost = Some(info.total_estimated_cost.unwrap_or(0.0) + v);
            }
            info.updated_at = chrono::Utc::now();
        } else {
            // Create a stub entry for this channel
            let mut info = QuotaInfo::new(channel_id, "", "", "passive");
            info.total_input_tokens = input_tokens;
            info.total_output_tokens = output_tokens;
            info.total_cache_hit_tokens = cache_hit_tokens;
            info.total_cache_miss_tokens = cache_miss_tokens;
            info.total_requests_counted = Some(1);
            info.total_estimated_cost = estimated_cost;
            quotas.insert(channel_id, info);
        }
    }
}

pub type SharedQuotaStore = Arc<QuotaStore>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accumulate_usage_additive() {
        let store = QuotaStore::new();
        let id = Uuid::new_v4();

        // First accumulation
        store
            .accumulate_usage(id, Some(100), Some(50), Some(30), Some(20), Some(0.001))
            .await;

        let info = store.get(id).await.unwrap();
        assert_eq!(info.total_input_tokens, Some(100));
        assert_eq!(info.total_output_tokens, Some(50));
        assert_eq!(info.total_cache_hit_tokens, Some(30));
        assert_eq!(info.total_cache_miss_tokens, Some(20));
        assert_eq!(info.total_requests_counted, Some(1));
        assert_eq!(info.total_estimated_cost, Some(0.001));

        // Second accumulation — should be additive
        store
            .accumulate_usage(id, Some(200), Some(100), Some(10), Some(5), Some(0.002))
            .await;

        let info = store.get(id).await.unwrap();
        assert_eq!(info.total_input_tokens, Some(300));
        assert_eq!(info.total_output_tokens, Some(150));
        assert_eq!(info.total_cache_hit_tokens, Some(40));
        assert_eq!(info.total_cache_miss_tokens, Some(25));
        assert_eq!(info.total_requests_counted, Some(2));
        assert_eq!(info.total_estimated_cost, Some(0.003));
    }

    #[tokio::test]
    async fn accumulate_usage_none_does_not_reset() {
        let store = QuotaStore::new();
        let id = Uuid::new_v4();

        // Set initial values
        store
            .accumulate_usage(id, Some(100), Some(50), Some(30), Some(20), Some(0.001))
            .await;

        // Accumulate with None fields — should not reset existing totals
        store
            .accumulate_usage(id, None, None, None, None, None)
            .await;

        let info = store.get(id).await.unwrap();
        assert_eq!(info.total_input_tokens, Some(100));
        assert_eq!(info.total_output_tokens, Some(50));
        assert_eq!(info.total_cache_hit_tokens, Some(30));
        assert_eq!(info.total_cache_miss_tokens, Some(20));
        // But request count should still increment
        assert_eq!(info.total_requests_counted, Some(2));
        // And cost should be preserved
        assert_eq!(info.total_estimated_cost, Some(0.001));
    }

    #[tokio::test]
    async fn accumulate_usage_creates_stub_for_unknown_channel() {
        let store = QuotaStore::new();
        let id = Uuid::new_v4();

        // No prior QuotaInfo exists for this channel
        assert!(store.get(id).await.is_none());

        store
            .accumulate_usage(id, Some(50), Some(25), None, None, None)
            .await;

        let info = store.get(id).await.unwrap();
        assert_eq!(info.channel_id, id);
        assert_eq!(info.total_input_tokens, Some(50));
        assert_eq!(info.total_output_tokens, Some(25));
        assert_eq!(info.total_cache_hit_tokens, None);
        assert_eq!(info.total_cache_miss_tokens, None);
        assert_eq!(info.total_requests_counted, Some(1));
        assert_eq!(info.total_estimated_cost, None);
    }

    #[tokio::test]
    async fn update_preserves_token_usage_from_existing() {
        let store = QuotaStore::new();
        let id = Uuid::new_v4();

        // Simulate passive accumulation: 500 input, 200 output tokens
        store
            .accumulate_usage(id, Some(500), Some(200), Some(100), Some(50), Some(0.005))
            .await;

        // Simulate poller refresh: fresh QuotaInfo with balance but no token data
        let fresh = QuotaInfo {
            balance: Some(42.0),
            items: vec![QuotaItem {
                label: "Balance (CNY)".into(),
                value: "42.00".into(),
            }],
            ..QuotaInfo::new(id, "TestChannel", "deepseek", "http_api")
        };
        store.update(fresh).await;

        let info = store.get(id).await.unwrap();

        // Poller data should be present
        assert_eq!(info.balance, Some(42.0));
        assert_eq!(info.items.len(), 1);
        assert_eq!(info.channel_name, "TestChannel");

        // Token usage must survive the poller overwrite
        assert_eq!(info.total_input_tokens, Some(500));
        assert_eq!(info.total_output_tokens, Some(200));
        assert_eq!(info.total_cache_hit_tokens, Some(100));
        assert_eq!(info.total_cache_miss_tokens, Some(50));
        assert_eq!(info.total_requests_counted, Some(1));
        // Cost must also survive
        assert_eq!(info.total_estimated_cost, Some(0.005));
    }
}
