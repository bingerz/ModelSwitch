use crate::admin::ApiResponse;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

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

    super::persist_virtual_keys(&state).await;
    Ok(Json(ApiResponse::ok(created)))
}
