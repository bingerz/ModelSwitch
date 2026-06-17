use super::ApiResponse;
use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
use crate::middleware::error::ApiError;
use crate::proxy::openai::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ─── Channel CRUD ─────────────────────────────────────

pub async fn list_channels(State(state): State<Arc<AppState>>) -> Json<ApiResponse<Vec<Channel>>> {
    let channels = state.channel_mgr.list().await;
    Json(ApiResponse::ok(channels))
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub provider: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default = "default_weight")]
    pub weight: u32,
    pub cost_per_token: Option<f64>,
    #[serde(default = "default_credential_type")]
    pub credential_type: String,
    #[serde(default)]
    pub credential_value: String,
    pub base_url: String,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: String,
    pub provider: String,
    pub priority: u8,
    pub weight: u32,
    pub cost_per_token: Option<f64>,
    pub base_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    pub cooldown_minutes: Option<u64>,
    /// If provided, update the stored API key credential
    #[serde(default)]
    pub credential_value: Option<String>,
    pub input_cost_per_mtok: Option<f64>,
    pub output_cost_per_mtok: Option<f64>,
    pub rpm_limit: Option<u64>,
    pub tpm_limit: Option<u64>,
}

fn default_priority() -> u8 {
    1
}
fn default_weight() -> u32 {
    100
}
fn default_credential_type() -> String {
    "api_key".to_string()
}

pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, axum::response::Response> {
    let cred_type = match req.credential_type.as_str() {
        "web_session" => CredentialType::WebSession,
        _ => CredentialType::ApiKey,
    };

    let id = Uuid::new_v4();
    let key_ref = format!("{}_{}", req.provider, id);

    // Store credential via CredentialStore abstraction
    state
        .credential_store
        .set("modelswitch", &key_ref, &req.credential_value)
        .map_err(|e| {
            tracing::error!("Failed to store credential: {}", e);
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store credential",
            )
        })?;

    let channel = Channel {
        id,
        name: req.name,
        provider: Provider::from_str(&req.provider),
        priority: req.priority,
        weight: req.weight,
        cost_per_token: req.cost_per_token,
        input_cost_per_mtok: None,
        output_cost_per_mtok: None,
        credential: Credential {
            cred_type,
            key_ref,
            api_key: None,
            expires_at: None,
        },
        enabled: true,
        status: ChannelStatus::Healthy,
        circuit_open_until: None,
        base_url: req.base_url,
        model_mapping: req.model_mapping,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        avg_latency_ms: 0,
        consecutive_failures: 0,
        cooldown_minutes: None,
        rpm_limit: None,
        tpm_limit: None,
        account_group: None,
        failure_window_start: None,
        window_failure_count: 0,
        max_concurrent: None,
        api_keys: vec![],
    };

    let created = state.channel_mgr.create(channel).await;
    state.channel_mgr.persist().await;
    Ok(Json(ApiResponse::ok(created)))
}

pub async fn update_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, axum::response::Response> {
    let mut existing = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    existing.name = req.name;
    existing.provider = Provider::from_str(&req.provider);
    existing.priority = req.priority;
    existing.weight = req.weight;
    existing.cost_per_token = req.cost_per_token;
    existing.input_cost_per_mtok = req.input_cost_per_mtok;
    existing.output_cost_per_mtok = req.output_cost_per_mtok;
    existing.rpm_limit = req.rpm_limit;
    existing.tpm_limit = req.tpm_limit;
    existing.base_url = req.base_url;
    if !req.enabled && existing.enabled {
        existing.status = ChannelStatus::Disabled;
    } else if req.enabled && !existing.enabled {
        existing.status = ChannelStatus::Healthy;
    }
    existing.enabled = req.enabled;
    existing.model_mapping = req.model_mapping;
    existing.cooldown_minutes = req.cooldown_minutes;
    existing.updated_at = chrono::Utc::now();

    // Update credential if a new value is provided
    if let Some(ref new_key) = req.credential_value {
        if !new_key.is_empty() {
            let username = &existing.credential.key_ref;
            state
                .credential_store
                .set("modelswitch", username, new_key)
                .map_err(|e| {
                    tracing::error!("Failed to update credential: {}", e);
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to update credential",
                    )
                })?;
            tracing::info!(channel = %existing.name, "Credential updated");
        }
    }

    let result = state.channel_mgr.update(id, existing).await;

    match result {
        Some(channel) => {
            state.channel_mgr.persist().await;
            Ok(Json(ApiResponse::ok(channel)))
        }
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "Channel not found")),
    }
}

pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    // Clean up credential before deleting
    if let Some(channel) = state.channel_mgr.get(id).await {
        let username = &channel.credential.key_ref;
        if let Err(e) = state.credential_store.delete("modelswitch", username) {
            tracing::warn!("Failed to delete credential for {}: {}", username, e);
        }
    }

    if state.channel_mgr.delete(id).await {
        state.billing.quota_store.delete(id).await;
        state.channel_mgr.persist().await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Channel not found")
    }
}

// ─── Channel Actions ──────────────────────────────────

pub async fn ping_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    let api_key = state.channel_mgr.get_credential(id).await.ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to retrieve credential",
        )
    })?;

    // Use provider-appropriate ping endpoint and auth
    let (url, auth_headers) = match &channel.provider {
        Provider::Anthropic => (
            format!("{}/v1/messages", channel.base_url.trim_end_matches('/')),
            vec![
                ("x-api-key", api_key.clone()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
        ),
        _ => (
            format!("{}/v1/models", channel.base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", api_key))],
        ),
    };

    let pool_guard = state.http_pool.get();
    let mut req_builder = pool_guard
        .get(&url)
        .timeout(std::time::Duration::from_secs(10));

    for (key, value) in &auth_headers {
        req_builder = req_builder.header(*key, value.as_str());
    }

    let start = std::time::Instant::now();
    let resp = req_builder.send().await;

    match resp {
        Ok(r) => {
            let latency = start.elapsed().as_millis() as u64;
            let success = r.status().is_success();
            Ok(Json(ApiResponse::ok(serde_json::json!({
                "success": success,
                "status": r.status().as_u16(),
                "latency_ms": latency,
            }))))
        }
        Err(e) => Ok(Json(ApiResponse::ok(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        })))),
    }
}

pub async fn channel_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "id": channel.id,
        "name": channel.name,
        "status": channel.status,
        "enabled": channel.enabled,
        "circuit_open_until": channel.circuit_open_until,
    }))))
}

/// Set payload rules for a channel at runtime.
pub async fn set_payload_rules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(rules): Json<crate::config::PayloadRulesConfig>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    // Verify channel exists
    state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    use crate::proxy::payload_rules::PayloadRules;
    state.limits.payload_rules.add(
        id,
        PayloadRules {
            defaults: rules.defaults,
            overrides: rules.overrides,
            strip: rules.strip,
        },
    );

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "channel_id": id,
        "updated": true
    }))))
}
