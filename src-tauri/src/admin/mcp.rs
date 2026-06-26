use super::ApiResponse;
use crate::config::McpServerConfig;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
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
pub async fn list_mcp_servers(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<McpServerResponse>>> {
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
    Json(ApiResponse::ok(responses))
}

/// POST /api/mcp/servers -- add a new server config.
pub async fn create_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateMcpServerRequest>,
) -> Result<Json<ApiResponse<McpServerResponse>>, axum::response::Response> {
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

    state
        .mcp
        .mcp_manager
        .reload_configs(&config.mcp_servers)
        .await;

    Ok(Json(ApiResponse::ok(McpServerResponse {
        id: new_config.id,
        name: new_config.name,
        command: new_config.command,
        args: new_config.args,
        env: new_config.env,
        cwd: new_config.cwd,
        enabled: new_config.enabled,
        expose_tools: new_config.expose_tools,
        status: crate::mcp::McpServerStatus::Stopped,
    })))
}

/// PUT /api/mcp/servers/:id -- update server config.
pub async fn update_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMcpServerRequest>,
) -> Result<Json<ApiResponse<McpServerResponse>>, axum::response::Response> {
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

    state
        .mcp
        .mcp_manager
        .reload_configs(&config.mcp_servers)
        .await;

    let status = state
        .mcp
        .mcp_manager
        .list_status()
        .await
        .into_iter()
        .find(|(sid, _, _)| *sid == id)
        .map(|(_, _, s)| s)
        .unwrap_or(crate::mcp::McpServerStatus::Stopped);

    Ok(Json(ApiResponse::ok(McpServerResponse {
        id: updated_config.id,
        name: updated_config.name,
        command: updated_config.command,
        args: updated_config.args,
        env: updated_config.env,
        cwd: updated_config.cwd,
        enabled: updated_config.enabled,
        expose_tools: updated_config.expose_tools,
        status,
    })))
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
    state
        .mcp
        .mcp_manager
        .reload_configs(&config.mcp_servers)
        .await;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /api/mcp/servers/:id/start -- spawn the subprocess.
pub async fn start_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    state
        .mcp
        .mcp_manager
        .start_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(ApiResponse::ok(serde_json::json!({ "ok": true }))))
}

/// POST /api/mcp/servers/:id/stop -- terminate the subprocess.
pub async fn stop_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    state
        .mcp
        .mcp_manager
        .stop_server(&id)
        .await
        .map_err(mcp_error_to_response)?;

    Ok(Json(ApiResponse::ok(serde_json::json!({ "ok": true }))))
}

/// GET /api/mcp/servers/:id/tools -- list tools from this server.
pub async fn list_mcp_server_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Vec<McpToolResponse>>>, axum::response::Response> {
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

    Ok(Json(ApiResponse::ok(response)))
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
) -> Json<ApiResponse<Vec<crate::mcp::AggregatedTool>>> {
    Json(ApiResponse::ok(
        crate::mcp::aggregator::aggregate_all_tools(&state.mcp.mcp_manager).await,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServerConfig;
    use crate::test_helpers::{build_test_state, response_status};
    use std::collections::HashMap;

    fn make_config(id: &str, name: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            name: name.to_string(),
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            expose_tools: true,
        }
    }

    fn make_create_request(id: &str, name: &str, command: &str) -> CreateMcpServerRequest {
        CreateMcpServerRequest {
            id: id.to_string(),
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            expose_tools: true,
        }
    }

    // ─── P0: error mapping helper ─────────────────────────

    #[test]
    fn mcp_error_to_response_maps_unknown_to_404() {
        let err = anyhow::anyhow!("Unknown server id");
        let resp = mcp_error_to_response(err);
        assert_eq!(response_status(&resp), 404);
    }

    #[test]
    fn mcp_error_to_response_maps_already_running_to_409() {
        let err = anyhow::anyhow!("Server already running");
        let resp = mcp_error_to_response(err);
        assert_eq!(response_status(&resp), 409);
    }

    #[test]
    fn mcp_error_to_response_maps_not_running_to_409() {
        let err = anyhow::anyhow!("Server not running");
        let resp = mcp_error_to_response(err);
        assert_eq!(response_status(&resp), 409);
    }

    #[test]
    fn mcp_error_to_response_maps_other_to_500() {
        let err = anyhow::anyhow!("spawn failed: EACCES");
        let resp = mcp_error_to_response(err);
        assert_eq!(response_status(&resp), 500);
    }

    // ─── P1: read-only handlers ──────────────────────────

    #[tokio::test]
    async fn list_mcp_servers_returns_empty_initially() {
        let state = build_test_state(vec![]);
        let result = list_mcp_servers(State(state)).await;
        assert!(result.data.is_empty());
        assert!(result.ok);
    }

    #[tokio::test]
    async fn list_all_mcp_tools_returns_empty_when_no_servers() {
        let state = build_test_state(vec![]);
        let result = list_all_mcp_tools(State(state)).await;
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn list_mcp_servers_reflects_loaded_configs() {
        let state = build_test_state(vec![]);
        state
            .mcp
            .mcp_manager
            .load_configs(&[make_config("srv1", "My Server")])
            .await;

        let result = list_mcp_servers(State(state)).await;
        assert_eq!(result.data.len(), 1);
        assert_eq!(result.data[0].id, "srv1");
        assert_eq!(result.data[0].name, "My Server");
    }

    // ─── P2: error paths for action handlers ─────────────
    //
    // `ApiResponse<T>` is not Debug, so we cannot use `expect_err`. Match on
    // the `Err` variant to pull out the error Response directly.

    #[tokio::test]
    async fn start_mcp_server_returns_404_for_unknown_id() {
        let state = build_test_state(vec![]);
        let result = start_mcp_server(State(state), Path("no-such".to_string())).await;
        match result {
            Ok(_) => panic!("expected error for unknown id"),
            Err(resp) => assert_eq!(response_status(&resp), 404),
        }
    }

    #[tokio::test]
    async fn stop_mcp_server_returns_404_for_unknown_id() {
        let state = build_test_state(vec![]);
        let result = stop_mcp_server(State(state), Path("no-such".to_string())).await;
        match result {
            Ok(_) => panic!("expected error for unknown id"),
            Err(resp) => assert_eq!(response_status(&resp), 404),
        }
    }

    #[tokio::test]
    async fn list_mcp_server_tools_returns_error_for_unknown_id() {
        let state = build_test_state(vec![]);
        let result = list_mcp_server_tools(State(state), Path("no-such".to_string())).await;
        match result {
            Ok(_) => panic!("expected error for unknown id"),
            Err(resp) => assert_eq!(response_status(&resp), 404),
        }
    }

    #[tokio::test]
    async fn list_mcp_server_tools_returns_error_for_stopped_server() {
        let state = build_test_state(vec![]);
        state
            .mcp
            .mcp_manager
            .load_configs(&[make_config("srv1", "Server One")])
            .await;

        let result = list_mcp_server_tools(State(state), Path("srv1".to_string())).await;
        match result {
            Ok(_) => panic!("expected error for stopped server"),
            Err(resp) => {
                // "MCP server srv1 is not running" maps to CONFLICT (409)
                // because the error mapping treats "not running" as a state
                // conflict.
                assert_eq!(response_status(&resp), 409);
            }
        }
    }

    // ─── P3: create validation branches (pre-disk-I/O) ───
    //
    // These branches return BEFORE any AppConfig::load() / config.save() disk
    // I/O, so they are safe to exercise without corrupting the developer's
    // config file.

    #[tokio::test]
    async fn create_mcp_server_rejects_empty_id() {
        let state = build_test_state(vec![]);
        let req = make_create_request("", "test", "echo");
        let result = create_mcp_server(State(state), Json(req)).await;
        match result {
            Ok(_) => panic!("expected BAD_REQUEST for empty id"),
            Err(resp) => assert_eq!(response_status(&resp), 400),
        }
    }

    #[tokio::test]
    async fn create_mcp_server_rejects_empty_name() {
        let state = build_test_state(vec![]);
        let req = make_create_request("srv1", "", "echo");
        let result = create_mcp_server(State(state), Json(req)).await;
        match result {
            Ok(_) => panic!("expected BAD_REQUEST for empty name"),
            Err(resp) => assert_eq!(response_status(&resp), 400),
        }
    }

    #[tokio::test]
    async fn create_mcp_server_rejects_whitespace_only_fields() {
        let state = build_test_state(vec![]);
        let req = make_create_request("   ", "test", "echo");
        let result = create_mcp_server(State(state), Json(req)).await;
        match result {
            Ok(_) => panic!("expected BAD_REQUEST for whitespace-only id"),
            Err(resp) => assert_eq!(response_status(&resp), 400),
        }
    }
}
