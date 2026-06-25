use super::ApiResponse;
use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
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
        api_keys: vec![],
        excluded_models: vec![],
        model_cooldowns: std::collections::HashMap::new(),
        proxy_url: None,
        headers: std::collections::HashMap::new(),
        max_retries: None,
        models_endpoint: None,
        models_refresh_interval_secs: 300,
        tags: vec![],
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

// ─── Batch Operations ─────────────────────────────────

/// Request body for batch operations that only need channel IDs.
#[derive(Debug, Deserialize)]
pub struct BatchChannelRequest {
    pub ids: Vec<Uuid>,
}

/// Request body for batch tag updates.
#[derive(Debug, Deserialize)]
pub struct BatchTagUpdate {
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

/// Summary of a batch operation result.
#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub errors: Vec<BatchErrorEntry>,
}

/// A single error within a batch result.
#[derive(Debug, Serialize)]
pub struct BatchErrorEntry {
    pub id: Uuid,
    pub error: String,
}

impl BatchResult {
    fn new(total: usize) -> Self {
        Self {
            total,
            success: 0,
            failed: 0,
            errors: Vec::new(),
        }
    }
}

/// POST /api/channels/batch/enable — enable multiple channels at once.
pub async fn batch_enable_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                channel.enabled = true;
                if channel.status == ChannelStatus::Disabled {
                    channel.status = ChannelStatus::Healthy;
                }
                channel.updated_at = chrono::Utc::now();
                if state.channel_mgr.update(id, channel.clone()).await.is_some() {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_enable",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": channel.name,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// POST /api/channels/batch/disable — disable multiple channels at once.
pub async fn batch_disable_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                channel.enabled = false;
                channel.status = ChannelStatus::Disabled;
                channel.updated_at = chrono::Utc::now();
                if state.channel_mgr.update(id, channel.clone()).await.is_some() {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_disable",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": channel.name,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// POST /api/channels/batch/delete — delete multiple channels at once.
pub async fn batch_delete_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
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
            result.success += 1;
            state
                .audit_log
                .record(
                    "channel.batch_delete",
                    "admin-api",
                    &id.to_string(),
                    serde_json::json!({
                        "name": channel_name,
                    }),
                )
                .await;
        } else {
            result.failed += 1;
            result.errors.push(BatchErrorEntry {
                id,
                error: "Channel not found".to_string(),
            });
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// PUT /api/channels/batch/tags — add and/or remove tags on multiple channels.
pub async fn batch_update_tags(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchTagUpdate>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                // Remove requested tags
                channel.tags.retain(|t| !req.remove_tags.contains(t));
                // Add requested tags (avoid duplicates)
                for tag in &req.add_tags {
                    if !channel.tags.contains(tag) {
                        channel.tags.push(tag.clone());
                    }
                }
                channel.updated_at = chrono::Utc::now();
                let tags_snapshot = channel.tags.clone();
                let name_snapshot = channel.name.clone();
                if state.channel_mgr.update(id, channel).await.is_some() {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_update_tags",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": name_snapshot,
                                "tags": tags_snapshot,
                                "added": req.add_tags,
                                "removed": req.remove_tags,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn list_channels_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_channels(State(state)).await;
        assert!(result.ok);
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn create_channel_adds_to_list() {
        let state = build_test_state(vec![]);
        let req = CreateChannelRequest {
            name: "test-channel".to_string(),
            provider: "openai".to_string(),
            priority: 1,
            weight: 100,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential_type: "api_key".to_string(),
            credential_value: "sk-test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
        };
        let created = create_channel(State(state.clone()), Json(req))
            .await
            .expect("create_channel should succeed");
        let channel_id = created.0.data.id;

        let list_result = list_channels(State(state)).await;
        assert!(list_result.ok);
        assert_eq!(list_result.data.len(), 1);
        assert_eq!(list_result.data[0].id, channel_id);
    }

    #[tokio::test]
    async fn delete_channel_removes_from_list() {
        let state = build_test_state(vec![]);
        let req = CreateChannelRequest {
            name: "delete-me".to_string(),
            provider: "openai".to_string(),
            priority: 1,
            weight: 100,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential_type: "api_key".to_string(),
            credential_value: "sk-test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
        };
        let created = create_channel(State(state.clone()), Json(req))
            .await
            .expect("create_channel should succeed");
        let channel_id = created.0.data.id;

        let delete_response = delete_channel(State(state.clone()), Path(channel_id)).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_channels(State(state)).await;
        assert!(list_result.ok);
        assert!(list_result.data.is_empty());
    }

    // ─── Batch operation helpers ───────────────────────────

    /// Helper: create N test channels and return their IDs.
    async fn create_test_channels(state: &Arc<AppState>, count: usize) -> Vec<Uuid> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let req = CreateChannelRequest {
                name: format!("batch-ch-{i}"),
                provider: "openai".to_string(),
                priority: 1,
                weight: 100,
                cost_per_token: None,
                input_cost_per_mtok: None,
                output_cost_per_mtok: None,
                credential_type: "api_key".to_string(),
                credential_value: "sk-test".to_string(),
                base_url: "https://api.openai.com".to_string(),
                model_mapping: HashMap::new(),
                cooldown_minutes: None,
                rpm_limit: None,
                tpm_limit: None,
                account_group: None,
                max_concurrent: None,
            };
            let created = create_channel(State(state.clone()), Json(req))
                .await
                .expect("create_channel should succeed");
            ids.push(created.0.data.id);
        }
        ids
    }

    // ─── Batch enable tests ────────────────────────────────

    #[tokio::test]
    async fn batch_enable_enables_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 3).await;

        // Disable them first
        for &id in &ids {
            let mut ch = state.channel_mgr.get(id).await.unwrap();
            ch.enabled = false;
            ch.status = ChannelStatus::Disabled;
            state.channel_mgr.update(id, ch).await;
        }

        // Batch enable
        let result = batch_enable_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 3);
        assert_eq!(result.data.failed, 0);
        assert!(result.data.errors.is_empty());

        // Verify all channels are enabled
        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(ch.enabled);
            assert_ne!(ch.status, ChannelStatus::Disabled);
        }
    }

    #[tokio::test]
    async fn batch_enable_reports_not_found_errors() {
        let state = build_test_state(vec![]);
        let fake_id = Uuid::new_v4();

        let result = batch_enable_channels(
            State(state),
            Json(BatchChannelRequest {
                ids: vec![fake_id],
            }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 1);
        assert_eq!(result.data.success, 0);
        assert_eq!(result.data.failed, 1);
        assert_eq!(result.data.errors[0].id, fake_id);
    }

    // ─── Batch disable tests ───────────────────────────────

    #[tokio::test]
    async fn batch_disable_disables_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        let result = batch_disable_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 2);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 0);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(!ch.enabled);
            assert_eq!(ch.status, ChannelStatus::Disabled);
        }
    }

    // ─── Batch delete tests ────────────────────────────────

    #[tokio::test]
    async fn batch_delete_removes_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 3).await;

        let result = batch_delete_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 3);
        assert_eq!(result.data.failed, 0);

        let list = list_channels(State(state)).await;
        assert!(list.data.is_empty());
    }

    #[tokio::test]
    async fn batch_delete_with_partial_failures() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;
        let fake_id = Uuid::new_v4();

        let mut all_ids = ids.clone();
        all_ids.push(fake_id);

        let result = batch_delete_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: all_ids }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 1);
        assert_eq!(result.data.errors.len(), 1);
        assert_eq!(result.data.errors[0].id, fake_id);

        let list = list_channels(State(state)).await;
        assert!(list.data.is_empty());
    }

    // ─── Batch update tags tests ───────────────────────────

    #[tokio::test]
    async fn batch_update_tags_adds_tags_to_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        let result = batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["production".to_string(), "fast".to_string()],
                remove_tags: vec![],
            }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 2);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 0);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(ch.tags.contains(&"production".to_string()));
            assert!(ch.tags.contains(&"fast".to_string()));
        }
    }

    #[tokio::test]
    async fn batch_update_tags_removes_tags_from_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        // First add tags
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["production".to_string(), "staging".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        // Now remove "production"
        let result = batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec![],
                remove_tags: vec!["production".to_string()],
            }),
        )
        .await
        .0;

        assert_eq!(result.data.success, 2);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(!ch.tags.contains(&"production".to_string()));
            assert!(ch.tags.contains(&"staging".to_string()));
        }
    }

    #[tokio::test]
    async fn batch_update_tags_does_not_create_duplicates() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;

        // Add tag
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["alpha".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        // Add same tag again
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["alpha".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        let ch = state.channel_mgr.get(ids[0]).await.unwrap();
        let count = ch.tags.iter().filter(|t| *t == "alpha").count();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn batch_enable_empty_ids_is_noop() {
        let state = build_test_state(vec![]);

        let result =
            batch_enable_channels(State(state), Json(BatchChannelRequest { ids: vec![] }))
                .await
                .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 0);
        assert_eq!(result.data.success, 0);
        assert_eq!(result.data.failed, 0);
    }
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
    let model_rules = rules.model_rules.clone();
    state.limits.payload_rules.add(
        id,
        PayloadRules {
            defaults: rules.defaults,
            overrides: rules.overrides,
            strip: rules.strip,
        },
    );
    if !model_rules.is_empty() {
        state.limits.payload_rules.set_model_rules(id, model_rules);
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "channel_id": id,
        "updated": true
    }))))
}

/// Get the current payload rules for a channel.
///
/// Returns the stored rules, or an empty default if none have been
/// configured. Returns 404 if the channel itself does not exist.
pub async fn get_payload_rules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<crate::config::PayloadRulesConfig>>, axum::response::Response> {
    // Verify channel exists
    state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    let rules = state
        .limits
        .payload_rules
        .get_full(id)
        .map(|full| crate::config::PayloadRulesConfig {
            defaults: full.channel.defaults,
            overrides: full.channel.overrides,
            strip: full.channel.strip,
            model_rules: full.model_rules,
        })
        .unwrap_or_default();

    Ok(Json(ApiResponse::ok(rules)))
}
