use crate::config::McpServerConfig;
use crate::middleware::error::ApiError;
use crate::proxy::openai::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ─── Types ─────────────────────────────────────────────

fn default_true() -> bool {
    true
}

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

// ─── Endpoints ─────────────────────────────────────────

/// GET /api/mcp/servers -- list all servers with config and status.
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

/// POST /api/mcp/servers -- add a new server config.
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

/// PUT /api/mcp/servers/:id -- update server config.
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

    let status = state
        .mcp
        .mcp_manager
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

/// DELETE /api/mcp/servers/:id -- remove server (stop if running).
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

/// POST /api/mcp/servers/:id/start -- spawn the subprocess.
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    state
        .mcp
        .mcp_manager
        .start_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(serde_json::json!({ "ok": true })).into_response())
}

/// POST /api/mcp/servers/:id/stop -- terminate the subprocess.
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    state
        .mcp
        .mcp_manager
        .stop_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(serde_json::json!({ "ok": true })).into_response())
}

/// GET /api/mcp/servers/:id/tools -- list tools from this server.
pub async fn list_mcp_server_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    let tools = state
        .mcp
        .mcp_manager
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

/// GET /api/mcp/tools -- aggregated tools across all running servers.
///
/// Returns full `AggregatedTool` records (including `input_schema`) so the
/// admin UI can render tool documentation. Unlike the proxy `/v1/tools`
/// endpoint, this does NOT filter by `expose_tools` -- the admin UI should
/// be able to see every tool regardless of whether it is exposed to LLM
/// clients.
pub async fn list_all_mcp_tools(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::mcp::AggregatedTool>> {
    Json(crate::mcp::aggregator::aggregate_all_tools(&state.mcp.mcp_manager).await)
}
