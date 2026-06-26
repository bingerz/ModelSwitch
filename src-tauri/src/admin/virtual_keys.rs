use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::ApiResponse;

// ─── Types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateVirtualKeyRequest {
    pub name: String,
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub denied_models: Vec<String>,
    #[serde(default)]
    pub allowed_ips: Option<Vec<String>>,
    /// Per-key requests-per-minute limit.
    #[serde(default)]
    pub rpm_limit: Option<u32>,
    /// Per-key tokens-per-minute limit.
    #[serde(default)]
    pub tpm_limit: Option<u32>,
    /// Optional expiry timestamp (RFC 3339). When set, the key becomes
    /// invalid after this time.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional department/group label for spend aggregation.
    #[serde(default)]
    pub group: Option<String>,
}

/// Request body for `POST /api/virtual-keys/batch`.
///
/// Generates `count` keys with names formatted as `{name_prefix}-001`,
/// `{name_prefix}-002`, etc. All generated keys share the same budget,
/// model, and IP configuration.
#[derive(Debug, Deserialize)]
pub struct BatchCreateVirtualKeyRequest {
    /// Number of keys to generate (1–500).
    pub count: u32,
    /// Name prefix for the generated keys. Each key gets a zero-padded
    /// suffix appended (e.g. `team-001`).
    pub name_prefix: String,
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    /// Shared IP allowlist applied to every generated key.
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    /// Per-key RPM limit applied to every generated key.
    #[serde(default)]
    pub rpm_limit: Option<u32>,
    /// Per-key TPM limit applied to every generated key.
    #[serde(default)]
    pub tpm_limit: Option<u32>,
    /// Optional expiry timestamp applied to every generated key.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional department/group label applied to every generated key.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateVirtualKeyRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub daily_budget_cents: Option<Option<u64>>,
    #[serde(default)]
    pub monthly_budget_cents: Option<Option<u64>>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub allowed_models: Option<Option<Vec<String>>>,
    #[serde(default)]
    pub denied_models: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_ips: Option<Vec<String>>,
    #[serde(default)]
    pub rpm_limit: Option<Option<u32>>,
    #[serde(default)]
    pub tpm_limit: Option<Option<u32>>,
    #[serde(default)]
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    #[serde(default)]
    pub group: Option<Option<String>>,
}

/// Response shape for the list endpoint -- never exposes `key_hash`.
#[derive(Debug, Serialize)]
pub struct VirtualKeyResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub daily_budget_cents: Option<u64>,
    pub monthly_budget_cents: Option<u64>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub spend: crate::virtual_key::VirtualKeySpend,
    pub allowed_models: Option<Vec<String>>,
    pub denied_models: Vec<String>,
    pub allowed_ips: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpm_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl From<&crate::virtual_key::VirtualKey> for VirtualKeyResponse {
    fn from(k: &crate::virtual_key::VirtualKey) -> Self {
        Self {
            id: k.id,
            name: k.name.clone(),
            key_prefix: k.key_prefix.clone(),
            daily_budget_cents: k.daily_budget_cents,
            monthly_budget_cents: k.monthly_budget_cents,
            enabled: k.enabled,
            created_at: k.created_at,
            spend: k.spend.clone(),
            allowed_models: k.allowed_models.clone(),
            denied_models: k.denied_models.clone(),
            allowed_ips: k.allowed_ips.clone(),
            rpm_limit: k.rpm_limit,
            tpm_limit: k.tpm_limit,
            expires_at: k.expires_at,
            group: k.group.clone(),
        }
    }
}

/// Persist virtual keys to disk. Logs a warning on failure so that one bad
/// write does not crash an otherwise-successful CRUD call.
async fn persist_virtual_keys(state: &Arc<AppState>) {
    if let Err(e) = state.billing.virtual_key_store.persist().await {
        tracing::warn!(error = %e, "Failed to persist virtual keys");
    }
}

// ─── Endpoints ─────────────────────────────────────────

/// Maximum number of keys returned in a single page.
const MAX_PAGE_LIMIT: u32 = 200;
/// Default page size when neither `page` nor `limit` is provided.
const DEFAULT_PAGE_LIMIT: u32 = 50;

fn default_page() -> u32 {
    1
}
fn default_page_limit() -> u32 {
    DEFAULT_PAGE_LIMIT
}

/// Query parameters for `GET /api/virtual-keys` (pagination + search).
#[derive(Debug, Deserialize)]
pub struct ListVirtualKeysParams {
    /// 1-based page number (default 1).
    #[serde(default = "default_page")]
    pub page: u32,
    /// Maximum keys per page (default 50, capped at 200).
    #[serde(default = "default_page_limit")]
    pub limit: u32,
    /// Optional case-insensitive name prefix filter.
    #[serde(default)]
    pub search: Option<String>,
    /// Optional exact group filter (matches the `group` field on keys).
    #[serde(default)]
    pub group: Option<String>,
}

/// Paginated response payload for `GET /api/virtual-keys`.
#[derive(Debug, Serialize)]
pub struct ListVirtualKeysResponse {
    pub data: Vec<VirtualKeyResponse>,
    pub total: usize,
    pub page: u32,
    pub limit: u32,
}

/// GET /api/virtual-keys -- list keys with pagination and optional search.
///
/// Query params:
/// - `page` (default 1, 1-based)
/// - `limit` (default 50, max 200)
/// - `search` (optional, case-insensitive name prefix match)
///
/// Never returns `key_hash`. Plaintext is only returned once at creation time.
pub async fn list_virtual_keys(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListVirtualKeysParams>,
) -> Json<ApiResponse<ListVirtualKeysResponse>> {
    let page = params.page.max(1);
    let limit = params.limit.clamp(1, MAX_PAGE_LIMIT);

    let mut keys = state.billing.virtual_key_store.list().await;

    // Optional search: case-insensitive prefix match on name.
    if let Some(ref search) = params.search {
        let lower = search.to_lowercase();
        keys.retain(|k| k.name.to_lowercase().starts_with(&lower));
    }

    // Optional group filter: exact match on the group label.
    if let Some(ref group) = params.group {
        keys.retain(|k| k.group.as_deref() == Some(group.as_str()));
    }

    let total = keys.len();
    let offset = ((page - 1) as usize * limit as usize).min(total);
    let paged: Vec<VirtualKeyResponse> = keys
        .iter()
        .skip(offset)
        .take(limit as usize)
        .map(VirtualKeyResponse::from)
        .collect();

    Json(ApiResponse::ok(ListVirtualKeysResponse {
        data: paged,
        total,
        page,
        limit,
    }))
}

/// POST /api/virtual-keys -- create a new virtual key.
/// Returns the plaintext key in the response body. This is the only time the
/// plaintext is visible; subsequent calls only ever see `key_prefix`.
pub async fn create_virtual_key(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateVirtualKeyRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    if req.name.trim().is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "Name is required"));
    }
    if let Some(d) = req.daily_budget_cents {
        if let Some(m) = req.monthly_budget_cents {
            if m < d {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Monthly budget must be >= daily budget",
                ));
            }
        }
    }
    let (vk, plaintext) = state
        .billing
        .virtual_key_store
        .create(
            req.name,
            req.daily_budget_cents,
            req.monthly_budget_cents,
            req.allowed_models,
            req.denied_models,
            req.allowed_ips.unwrap_or_default(),
            req.rpm_limit,
            req.tpm_limit,
            req.expires_at,
            req.group,
        )
        .await;
    persist_virtual_keys(&state).await;
    let body = serde_json::json!({
        "id": vk.id,
        "name": vk.name,
        "key": plaintext,
        "key_prefix": vk.key_prefix,
        "daily_budget_cents": vk.daily_budget_cents,
        "monthly_budget_cents": vk.monthly_budget_cents,
        "enabled": vk.enabled,
        "created_at": vk.created_at,
        "spend": vk.spend,
        "allowed_models": vk.allowed_models,
        "denied_models": vk.denied_models,
        "allowed_ips": vk.allowed_ips,
        "rpm_limit": vk.rpm_limit,
        "tpm_limit": vk.tpm_limit,
        "expires_at": vk.expires_at,
        "group": vk.group,
    });
    Ok(Json(ApiResponse::ok(body)))
}

/// POST /api/virtual-keys/batch -- generate `count` keys with a shared
/// configuration. Names follow the pattern `{name_prefix}-001`,
/// `{name_prefix}-002`, etc. Returns the plaintext key for each created key.
///
/// This is the only endpoint that returns multiple plaintext keys; callers
/// must store them immediately as they will not be shown again.
pub async fn batch_create_virtual_keys(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchCreateVirtualKeyRequest>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>, axum::response::Response> {
    if req.name_prefix.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "name_prefix is required",
        ));
    }
    if req.count == 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "count must be at least 1",
        ));
    }
    const MAX_BATCH: u32 = 500;
    if req.count > MAX_BATCH {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("count must not exceed {MAX_BATCH}"),
        ));
    }
    if let Some(d) = req.daily_budget_cents {
        if let Some(m) = req.monthly_budget_cents {
            if m < d {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Monthly budget must be >= daily budget",
                ));
            }
        }
    }

    let width = format!("{}", req.count).len().max(3);
    let mut created: Vec<serde_json::Value> = Vec::with_capacity(req.count as usize);

    for i in 1..=req.count {
        let name = format!("{}-{:0width$}", req.name_prefix, i, width = width);
        let (vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                name,
                req.daily_budget_cents,
                req.monthly_budget_cents,
                req.allowed_models.clone(),
                Vec::new(),
                req.allowed_ips.clone(),
                req.rpm_limit,
                req.tpm_limit,
                req.expires_at,
                req.group.clone(),
            )
            .await;
        created.push(serde_json::json!({
            "id": vk.id,
            "name": vk.name,
            "key": plaintext,
            "key_prefix": vk.key_prefix,
            "daily_budget_cents": vk.daily_budget_cents,
            "monthly_budget_cents": vk.monthly_budget_cents,
            "enabled": vk.enabled,
            "created_at": vk.created_at,
            "spend": vk.spend,
            "allowed_models": vk.allowed_models,
            "denied_models": vk.denied_models,
            "allowed_ips": vk.allowed_ips,
            "rpm_limit": vk.rpm_limit,
            "tpm_limit": vk.tpm_limit,
            "expires_at": vk.expires_at,
            "group": vk.group,
        }));
    }

    persist_virtual_keys(&state).await;
    Ok(Json(ApiResponse::ok(created)))
}

/// PUT /api/virtual-keys/:id -- update fields on a virtual key.
pub async fn update_virtual_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateVirtualKeyRequest>,
) -> Result<Json<ApiResponse<VirtualKeyResponse>>, axum::response::Response> {
    let updated = state
        .billing
        .virtual_key_store
        .update(
            id,
            req.name,
            req.daily_budget_cents,
            req.monthly_budget_cents,
            req.enabled,
            req.allowed_models,
            req.denied_models,
            req.allowed_ips,
            req.rpm_limit,
            req.tpm_limit,
            req.expires_at,
            req.group,
        )
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Virtual key not found"))?;
    persist_virtual_keys(&state).await;
    Ok(Json(ApiResponse::ok(VirtualKeyResponse::from(&updated))))
}

/// DELETE /api/virtual-keys/:id -- remove a virtual key permanently.
pub async fn delete_virtual_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    if state.billing.virtual_key_store.delete(id).await {
        persist_virtual_keys(&state).await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Virtual key not found")
    }
}

// ─── Group Summary ─────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChannelConfig;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;
    use uuid::Uuid;

    fn list_params() -> ListVirtualKeysParams {
        ListVirtualKeysParams {
            page: 1,
            limit: DEFAULT_PAGE_LIMIT,
            search: None,
            group: None,
        }
    }

    fn make_create_req(name: &str) -> CreateVirtualKeyRequest {
        CreateVirtualKeyRequest {
            name: name.to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        }
    }

    #[tokio::test]
    async fn list_virtual_keys_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_virtual_keys(State(state), Query(list_params())).await;
        assert!(result.data.data.is_empty());
        assert_eq!(result.data.total, 0);
    }

    #[tokio::test]
    async fn create_virtual_key_succeeds() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "test-key".to_string(),
            daily_budget_cents: Some(1000),
            monthly_budget_cents: Some(30000),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = create_virtual_key(State(state), Json(req)).await;
        assert!(result.is_ok());
        let data = result.unwrap().0.data;
        assert!(data["key"].as_str().is_some());
        assert!(data["key"].as_str().unwrap().starts_with("ms-vk-"));
        assert_eq!(data["name"].as_str().unwrap(), "test-key");
    }

    #[tokio::test]
    async fn create_then_delete_virtual_key() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "delete-me".to_string(),
            daily_budget_cents: Some(500),
            monthly_budget_cents: Some(10000),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let create_result = create_virtual_key(State(state.clone()), Json(req))
            .await
            .unwrap();
        let id_str = create_result.0.data["id"].as_str().unwrap().to_string();
        let id: Uuid = id_str.parse().unwrap();

        let delete_response = delete_virtual_key(State(state.clone()), Path(id)).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_virtual_keys(State(state), Query(list_params())).await;
        assert!(list_result.data.data.is_empty());
    }

    #[tokio::test]
    async fn list_virtual_keys_paginates() {
        let state = build_test_state(vec![]);
        for i in 0..12 {
            let req = CreateVirtualKeyRequest {
                name: format!("key-{i:02}"),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: None,
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        // Page 1 with limit 5 → 5 keys, total 12.
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 5,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 5);
        assert_eq!(result.data.total, 12);
        assert_eq!(result.data.page, 1);
        assert_eq!(result.data.limit, 5);

        // Page 3 with limit 5 → only 2 keys (12 - 10).
        let params = ListVirtualKeysParams {
            page: 3,
            limit: 5,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 12);
    }

    #[tokio::test]
    async fn list_virtual_keys_search_filters_by_name_prefix() {
        let state = build_test_state(vec![]);
        for name in &["alpha-1", "alpha-2", "beta-1"] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: None,
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: Some("alpha".to_string()),
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 2);

        // Case-insensitive match.
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: Some("ALPHA".to_string()),
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
    }

    #[tokio::test]
    async fn list_virtual_keys_limit_capped_at_max() {
        let state = build_test_state(vec![]);
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 10_000,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.limit, MAX_PAGE_LIMIT);
    }

    #[tokio::test]
    async fn batch_create_generates_named_keys() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 5,
            name_prefix: "team".to_string(),
            daily_budget_cents: Some(100),
            monthly_budget_cents: Some(3000),
            allowed_models: Some(vec!["gpt-4".to_string()]),
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state.clone()), Json(req))
            .await
            .unwrap();
        let created = result.0.data;
        assert_eq!(created.len(), 5);
        assert_eq!(created[0]["name"].as_str().unwrap(), "team-001");
        assert_eq!(created[1]["name"].as_str().unwrap(), "team-002");
        assert_eq!(created[4]["name"].as_str().unwrap(), "team-005");
        // Each key has a plaintext.
        for entry in &created {
            assert!(entry["key"].as_str().unwrap().starts_with("ms-vk-"));
        }

        // Total key count in store should be 5.
        let list = list_virtual_keys(State(state), Query(list_params())).await;
        assert_eq!(list.data.total, 5);
    }

    #[tokio::test]
    async fn batch_create_rejects_zero_count() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 0,
            name_prefix: "team".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn batch_create_rejects_empty_prefix() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 3,
            name_prefix: "   ".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn batch_create_rejects_count_above_max() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 501,
            name_prefix: "team".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn groups_summary_returns_empty_when_no_keys() {
        let state = build_test_state(vec![]);
        let result = list_virtual_key_groups(State(state)).await;
        assert!(result.0.data.is_empty());
    }

    #[tokio::test]
    async fn groups_summary_aggregates_by_group() {
        let state = build_test_state(vec![]);
        // Two keys in "engineering", one in "sales", one without a group.
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("eng-2", Some("engineering")),
            ("sales-1", Some("sales")),
            ("ungrouped-1", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let result = list_virtual_key_groups(State(state)).await;
        let groups = result.0.data;
        // Only two groups should appear (ungrouped excluded).
        assert_eq!(groups.len(), 2);
        let eng = groups.iter().find(|g| g.group == "engineering").unwrap();
        assert_eq!(eng.key_count, 2);
        let sales = groups.iter().find(|g| g.group == "sales").unwrap();
        assert_eq!(sales.key_count, 1);
    }

    #[tokio::test]
    async fn create_virtual_key_with_group() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "grouped-key".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: Some("marketing".to_string()),
        };
        let result = create_virtual_key(State(state), Json(req)).await;
        assert!(result.is_ok());
        let data = result.unwrap().0.data;
        assert_eq!(data["group"].as_str().unwrap(), "marketing");
    }

    #[tokio::test]
    async fn list_with_group_filter_returns_only_matching_keys() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("eng-2", Some("engineering")),
            ("sales-1", Some("sales")),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("engineering".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 2);
        for key in &result.data.data {
            assert_eq!(key.group.as_deref(), Some("engineering"));
        }
    }

    #[tokio::test]
    async fn list_with_group_filter_returns_empty_when_no_match() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "eng-1".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: Some("engineering".to_string()),
        };
        create_virtual_key(State(state.clone()), Json(req))
            .await
            .unwrap();

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("nonexistent".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert!(result.data.data.is_empty());
        assert_eq!(result.data.total, 0);
    }

    #[tokio::test]
    async fn list_with_group_filter_ignores_keys_with_no_group() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("ungrouped-1", None),
            ("ungrouped-2", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("engineering".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 1);
        assert_eq!(result.data.total, 1);
        assert_eq!(result.data.data[0].group.as_deref(), Some("engineering"));
    }

    #[tokio::test]
    async fn list_without_group_filter_returns_all() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("sales-1", Some("sales")),
            ("ungrouped-1", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 3);
        assert_eq!(result.data.total, 3);
    }
}
