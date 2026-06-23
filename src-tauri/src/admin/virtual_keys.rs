use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
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

/// GET /api/virtual-keys -- list all keys with current spend.
/// Never returns `key_hash`. Plaintext is only returned once at creation time.
pub async fn list_virtual_keys(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<VirtualKeyResponse>>> {
    let keys = state.billing.virtual_key_store.list().await;
    Json(ApiResponse::ok(
        keys.iter().map(VirtualKeyResponse::from).collect(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChannelConfig;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;
    use uuid::Uuid;

    #[tokio::test]
    async fn list_virtual_keys_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_virtual_keys(State(state)).await;
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn create_virtual_key_succeeds() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "test-key".to_string(),
            daily_budget_cents: Some(1000),
            monthly_budget_cents: Some(30000),
            allowed_models: None,
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
        };
        let create_result = create_virtual_key(State(state.clone()), Json(req))
            .await
            .unwrap();
        let id_str = create_result.0.data["id"].as_str().unwrap().to_string();
        let id: Uuid = id_str.parse().unwrap();

        let delete_response = delete_virtual_key(State(state.clone()), Path(id)).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_virtual_keys(State(state)).await;
        assert!(list_result.data.is_empty());
    }
}
