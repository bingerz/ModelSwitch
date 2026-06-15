//! Virtual API keys with budget tracking for multi-agent spend isolation.
//!
//! A virtual key wraps the gateway's upstream credentials so that downstream
//! clients (agents, scripts, IDEs) can be issued per-key credentials with
//! their own daily/monthly spend caps. When at least one virtual key exists,
//! the proxy enforces that incoming requests carry a valid key; otherwise
//! the gateway behaves exactly as before (open proxy keyed on channel creds).

use std::sync::Arc;

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::persisted_store::PersistedStore;

/// A virtual API key with budget limits and spend tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualKey {
    pub id: Uuid,
    /// SHA-256 hex hash of the plaintext key. Plaintext is shown only at creation time.
    pub key_hash: String,
    /// Key prefix for identification (first 16 chars of plaintext, for display).
    pub key_prefix: String,
    pub name: String,
    /// Daily budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    /// Monthly budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<Utc>,
    /// Spend tracking (mutable, updated by accumulate_spend).
    #[serde(default)]
    pub spend: VirtualKeySpend,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualKeySpend {
    /// Spend today in cents, with the date it tracks.
    #[serde(default)]
    pub today: DailySpend,
    /// Spend this month in cents, with the month it tracks.
    #[serde(default)]
    pub this_month: MonthlySpend,
    /// All-time total spend in cents.
    #[serde(default)]
    pub total_cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailySpend {
    /// YYYY-MM-DD in the gateway's local timezone.
    pub date: String,
    pub cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonthlySpend {
    /// YYYY-MM in the gateway's local timezone.
    pub month: String,
    pub cents: u64,
}

impl VirtualKey {
    /// Check if this key has exceeded its budget. Stale spend periods (where
    /// the stored date/month does not match the current local date/month) are
    /// treated as zero, so a new day/month automatically resets the cap.
    pub fn is_budget_exceeded(&self) -> bool {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        let daily_spend = if self.spend.today.date == today {
            self.spend.today.cents
        } else {
            0
        };
        let monthly_spend = if self.spend.this_month.month == this_month {
            self.spend.this_month.cents
        } else {
            0
        };

        if let Some(daily_limit) = self.daily_budget_cents {
            if daily_spend >= daily_limit {
                return true;
            }
        }
        if let Some(monthly_limit) = self.monthly_budget_cents {
            if monthly_spend >= monthly_limit {
                return true;
            }
        }
        false
    }
}

/// Result of a budget reservation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveResult {
    /// No budget enforcement needed (unlimited key, disabled, or missing key).
    NoBudget,
    /// Reservation succeeded — this many cents were pre-charged.
    Reserved(u64),
    /// Budget exceeded — request should be rejected.
    Exceeded,
}

pub struct VirtualKeyStore {
    store: PersistedStore<Uuid, VirtualKey>,
}

pub type SharedVirtualKeyStore = Arc<VirtualKeyStore>;

impl VirtualKeyStore {
    pub fn new() -> Self {
        Self {
            store: PersistedStore::new(persistence_path()),
        }
    }

    /// Construct with a custom persistence path (for testing).
    pub fn with_store_path(path: std::path::PathBuf) -> Self {
        Self {
            store: PersistedStore::new(path),
        }
    }

    /// Validate a plaintext key string. Returns the [`VirtualKey`] if it is
    /// valid, enabled, and within budget. Uses constant-time comparison via
    /// the `subtle` crate to avoid leaking which prefix matched via timing.
    pub async fn validate(&self, plaintext: &str) -> Option<VirtualKey> {
        let hash = sha256_hex(plaintext);
        let hash_bytes = hash.into_bytes();
        let keys = self.store.read().await;
        for vk in keys.values() {
            let stored = vk.key_hash.as_bytes();
            // Only compare equal-length buffers via ct_eq.
            let matched = stored.len() == hash_bytes.len() && bool::from(stored.ct_eq(&hash_bytes));
            if matched {
                if !vk.enabled {
                    return None;
                }
                if vk.is_budget_exceeded() {
                    return None;
                }
                return Some(vk.clone());
            }
        }
        None
    }

    /// Create a new virtual key. Returns `(VirtualKey, plaintext_key)`.
    /// The plaintext is shown to the user once and never stored.
    pub async fn create(
        &self,
        name: String,
        daily_budget_cents: Option<u64>,
        monthly_budget_cents: Option<u64>,
    ) -> (VirtualKey, String) {
        let plaintext = format!("ms-vk-{}", Uuid::new_v4().simple());
        let hash = sha256_hex(&plaintext);
        // 16 chars covers "ms-vk-" + 9 chars of uuid — enough to identify a key in the UI.
        let prefix = plaintext[..16].to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: hash,
            key_prefix: prefix,
            name,
            daily_budget_cents,
            monthly_budget_cents,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
        };
        let vk_clone = vk.clone();
        self.store.write().await.insert(vk.id, vk);
        (vk_clone, plaintext)
    }

    pub async fn list(&self) -> Vec<VirtualKey> {
        self.store.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: Uuid) -> Option<VirtualKey> {
        self.store.read().await.get(&id).cloned()
    }

    pub async fn delete(&self, id: Uuid) -> bool {
        self.store.write().await.remove(&id).is_some()
    }

    /// Update fields on a virtual key. Each `Option<T>` field, when `Some`,
    /// replaces the stored value; `None` leaves it untouched.
    #[allow(clippy::option_option)]
    pub async fn update(
        &self,
        id: Uuid,
        name: Option<String>,
        daily_budget_cents: Option<Option<u64>>,
        monthly_budget_cents: Option<Option<u64>>,
        enabled: Option<bool>,
    ) -> Option<VirtualKey> {
        let mut keys = self.store.write().await;
        let vk = keys.get_mut(&id)?;
        if let Some(n) = name {
            vk.name = n;
        }
        if let Some(d) = daily_budget_cents {
            vk.daily_budget_cents = d;
        }
        if let Some(m) = monthly_budget_cents {
            vk.monthly_budget_cents = m;
        }
        if let Some(e) = enabled {
            vk.enabled = e;
        }
        Some(vk.clone())
    }

    /// Reserve estimated spend before a request is dispatched.
    /// This pre-charges the daily/monthly/total counters to prevent
    /// concurrent requests from exceeding the budget.
    ///
    /// Returns [`ReserveResult::NoBudget`] for unlimited, disabled, or missing keys,
    /// [`ReserveResult::Reserved(n)`] on success, or [`ReserveResult::Exceeded`] if
    /// the reservation would push daily or monthly spend past the configured limit.
    pub async fn reserve_spend(&self, key_id: Uuid, estimated_cost_cents: u64) -> ReserveResult {
        let mut keys = self.store.write().await;
        let Some(vk) = keys.get_mut(&key_id) else {
            return ReserveResult::NoBudget;
        };
        if !vk.enabled {
            return ReserveResult::NoBudget;
        }
        // Only reserve if there's a budget limit — unlimited keys don't need pre-deduction
        let has_budget = vk.daily_budget_cents.is_some() || vk.monthly_budget_cents.is_some();
        if !has_budget {
            return ReserveResult::NoBudget;
        }

        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        // Reset stale periods before reserving
        if vk.spend.today.date != today {
            vk.spend.today = DailySpend {
                date: today,
                cents: 0,
            };
        }
        if vk.spend.this_month.month != this_month {
            vk.spend.this_month = MonthlySpend {
                month: this_month,
                cents: 0,
            };
        }

        // Calculate projected spend using saturating_add to detect overflow safely.
        let new_daily = vk.spend.today.cents.saturating_add(estimated_cost_cents);
        let new_monthly = vk
            .spend
            .this_month
            .cents
            .saturating_add(estimated_cost_cents);

        // Reject if either limit would be exceeded — do NOT modify spend counters.
        if let Some(daily_limit) = vk.daily_budget_cents {
            if new_daily > daily_limit {
                return ReserveResult::Exceeded;
            }
        }
        if let Some(monthly_limit) = vk.monthly_budget_cents {
            if new_monthly > monthly_limit {
                return ReserveResult::Exceeded;
            }
        }

        vk.spend.today.cents = new_daily;
        vk.spend.this_month.cents = new_monthly;
        vk.spend.total_cents = vk.spend.total_cents.saturating_add(estimated_cost_cents);

        ReserveResult::Reserved(estimated_cost_cents)
    }

    /// Reconcile a previous reservation with the actual cost.
    /// If actual < reserved, refund the difference. If actual > reserved, charge more.
    pub async fn reconcile_spend(&self, key_id: Uuid, reserved_cents: u64, actual_cents: u64) {
        if reserved_cents == actual_cents {
            return;
        }
        let mut keys = self.store.write().await;
        let Some(vk) = keys.get_mut(&key_id) else {
            return;
        };
        let delta = actual_cents as i64 - reserved_cents as i64;
        if delta < 0 {
            // Refund
            let refund = (-delta) as u64;
            vk.spend.today.cents = vk.spend.today.cents.saturating_sub(refund);
            vk.spend.this_month.cents = vk.spend.this_month.cents.saturating_sub(refund);
            vk.spend.total_cents = vk.spend.total_cents.saturating_sub(refund);
        } else {
            // Charge more
            let extra = delta as u64;
            vk.spend.today.cents += extra;
            vk.spend.this_month.cents += extra;
            vk.spend.total_cents += extra;
        }
    }

    /// Accumulate spend for a virtual key. Called after an upstream response
    /// completes. Stale spend periods roll over before the new spend is added.
    pub async fn accumulate_spend(&self, vk_id: Uuid, estimated_cost_cents: u64) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        let mut keys = self.store.write().await;
        let Some(vk) = keys.get_mut(&vk_id) else {
            return;
        };
        if vk.spend.today.date != today {
            vk.spend.today = DailySpend {
                date: today,
                cents: 0,
            };
        }
        if vk.spend.this_month.month != this_month {
            vk.spend.this_month = MonthlySpend {
                month: this_month,
                cents: 0,
            };
        }
        vk.spend.today.cents += estimated_cost_cents;
        vk.spend.this_month.cents += estimated_cost_cents;
        vk.spend.total_cents += estimated_cost_cents;
    }

    /// Returns true if at least one virtual key is configured. The middleware
    /// uses this to decide whether enforcement is active (open proxy vs. gated).
    pub async fn has_keys(&self) -> bool {
        !self.store.read().await.is_empty()
    }

    /// Persist all keys to disk. Best-effort — errors are logged internally.
    pub async fn persist(&self) -> anyhow::Result<()> {
        self.store.persist().await;
        Ok(())
    }

    /// Load keys from disk. A missing file is treated as an empty store.
    pub async fn load(&self) -> anyhow::Result<()> {
        self.store.load().await;
        Ok(())
    }
}

impl Default for VirtualKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the lowercase hex SHA-256 digest of the input.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let bytes = hasher.finalize();
    // Manual hex encode to avoid pulling in an extra dependency.
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Resolve the canonical virtual-key persistence path under the user's
/// config dir. Falls back to a relative path if `dirs` cannot resolve.
pub fn persistence_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch")
        .join("virtual_keys.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_generates_key_with_ms_vk_prefix() {
        let store = VirtualKeyStore::new();
        let (vk, plaintext) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        assert!(plaintext.starts_with("ms-vk-"), "prefix was: {plaintext}");
        assert!(plaintext.len() > "ms-vk-".len() + 8);
        assert!(!plaintext.is_empty());
        assert_eq!(vk.name, "test");
        assert_eq!(vk.daily_budget_cents, Some(100));
        assert_eq!(vk.monthly_budget_cents, Some(1000));
        assert!(vk.enabled);
        assert_eq!(vk.key_prefix.len(), 16);
    }

    #[tokio::test]
    async fn validate_rejects_wrong_key() {
        let store = VirtualKeyStore::new();
        let _ = store.create("test".to_string(), None, None).await;
        let result = store.validate("ms-vk-wrongkey").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn validate_returns_key_when_correct() {
        let store = VirtualKeyStore::new();
        let (created, plaintext) = store.create("test".to_string(), None, None).await;
        let validated = store.validate(&plaintext).await;
        assert!(validated.is_some());
        assert_eq!(validated.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn validate_returns_none_when_disabled() {
        let store = VirtualKeyStore::new();
        let (created, plaintext) = store.create("test".to_string(), None, None).await;
        store
            .update(created.id, None, None, None, Some(false))
            .await;
        let validated = store.validate(&plaintext).await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn accumulate_spend_updates_daily_and_monthly() {
        let store = VirtualKeyStore::new();
        let (vk, _plaintext) = store.create("test".to_string(), None, None).await;
        store.accumulate_spend(vk.id, 50).await;
        store.accumulate_spend(vk.id, 25).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 75);
        assert_eq!(fetched.spend.this_month.cents, 75);
        assert_eq!(fetched.spend.total_cents, 75);
    }

    #[tokio::test]
    async fn accumulate_spend_resets_stale_periods() {
        let store = VirtualKeyStore::new();
        let (mut vk, _plaintext) = store.create("test".to_string(), None, None).await;
        // Manually backdate the spend to a stale day/month
        vk.spend.today = DailySpend {
            date: "1999-01-01".to_string(),
            cents: 999,
        };
        vk.spend.this_month = MonthlySpend {
            month: "1999-01".to_string(),
            cents: 999,
        };
        vk.spend.total_cents = 999;
        let vk_id = vk.id;
        store.store.write().await.insert(vk.id, vk);

        store.accumulate_spend(vk_id, 10).await;
        let fetched = store.get(vk_id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 10, "daily should reset");
        assert_eq!(fetched.spend.this_month.cents, 10, "monthly should reset");
        assert_eq!(
            fetched.spend.total_cents,
            999 + 10,
            "total should accumulate"
        );
        assert_ne!(fetched.spend.today.date, "1999-01-01");
    }

    #[test]
    fn is_budget_exceeded_daily() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: Some(100),
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 100,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 100,
                },
                total_cents: 100,
            },
        };
        assert!(vk.is_budget_exceeded());
    }

    #[test]
    fn is_budget_exceeded_monthly() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: Some(500),
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 50,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 500,
                },
                total_cents: 500,
            },
        };
        assert!(vk.is_budget_exceeded());
    }

    #[test]
    fn is_budget_exceeded_unlimited_when_none() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 1_000_000,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 1_000_000,
                },
                total_cents: 1_000_000,
            },
        };
        assert!(!vk.is_budget_exceeded());
    }

    #[test]
    fn sha256_hex_is_stable_and_lowercase() {
        let a = sha256_hex("hello");
        let b = sha256_hex("hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Known SHA-256 of "hello"
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn reserve_spend_charges_budgeted_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        let result = store.reserve_spend(vk.id, 10).await;
        assert!(matches!(result, ReserveResult::Reserved(n) if n == 10));
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 10);
        assert_eq!(fetched.spend.this_month.cents, 10);
        assert_eq!(fetched.spend.total_cents, 10);
    }

    #[tokio::test]
    async fn reserve_spend_skips_unlimited_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store.create("test".to_string(), None, None).await;
        let result = store.reserve_spend(vk.id, 10).await;
        assert!(matches!(result, ReserveResult::NoBudget));
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 0);
    }

    #[tokio::test]
    async fn reserve_spend_skips_disabled_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        store
            .update(vk.id, None, None, None, Some(false))
            .await;
        let result = store.reserve_spend(vk.id, 10).await;
        assert!(matches!(result, ReserveResult::NoBudget));
    }

    #[tokio::test]
    async fn reserve_spend_returns_zero_for_missing_key() {
        let store = VirtualKeyStore::new();
        let result = store.reserve_spend(Uuid::new_v4(), 10).await;
        assert!(matches!(result, ReserveResult::NoBudget));
    }

    #[tokio::test]
    async fn reserve_spend_resets_stale_periods() {
        let store = VirtualKeyStore::new();
        let (mut vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        vk.spend.today = DailySpend {
            date: "1999-01-01".to_string(),
            cents: 999,
        };
        vk.spend.this_month = MonthlySpend {
            month: "1999-01".to_string(),
            cents: 999,
        };
        vk.spend.total_cents = 999;
        let vk_id = vk.id;
        store.store.write().await.insert(vk.id, vk);

        store.reserve_spend(vk_id, 10).await;
        let fetched = store.get(vk_id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 10, "daily should reset before reserve");
        assert_eq!(
            fetched.spend.this_month.cents, 10,
            "monthly should reset before reserve"
        );
        assert_eq!(fetched.spend.total_cents, 999 + 10);
    }

    #[tokio::test]
    async fn reserve_spend_rejects_when_daily_budget_exceeded() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        // Accumulate 90 cents of spend
        store.accumulate_spend(vk.id, 90).await;
        // Try to reserve 20 cents → 90 + 20 = 110 > 100 daily limit
        let result = store.reserve_spend(vk.id, 20).await;
        assert!(matches!(result, ReserveResult::Exceeded));
        // Verify spend counters were NOT modified
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(
            fetched.spend.today.cents, 90,
            "daily spend must not change on rejected reservation"
        );
        assert_eq!(fetched.spend.this_month.cents, 90);
        assert_eq!(fetched.spend.total_cents, 90);
    }

    #[tokio::test]
    async fn reserve_spend_rejects_when_monthly_budget_exceeded() {
        let store = VirtualKeyStore::new();
        // Daily budget is large so only monthly triggers
        let (vk, _) = store
            .create("test".to_string(), Some(10_000), Some(500))
            .await;
        // Accumulate 490 cents of spend
        store.accumulate_spend(vk.id, 490).await;
        // Try to reserve 20 cents → 490 + 20 = 510 > 500 monthly limit
        let result = store.reserve_spend(vk.id, 20).await;
        assert!(matches!(result, ReserveResult::Exceeded));
        // Verify spend counters were NOT modified
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(
            fetched.spend.this_month.cents, 490,
            "monthly spend must not change on rejected reservation"
        );
        assert_eq!(fetched.spend.today.cents, 490);
        assert_eq!(fetched.spend.total_cents, 490);
    }

    #[tokio::test]
    async fn reserve_spend_allows_exact_limit_boundary() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        // Accumulate 90 cents, then reserve exactly 10 → 90 + 10 = 100, not exceeding
        store.accumulate_spend(vk.id, 90).await;
        let result = store.reserve_spend(vk.id, 10).await;
        assert!(matches!(result, ReserveResult::Reserved(n) if n == 10));
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 100);
    }

    #[tokio::test]
    async fn reconcile_spend_refunds_when_actual_less() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        store.reserve_spend(vk.id, 50).await;
        store.reconcile_spend(vk.id, 50, 20).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 20);
        assert_eq!(fetched.spend.this_month.cents, 20);
        assert_eq!(fetched.spend.total_cents, 20);
    }

    #[tokio::test]
    async fn reconcile_spend_charges_more_when_actual_greater() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        store.reserve_spend(vk.id, 20).await;
        store.reconcile_spend(vk.id, 20, 50).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 50);
        assert_eq!(fetched.spend.this_month.cents, 50);
        assert_eq!(fetched.spend.total_cents, 50);
    }

    #[tokio::test]
    async fn reconcile_spend_noop_when_equal() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        store.reserve_spend(vk.id, 30).await;
        store.reconcile_spend(vk.id, 30, 30).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 30);
    }

    #[tokio::test]
    async fn reconcile_spend_refunds_full_on_failure() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        store.reserve_spend(vk.id, 40).await;
        store.reconcile_spend(vk.id, 40, 0).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.today.cents, 0);
        assert_eq!(fetched.spend.this_month.cents, 0);
        assert_eq!(fetched.spend.total_cents, 0);
    }

    #[tokio::test]
    async fn reserve_and_reconcile_concurrent_simulation() {
        // Simulate two concurrent requests reserving against the same budget
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create("test".to_string(), Some(100), Some(1000))
            .await;
        // First request reserves 30
        store.reserve_spend(vk.id, 30).await;
        // Second request reserves 40 (should see the 30 already charged)
        store.reserve_spend(vk.id, 40).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 70);
        // First request completes with actual 25 — refund 5
        store.reconcile_spend(vk.id, 30, 25).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 65);
        // Second request completes with actual 35 — refund 5
        store.reconcile_spend(vk.id, 40, 35).await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 60);
    }

    #[tokio::test]
    async fn has_keys_reflects_state() {
        let store = VirtualKeyStore::new();
        assert!(!store.has_keys().await);
        let _ = store.create("a".to_string(), None, None).await;
        assert!(store.has_keys().await);
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store.create("a".to_string(), None, None).await;
        assert!(store.delete(vk.id).await);
        assert!(store.get(vk.id).await.is_none());
        assert!(!store.delete(vk.id).await);
    }

    #[tokio::test]
    async fn persist_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-vk-test-{}.json",
            Uuid::new_v4().simple()
        ));
        // Clean up before and after so a previous panicked run can't poison us.
        let _ = std::fs::remove_file(&path);

        let store = VirtualKeyStore::with_store_path(path.clone());
        let (vk, plaintext) = store
            .create("persisted".to_string(), Some(10), Some(100))
            .await;
        store.accumulate_spend(vk.id, 5).await;
        store.persist().await.unwrap();
        assert!(path.exists());

        let store2 = VirtualKeyStore::with_store_path(path.clone());
        store2.load().await.unwrap();
        let keys = store2.list().await;
        assert_eq!(keys.len(), 1);
        let loaded = &keys[0];
        assert_eq!(loaded.name, "persisted");
        assert_eq!(loaded.spend.total_cents, 5);
        // Plaintext should still validate after a reload
        let v = store2.validate(&plaintext).await;
        assert!(v.is_some());

        let _ = std::fs::remove_file(&path);
    }
}
