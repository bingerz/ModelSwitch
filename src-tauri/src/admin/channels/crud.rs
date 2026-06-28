use crate::admin::ApiResponse;
use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

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
    #[serde(default)]
    pub input_cost_per_mtok: Option<f64>,
    #[serde(default)]
    pub output_cost_per_mtok: Option<f64>,
    #[serde(default = "default_credential_type")]
    pub credential_type: String,
    #[serde(default)]
    pub credential_value: String,
    pub base_url: String,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    #[serde(default)]
    pub cooldown_minutes: Option<u64>,
    #[serde(default)]
    pub rpm_limit: Option<u64>,
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    #[serde(default)]
    pub account_group: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub models_refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
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
    #[serde(default)]
    pub account_group: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub models_refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
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
        input_cost_per_mtok: req.input_cost_per_mtok,
        output_cost_per_mtok: req.output_cost_per_mtok,
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
        cooldown_minutes: req.cooldown_minutes,
        rpm_limit: req.rpm_limit,
        tpm_limit: req.tpm_limit,
        account_group: req.account_group,
        max_concurrent: req.max_concurrent,
        api_keys: req.api_keys,
        excluded_models: req.excluded_models,
        model_cooldowns: std::collections::HashMap::new(),
        proxy_url: req.proxy_url,
        headers: req.headers,
        max_retries: req.max_retries,
        models_endpoint: req.models_endpoint,
        models_refresh_interval_secs: req.models_refresh_interval_secs.unwrap_or(300),
        tags: req.tags,
    };

    let created = state.channel_mgr.create(channel).await;
    state.channel_mgr.persist().await;
    state
        .audit_log
        .record(
            "channel.create",
            "admin-api",
            &created.id.to_string(),
            serde_json::json!({
                "name": created.name,
                "provider": format!("{:?}", created.provider),
            }),
        )
        .await;
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
    existing.account_group = req.account_group;
    existing.max_concurrent = req.max_concurrent;
    existing.excluded_models = req.excluded_models;
    existing.api_keys = req.api_keys;
    existing.proxy_url = req.proxy_url;
    existing.headers = req.headers;
    existing.max_retries = req.max_retries;
    existing.models_endpoint = req.models_endpoint;
    existing.models_refresh_interval_secs = req.models_refresh_interval_secs.unwrap_or(300);
    existing.tags = req.tags;
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
            state
                .audit_log
                .record(
                    "channel.update",
                    "admin-api",
                    &id.to_string(),
                    serde_json::json!({
                        "name": channel.name,
                        "provider": format!("{:?}", channel.provider),
                        "enabled": channel.enabled,
                    }),
                )
                .await;
            Ok(Json(ApiResponse::ok(channel)))
        }
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "Channel not found")),
    }
}

pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    // Capture channel name for audit before deletion
    let channel_name = state
        .channel_mgr
        .get(id)
        .await
        .map(|ch| ch.name)
        .unwrap_or_default();

    // Clean up credential before deleting
    if let Some(channel) = state.channel_mgr.get(id).await {
        let username = &channel.credential.key_ref;
        if let Err(e) = state.credential_store.delete("modelswitch", username) {
            tracing::warn!("Failed to delete credential for {}: {}", username, e);
        }
    }

    if state.channel_mgr.delete(id).await {
        state.billing.quota_store.delete(id).await;
        state.router.cooldown_tracker.remove(id);
        state.channel_mgr.persist().await;
        state
            .audit_log
            .record(
                "channel.delete",
                "admin-api",
                &id.to_string(),
                serde_json::json!({
                    "name": channel_name,
                }),
            )
            .await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Channel not found")
    }
}
