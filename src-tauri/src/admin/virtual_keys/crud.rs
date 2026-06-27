use crate::admin::ApiResponse;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

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

// ─── Pagination ────────────────────────────────────────

/// Maximum number of keys returned in a single page.
pub const MAX_PAGE_LIMIT: u32 = 200;
/// Default page size when neither `page` nor `limit` is provided.
pub const DEFAULT_PAGE_LIMIT: u32 = 50;

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

// ─── Endpoints ─────────────────────────────────────────

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
    super::persist_virtual_keys(&state).await;
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
    super::persist_virtual_keys(&state).await;
    Ok(Json(ApiResponse::ok(VirtualKeyResponse::from(&updated))))
}

/// DELETE /api/virtual-keys/:id -- remove a virtual key permanently.
pub async fn delete_virtual_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    if state.billing.virtual_key_store.delete(id).await {
        super::persist_virtual_keys(&state).await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Virtual key not found")
    }
}
