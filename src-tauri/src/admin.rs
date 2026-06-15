use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
use crate::config::McpServerConfig;
use crate::log::DispatchLog;
use crate::middleware::error::ApiError;
use crate::proxy::openai::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Unified API response envelope.
#[derive(serde::Serialize)]
pub struct ApiResponse<T: serde::Serialize> {
    pub ok: bool,
    pub data: T,
}

impl<T: serde::Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { ok: true, data }
    }
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_offset")]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_offset() -> usize {
    0
}
fn default_limit() -> usize {
    50
}

fn default_hours() -> u64 {
    24
}

fn default_true() -> bool {
    true
}

// ─── Channel CRUD ─────────────────────────────────────

pub async fn list_channels(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<Channel>>> {
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
) -> Result<axum::response::Response, axum::response::Response> {
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
    };

    let created = state.channel_mgr.create(channel).await;
    state.channel_mgr.persist().await;
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

pub async fn update_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<axum::response::Response, axum::response::Response> {
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
            Ok(Json(channel).into_response())
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
) -> Result<axum::response::Response, axum::response::Response> {
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

    let mut req_builder = state
        .http_client
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
            Ok(Json(serde_json::json!({
                "success": success,
                "status": r.status().as_u16(),
                "latency_ms": latency,
            }))
            .into_response())
        }
        Err(e) => Ok(Json(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        }))
        .into_response()),
    }
}

pub async fn channel_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    Ok(Json(serde_json::json!({
        "id": channel.id,
        "name": channel.name,
        "status": channel.status,
        "enabled": channel.enabled,
        "circuit_open_until": channel.circuit_open_until,
    }))
    .into_response())
}

/// Set payload rules for a channel at runtime.
pub async fn set_payload_rules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(rules): Json<crate::config::PayloadRulesConfig>,
) -> Result<axum::response::Response, axum::response::Response> {
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

    Ok(Json(serde_json::json!({
        "channel_id": id,
        "updated": true
    }))
    .into_response())
}

// ─── Logs & Stats ─────────────────────────────────────

pub async fn get_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> Json<Vec<DispatchLog>> {
    let logs = state.logger.list(params.offset, params.limit).await;
    Json(logs)
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<crate::log::DispatchStats>> {
    let stats = state.logger.stats().await;
    Json(ApiResponse::ok(stats))
}

pub async fn get_cost_stats(State(state): State<Arc<AppState>>) -> Json<crate::log::CostStats> {
    let stats = state.logger.cost_stats().await;
    Json(stats)
}

pub async fn get_quota(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<crate::quota::QuotaInfo>>> {
    let quotas = state.billing.quota_store.list().await;
    Json(ApiResponse::ok(quotas))
}

#[derive(Debug, Deserialize)]
pub struct UsageParams {
    #[serde(default = "default_hours")]
    pub hours: u64,
}

pub async fn get_usage_history(
    Query(params): Query<UsageParams>,
    State(state): State<Arc<AppState>>,
) -> Json<crate::log::UsageHistory> {
    let history = state.logger.usage_history(params.hours).await;
    Json(history)
}

// ─── Operational Endpoints ────────────────────────────

/// Reset a channel's circuit breaker, forcing it back to healthy state.
pub async fn reset_circuit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    state.channel_mgr.force_recover(id).await;

    Ok(Json(serde_json::json!({
        "id": channel.id,
        "name": channel.name,
        "status": "healthy",
        "message": "Circuit breaker reset"
    }))
    .into_response())
}

/// Flush all cached responses.
pub async fn flush_cache(State(state): State<Arc<AppState>>) -> axum::response::Response {
    state.cache.request_cache.flush();
    let count = state.cache.request_cache.len();
    Json(serde_json::json!({
        "flushed": true,
        "remaining": count
    }))
    .into_response()
}

/// Reload configuration from disk and update channels.
pub async fn reload_config(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match crate::config::AppConfig::load() {
        Ok(new_config) => {
            let channels = state.channel_mgr.list().await;
            let mut created = 0u32;
            let mut updated = 0u32;
            let mut removed = 0u32;

            // Update or create channels from new config
            for cc in &new_config.channels {
                let id = match Uuid::parse_str(&cc.id) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                if let Some(existing) = state.channel_mgr.get(id).await {
                    let mut ch = existing;
                    ch.name = cc.name.clone();
                    ch.provider = Provider::from_str(&cc.provider);
                    ch.priority = cc.priority;
                    ch.weight = cc.weight;
                    ch.cost_per_token = cc.cost_per_token;
                    ch.input_cost_per_mtok = cc.input_cost_per_mtok;
                    ch.output_cost_per_mtok = cc.output_cost_per_mtok;
                    ch.base_url = cc.base_url.clone();
                    ch.model_mapping = cc.model_mapping.clone();
                    ch.enabled = cc.enabled;
                    ch.cooldown_minutes = cc.cooldown_minutes;
                    ch.rpm_limit = cc.rpm_limit;
                    ch.tpm_limit = cc.tpm_limit;
                    ch.updated_at = chrono::Utc::now();
                    let _ = state.channel_mgr.update(id, ch).await;
                    updated += 1;
                } else {
                    let new_channel = Channel::from_config(&cc);
                    let _ = state.channel_mgr.create(new_channel).await;
                    created += 1;
                }
            }

            // Remove channels no longer in config
            let config_ids: Vec<Uuid> = new_config
                .channels
                .iter()
                .filter_map(|c| Uuid::parse_str(&c.id).ok())
                .collect();
            for ch in &channels {
                if !config_ids.contains(&ch.id) {
                    state.channel_mgr.delete(ch.id).await;
                    state.billing.quota_store.delete(ch.id).await;
                    removed += 1;
                }
            }

            // Update rate limits
            for cc in &new_config.channels {
                let id = match Uuid::parse_str(&cc.id) {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                if let Some(rpm) = cc.rpm_limit {
                    state.limits.rate_limiter.set_channel_rpm_limit(id, rpm);
                }
                if let Some(tpm) = cc.tpm_limit {
                    state.limits.rate_limiter.set_channel_tpm_limit(id, tpm);
                }
            }

            tracing::info!(
                "Config reload: {updated} updated, {created} created, {removed} removed"
            );
            Json(serde_json::json!({
                "reloaded": true,
                "updated": updated,
                "created": created,
                "removed": removed
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!("Config reload failed: {}", e);
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to reload config")
                .into_response()
        }
    }
}

/// Receive cookies from the WebView login flow.
/// The WebView injects JS that POSTs cookies here after login succeeds.
pub async fn receive_login_cookies(
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let cookies = body.get("cookies").and_then(|v| v.as_str()).unwrap_or("");

    if cookies.is_empty() {
        tracing::warn!(provider, "WebView login returned empty cookies");
        return ApiError::new(StatusCode::BAD_REQUEST, "Empty cookies");
    }

    tracing::info!(
        provider,
        cookie_len = cookies.len(),
        "Received login cookies from WebView"
    );

    // Store in a temporary file for the frontend to pick up
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch");
    let _ = std::fs::create_dir_all(&dir);
    let pending_file = dir.join("pending_cookies.json");

    let data = serde_json::json!({
        "provider": provider,
        "cookies": cookies,
        "received_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = std::fs::write(&pending_file, data.to_string()) {
        tracing::error!("Failed to write pending cookies: {}", e);
        return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write cookies");
    }
    // Restrict file permissions to owner-only (sensitive session cookies)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&pending_file, std::fs::Permissions::from_mode(0o600));
    }

    StatusCode::OK.into_response()
}

/// Return the most recent pending cookies from WebView login (one-shot read).
pub async fn get_pending_cookies() -> Json<Option<serde_json::Value>> {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch");
    let pending_file = dir.join("pending_cookies.json");

    if !pending_file.exists() {
        return Json(None);
    }

    match std::fs::read_to_string(&pending_file) {
        Ok(content) => {
            // Delete after reading (one-shot)
            let _ = std::fs::remove_file(&pending_file);
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(v) => Json(Some(v)),
                Err(_) => Json(None),
            }
        }
        Err(_) => Json(None),
    }
}

// ─── MCP Server Management ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateMcpServerRequest {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub expose_tools: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMcpServerRequest {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub expose_tools: bool,
}

#[derive(Serialize)]
pub struct McpServerResponse {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub enabled: bool,
    pub expose_tools: bool,
    pub status: crate::mcp::McpServerStatus,
}

#[derive(Serialize)]
pub struct McpToolResponse {
    pub name: String,
    pub description: Option<String>,
}

/// Map an anyhow error to an appropriate HTTP response based on its message.
fn mcp_error_to_response(e: anyhow::Error) -> axum::response::Response {
    let msg = e.to_string();
    if msg.contains("Unknown") {
        ApiError::new(StatusCode::NOT_FOUND, msg.as_str())
    } else if msg.contains("already running") || msg.contains("not running") {
        ApiError::new(StatusCode::CONFLICT, msg.as_str())
    } else {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, msg.as_str())
    }
}

/// GET /api/mcp/servers — list all servers with config and status.
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> Json<Vec<McpServerResponse>> {
    let statuses = state.mcp.mcp_manager.list_status().await;
    let mut responses = Vec::with_capacity(statuses.len());
    for (id, _name, status) in statuses {
        if let Some(config) = state.mcp.mcp_manager.get_config(&id).await {
            responses.push(McpServerResponse {
                id: config.id,
                name: config.name,
                command: config.command,
                args: config.args,
                env: config.env,
                cwd: config.cwd,
                enabled: config.enabled,
                expose_tools: config.expose_tools,
                status,
            });
        }
    }
    Json(responses)
}

/// POST /api/mcp/servers — add a new server config.
pub async fn create_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateMcpServerRequest>,
) -> Result<axum::response::Response, axum::response::Response> {
    if req.id.trim().is_empty() || req.name.trim().is_empty() || req.command.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "id, name, and command are required",
        ));
    }

    let mut config = crate::config::AppConfig::load().map_err(|e| {
        tracing::error!("Failed to load config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load config")
    })?;

    if config.mcp_servers.iter().any(|s| s.id == req.id) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "MCP server with this ID already exists",
        ));
    }

    let new_config = McpServerConfig {
        id: req.id,
        name: req.name,
        command: req.command,
        args: req.args,
        env: req.env,
        cwd: req.cwd,
        enabled: req.enabled,
        expose_tools: req.expose_tools,
    };

    config.mcp_servers.push(new_config.clone());

    config.save().map_err(|e| {
        tracing::error!("Failed to save config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config")
    })?;

    state.mcp.mcp_manager.reload_configs(&config.mcp_servers).await;

    Ok((
        StatusCode::CREATED,
        Json(McpServerResponse {
            id: new_config.id,
            name: new_config.name,
            command: new_config.command,
            args: new_config.args,
            env: new_config.env,
            cwd: new_config.cwd,
            enabled: new_config.enabled,
            expose_tools: new_config.expose_tools,
            status: crate::mcp::McpServerStatus::Stopped,
        }),
    )
        .into_response())
}

/// PUT /api/mcp/servers/:id — update server config.
pub async fn update_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMcpServerRequest>,
) -> Result<axum::response::Response, axum::response::Response> {
    let mut config = crate::config::AppConfig::load().map_err(|e| {
        tracing::error!("Failed to load config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load config")
    })?;

    let updated_config = {
        let server_config = config
            .mcp_servers
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "MCP server not found"))?;

        server_config.name = req.name;
        server_config.command = req.command;
        server_config.args = req.args;
        server_config.env = req.env;
        server_config.cwd = req.cwd;
        server_config.enabled = req.enabled;
        server_config.expose_tools = req.expose_tools;
        server_config.clone()
    };

    config.save().map_err(|e| {
        tracing::error!("Failed to save config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config")
    })?;

    state.mcp.mcp_manager.reload_configs(&config.mcp_servers).await;

    let status = state.mcp.mcp_manager
        .list_status()
        .await
        .into_iter()
        .find(|(sid, _, _)| *sid == id)
        .map(|(_, _, s)| s)
        .unwrap_or(crate::mcp::McpServerStatus::Stopped);

    Ok(Json(McpServerResponse {
        id: updated_config.id,
        name: updated_config.name,
        command: updated_config.command,
        args: updated_config.args,
        env: updated_config.env,
        cwd: updated_config.cwd,
        enabled: updated_config.enabled,
        expose_tools: updated_config.expose_tools,
        status,
    })
    .into_response())
}

/// DELETE /api/mcp/servers/:id — remove server (stop if running).
pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    let mut config = crate::config::AppConfig::load().map_err(|e| {
        tracing::error!("Failed to load config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load config")
    })?;

    let before = config.mcp_servers.len();
    config.mcp_servers.retain(|s| s.id != id);
    if config.mcp_servers.len() == before {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "MCP server not found"));
    }

    config.save().map_err(|e| {
        tracing::error!("Failed to save config: {}", e);
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config")
    })?;

    // reload_configs will stop the server if running
    state.mcp.mcp_manager.reload_configs(&config.mcp_servers).await;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /api/mcp/servers/:id/start — spawn the subprocess.
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    state.mcp.mcp_manager
        .start_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(serde_json::json!({ "ok": true })).into_response())
}

/// POST /api/mcp/servers/:id/stop — terminate the subprocess.
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    state.mcp.mcp_manager
        .stop_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(serde_json::json!({ "ok": true })).into_response())
}

/// GET /api/mcp/servers/:id/tools — list tools from this server.
pub async fn list_mcp_server_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    let tools = state.mcp.mcp_manager
        .list_tools(&id)
        .await
        .map_err(mcp_error_to_response)?;

    let response: Vec<McpToolResponse> = tools
        .iter()
        .map(|t| McpToolResponse {
            name: t.name.to_string(),
            description: t.description.as_deref().map(String::from),
        })
        .collect();

    Ok(Json(response).into_response())
}

/// GET /api/mcp/tools — aggregated tools across all running servers.
///
/// Returns full `AggregatedTool` records (including `input_schema`) so the
/// admin UI can render tool documentation. Unlike the proxy `/v1/tools`
/// endpoint, this does NOT filter by `expose_tools` — the admin UI should
/// be able to see every tool regardless of whether it is exposed to LLM
/// clients.
pub async fn list_all_mcp_tools(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::mcp::AggregatedTool>> {
    Json(crate::mcp::aggregator::aggregate_all_tools(&state.mcp.mcp_manager).await)
}

// ─── Virtual Key CRUD ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateVirtualKeyRequest {
    pub name: String,
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
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
}

/// Response shape for the list endpoint — never exposes `key_hash`.
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

/// GET /api/virtual-keys — list all keys with current spend.
/// Never returns `key_hash`. Plaintext is only returned once at creation time.
pub async fn list_virtual_keys(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<VirtualKeyResponse>> {
    let keys = state.billing.virtual_key_store.list().await;
    Json(keys.iter().map(VirtualKeyResponse::from).collect())
}

/// POST /api/virtual-keys — create a new virtual key.
/// Returns the plaintext key in the response body. This is the only time the
/// plaintext is visible; subsequent calls only ever see `key_prefix`.
pub async fn create_virtual_key(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateVirtualKeyRequest>,
) -> axum::response::Response {
    if req.name.trim().is_empty() {
        return ApiError::new(StatusCode::BAD_REQUEST, "Name is required");
    }
    if let Some(d) = req.daily_budget_cents {
        if let Some(m) = req.monthly_budget_cents {
            if m < d {
                return ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Monthly budget must be >= daily budget",
                );
            }
        }
    }
    let (vk, plaintext) = state.billing.virtual_key_store
        .create(req.name, req.daily_budget_cents, req.monthly_budget_cents)
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
    });
    (StatusCode::CREATED, Json(body)).into_response()
}

/// PUT /api/virtual-keys/:id — update fields on a virtual key.
pub async fn update_virtual_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateVirtualKeyRequest>,
) -> Result<axum::response::Response, axum::response::Response> {
    let updated = state.billing.virtual_key_store
        .update(
            id,
            req.name,
            req.daily_budget_cents,
            req.monthly_budget_cents,
            req.enabled,
        )
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Virtual key not found"))?;
    persist_virtual_keys(&state).await;
    Ok(Json(VirtualKeyResponse::from(&updated)).into_response())
}

/// DELETE /api/virtual-keys/:id — remove a virtual key permanently.
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
