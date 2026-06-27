use crate::admin::ApiResponse;
use crate::proxy::AppState;
use axum::extract::State;
use axum::Json;
use serde::Serialize;
use std::sync::Arc;

/// A single department/group spend summary row.
#[derive(Debug, Serialize)]
pub struct GroupSummary {
    pub group: String,
    pub key_count: usize,
    pub daily_spent_cents: u64,
    pub monthly_spent_cents: u64,
}

/// GET /api/virtual-keys/groups -- aggregate key counts and spend by group.
///
/// Keys without a group are excluded from the result.
pub async fn list_virtual_key_groups(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<GroupSummary>>> {
    use std::collections::BTreeMap;

    let keys = state.billing.virtual_key_store.list().await;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let this_month = chrono::Local::now().format("%Y-%m").to_string();

    let mut buckets: BTreeMap<String, GroupSummary> = BTreeMap::new();
    for k in &keys {
        let group = match &k.group {
            Some(g) if !g.trim().is_empty() => g.clone(),
            _ => continue,
        };
        let entry = buckets.entry(group.clone()).or_insert(GroupSummary {
            group,
            key_count: 0,
            daily_spent_cents: 0,
            monthly_spent_cents: 0,
        });
        entry.key_count += 1;
        if k.spend.today.date == today {
            entry.daily_spent_cents += k.spend.today.cents;
        }
        if k.spend.this_month.month == this_month {
            entry.monthly_spent_cents += k.spend.this_month.cents;
        }
    }

    Json(ApiResponse::ok(buckets.into_values().collect()))
}
