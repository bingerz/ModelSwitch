use chrono::Local;
use uuid::Uuid;

use crate::virtual_key::key::{DailySpend, MonthlySpend};
use crate::virtual_key::store::VirtualKeyStore;

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

impl VirtualKeyStore {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_key::key::*;

    #[tokio::test]
    async fn accumulate_spend_updates_daily_and_monthly() {
        let store = VirtualKeyStore::new();
        let (vk, _plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
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
        let (mut vk, _plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
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

    #[tokio::test]
    async fn reserve_spend_charges_budgeted_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
        let (vk, _) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let result = store.reserve_spend(vk.id, 10).await;
        assert!(matches!(result, ReserveResult::NoBudget));
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.spend.total_cents, 0);
    }

    #[tokio::test]
    async fn reserve_spend_skips_disabled_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store
            .update(
                vk.id,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
        assert_eq!(
            fetched.spend.today.cents, 10,
            "daily should reset before reserve"
        );
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(10_000),
                Some(500),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
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
}
