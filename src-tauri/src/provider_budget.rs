//! Per-provider daily/monthly budget limits.
//!
//! Tracks spend per provider name (e.g. "openai", "anthropic") and enforces
//! configurable daily/monthly caps. The spend tracking mirrors the
//! [`VirtualKeySpend`](crate::virtual_key::VirtualKeySpend) pattern with
//! auto-reset on date/month change.

use std::sync::Arc;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::persisted_store::PersistedStore;
use crate::virtual_key::{DailySpend, MonthlySpend};

/// Budget configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderBudgetConfig {
    /// Daily budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    /// Monthly budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
}

/// Spend tracking for a provider (mirrors VirtualKeySpend pattern).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderSpend {
    #[serde(default)]
    pub today: DailySpend,
    #[serde(default)]
    pub this_month: MonthlySpend,
    #[serde(default)]
    pub total_cents: u64,
}

pub struct ProviderBudgetStore {
    /// Budget configs keyed by provider name. In-memory only; populated from
    /// GatewayConfig at startup and via admin API at runtime.
    configs: tokio::sync::RwLock<std::collections::HashMap<String, ProviderBudgetConfig>>,
    /// Spend tracking persisted to disk so limits survive restarts.
    spend: PersistedStore<String, ProviderSpend>,
}

pub type SharedProviderBudgetStore = Arc<ProviderBudgetStore>;

impl ProviderBudgetStore {
    pub fn new() -> Self {
        Self {
            configs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            spend: PersistedStore::new(persistence_path()),
        }
    }

    /// Construct with a custom persistence path (for testing).
    pub fn with_store_path(path: std::path::PathBuf) -> Self {
        Self {
            configs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            spend: PersistedStore::new(path),
        }
    }

    /// Set or update the budget configuration for a provider.
    pub async fn set_budget(&self, provider: &str, config: ProviderBudgetConfig) {
        self.configs
            .write()
            .await
            .insert(provider.to_string(), config);
    }

    /// Read the budget configuration for a provider.
    pub async fn get_budget(&self, provider: &str) -> Option<ProviderBudgetConfig> {
        self.configs.read().await.get(provider).cloned()
    }

    /// List all budget configurations.
    pub async fn list_budgets(&self) -> Vec<(String, ProviderBudgetConfig)> {
        self.configs
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Remove the budget configuration for a provider.
    pub async fn delete_budget(&self, provider: &str) -> bool {
        self.configs.write().await.remove(provider).is_some()
    }

    /// Returns `true` if the provider is within budget (or has no limit).
    /// Returns `false` if the provider has exceeded its daily or monthly cap.
    pub async fn check_budget(&self, provider: &str) -> bool {
        let configs = self.configs.read().await;
        let Some(config) = configs.get(provider) else {
            return true;
        };
        if config.daily_budget_cents.is_none() && config.monthly_budget_cents.is_none() {
            return true;
        }
        drop(configs);

        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        let spend_map = self.spend.read().await;
        let current = spend_map.get(provider);
        let daily_spend = match current {
            Some(s) if s.today.date == today => s.today.cents,
            _ => 0,
        };
        let monthly_spend = match current {
            Some(s) if s.this_month.month == this_month => s.this_month.cents,
            _ => 0,
        };
        drop(spend_map);

        let config = self.configs.read().await;
        let config = match config.get(provider) {
            Some(c) => c,
            None => return true,
        };

        if let Some(daily_limit) = config.daily_budget_cents {
            if daily_spend >= daily_limit {
                return false;
            }
        }
        if let Some(monthly_limit) = config.monthly_budget_cents {
            if monthly_spend >= monthly_limit {
                return false;
            }
        }
        true
    }

    /// Add spend for a provider after a request completes. Stale spend
    /// periods roll over before the new spend is added.
    pub async fn accumulate_spend(&self, provider: &str, cents: u64) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        let mut spend_map = self.spend.write().await;
        let entry = spend_map.entry(provider.to_string()).or_default();
        if entry.today.date != today {
            entry.today = DailySpend {
                date: today,
                cents: 0,
            };
        }
        if entry.this_month.month != this_month {
            entry.this_month = MonthlySpend {
                month: this_month,
                cents: 0,
            };
        }
        entry.today.cents += cents;
        entry.this_month.cents += cents;
        entry.total_cents += cents;
    }

    /// List all providers with their current spend.
    pub async fn list_spend(&self) -> Vec<(String, ProviderSpend)> {
        self.spend
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Get spend for a single provider.
    pub async fn get_spend(&self, provider: &str) -> Option<ProviderSpend> {
        self.spend.read().await.get(provider).cloned()
    }

    /// Persist spend data to disk.
    pub async fn persist(&self) -> anyhow::Result<()> {
        self.spend.persist().await;
        Ok(())
    }

    /// Load spend data from disk.
    pub async fn load(&self) -> anyhow::Result<()> {
        self.spend.load().await;
        Ok(())
    }

    /// Synchronous persist for shutdown path.
    pub fn persist_sync(&self) {
        self.spend.persist_sync();
    }
}

impl Default for ProviderBudgetStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the canonical provider-budget persistence path.
pub fn persistence_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch")
        .join("provider_budgets.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_store() -> ProviderBudgetStore {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-pb-test-{}.json",
            Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);
        ProviderBudgetStore::with_store_path(path)
    }

    #[tokio::test]
    async fn check_budget_returns_true_when_no_limit_set() {
        let store = test_store();
        assert!(store.check_budget("openai").await);
    }

    #[tokio::test]
    async fn check_budget_returns_true_when_budget_set_but_no_spend() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(100),
                    monthly_budget_cents: Some(1000),
                },
            )
            .await;
        assert!(store.check_budget("openai").await);
    }

    #[tokio::test]
    async fn check_budget_returns_false_when_daily_exceeded() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(100),
                    monthly_budget_cents: None,
                },
            )
            .await;
        store.accumulate_spend("openai", 100).await;
        assert!(!store.check_budget("openai").await);
    }

    #[tokio::test]
    async fn check_budget_returns_false_when_monthly_exceeded() {
        let store = test_store();
        store
            .set_budget(
                "anthropic",
                ProviderBudgetConfig {
                    daily_budget_cents: None,
                    monthly_budget_cents: Some(500),
                },
            )
            .await;
        store.accumulate_spend("anthropic", 500).await;
        assert!(!store.check_budget("anthropic").await);
    }

    #[tokio::test]
    async fn check_budget_resets_on_new_day() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(100),
                    monthly_budget_cents: Some(10_000),
                },
            )
            .await;

        // Manually backdate spend to a stale day
        let mut spend_map = store.spend.write().await;
        spend_map.insert(
            "openai".to_string(),
            ProviderSpend {
                today: DailySpend {
                    date: "1999-01-01".to_string(),
                    cents: 999,
                },
                this_month: MonthlySpend {
                    month: Local::now().format("%Y-%m").to_string(),
                    cents: 999,
                },
                total_cents: 999,
            },
        );
        drop(spend_map);

        // Budget check should see daily as reset (stale date → 0)
        assert!(
            store.check_budget("openai").await,
            "daily should be treated as zero on a new day"
        );
    }

    #[tokio::test]
    async fn check_budget_resets_on_new_month() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(10_000),
                    monthly_budget_cents: Some(500),
                },
            )
            .await;

        // Manually backdate monthly spend to a stale month
        let mut spend_map = store.spend.write().await;
        spend_map.insert(
            "openai".to_string(),
            ProviderSpend {
                today: DailySpend {
                    date: Local::now().format("%Y-%m-%d").to_string(),
                    cents: 100,
                },
                this_month: MonthlySpend {
                    month: "1999-01".to_string(),
                    cents: 999,
                },
                total_cents: 999,
            },
        );
        drop(spend_map);

        // Monthly is stale → treated as 0, within limit
        assert!(
            store.check_budget("openai").await,
            "monthly should be treated as zero in a new month"
        );
    }

    #[tokio::test]
    async fn accumulate_spend_tracks_correctly() {
        let store = test_store();
        store.accumulate_spend("openai", 50).await;
        store.accumulate_spend("openai", 25).await;

        let spend = store.get_spend("openai").await.unwrap();
        assert_eq!(spend.today.cents, 75);
        assert_eq!(spend.this_month.cents, 75);
        assert_eq!(spend.total_cents, 75);
    }

    #[tokio::test]
    async fn accumulate_spend_handles_unknown_provider() {
        let store = test_store();
        // Accumulating for a provider with no budget config should still track spend
        store.accumulate_spend("gemini", 10).await;
        let spend = store.get_spend("gemini").await;
        assert!(
            spend.is_some(),
            "spend should be tracked even without config"
        );
        assert_eq!(spend.unwrap().total_cents, 10);
    }

    #[tokio::test]
    async fn accumulate_spend_resets_stale_periods() {
        let store = test_store();

        // Manually backdate spend
        let mut spend_map = store.spend.write().await;
        spend_map.insert(
            "openai".to_string(),
            ProviderSpend {
                today: DailySpend {
                    date: "1999-01-01".to_string(),
                    cents: 999,
                },
                this_month: MonthlySpend {
                    month: "1999-01".to_string(),
                    cents: 999,
                },
                total_cents: 999,
            },
        );
        drop(spend_map);

        store.accumulate_spend("openai", 10).await;
        let spend = store.get_spend("openai").await.unwrap();
        assert_eq!(spend.today.cents, 10, "daily should reset");
        assert_eq!(spend.this_month.cents, 10, "monthly should reset");
        assert_eq!(spend.total_cents, 999 + 10, "total accumulates");
    }

    #[tokio::test]
    async fn set_and_get_budget() {
        let store = test_store();
        let config = ProviderBudgetConfig {
            daily_budget_cents: Some(200),
            monthly_budget_cents: Some(2000),
        };
        store.set_budget("openai", config.clone()).await;
        let fetched = store.get_budget("openai").await.unwrap();
        assert_eq!(fetched.daily_budget_cents, Some(200));
        assert_eq!(fetched.monthly_budget_cents, Some(2000));
    }

    #[tokio::test]
    async fn delete_budget_removes_config() {
        let store = test_store();
        store
            .set_budget("openai", ProviderBudgetConfig::default())
            .await;
        assert!(store.delete_budget("openai").await);
        assert!(store.get_budget("openai").await.is_none());
        assert!(!store.delete_budget("openai").await);
    }

    #[tokio::test]
    async fn check_budget_allows_exact_boundary() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(100),
                    monthly_budget_cents: Some(1000),
                },
            )
            .await;
        store.accumulate_spend("openai", 99).await;
        assert!(store.check_budget("openai").await);
        store.accumulate_spend("openai", 1).await;
        // Now at 100, which is >= 100 → exceeded
        assert!(!store.check_budget("openai").await);
    }

    #[tokio::test]
    async fn list_budgets_returns_all() {
        let store = test_store();
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(100),
                    monthly_budget_cents: None,
                },
            )
            .await;
        store
            .set_budget(
                "anthropic",
                ProviderBudgetConfig {
                    daily_budget_cents: None,
                    monthly_budget_cents: Some(5000),
                },
            )
            .await;
        let budgets = store.list_budgets().await;
        assert_eq!(budgets.len(), 2);
    }

    #[tokio::test]
    async fn persist_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-pb-test-{}.json",
            Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        let store = ProviderBudgetStore::with_store_path(path.clone());
        store
            .set_budget(
                "openai",
                ProviderBudgetConfig {
                    daily_budget_cents: Some(500),
                    monthly_budget_cents: Some(5000),
                },
            )
            .await;
        store.accumulate_spend("openai", 250).await;
        store.persist().await.unwrap();
        assert!(path.exists());

        let store2 = ProviderBudgetStore::with_store_path(path.clone());
        store2.load().await.unwrap();
        let spend = store2.get_spend("openai").await.unwrap();
        assert_eq!(spend.total_cents, 250);

        let _ = std::fs::remove_file(&path);
    }
}
