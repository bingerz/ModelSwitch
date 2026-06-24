use crate::model_registry::ModelCapabilities;
use crate::proxy::mcp_tools;
use crate::proxy::state::AppState;
use crate::proxy::stream::json_response;
use crate::proxy::{dispatch, provider::OpenAIAdaptor, RequestFormat};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

/// Handle OpenAI-compatible /v1/chat/completions requests.
///
/// When MCP tool auto-injection is enabled (default), the handler:
///
/// 1. Aggregates tools from every running MCP server and appends them
///    to `body.tools` under their `mcp__{server}__{tool}` namespace.
/// 2. Dispatches the request upstream. If the response contains
///    tool_calls targeting MCP tools, the handler executes each call
///    via `McpManager`, appends the results to the conversation, and
///    re-dispatches. This loop runs at most `mcp_max_iterations` times.
/// 3. Returns the final response (either text content or a remaining
///    set of non-MCP tool_calls the client must resolve).
///
/// **Streaming caveat**: when MCP tools are injected and the client
/// requested `stream: true`, the loop processes each iteration as
/// non-streaming internally and returns the final result as a single
/// JSON payload. Clients that require SSE streaming should disable MCP
/// auto-injection (set `mcp_auto_inject = false` in the gateway config)
/// or call without MCP servers running.
pub async fn handle_chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if let Err(resp) = crate::proxy::validate_chat_request(&body) {
        return resp;
    }

    let provider = OpenAIAdaptor;

    // If MCP auto-inject is disabled (or no servers running), short-circuit.
    if !state.mcp.mcp_auto_inject {
        return dispatch(
            &state,
            &headers,
            &body,
            &provider,
            RequestFormat::OpenAIChat,
        )
        .await;
    }

    let (mut current_body, injected) =
        mcp_tools::inject_mcp_tools(&body, &state.mcp.mcp_manager).await;
    if injected.is_empty() {
        // Nothing to intercept — normal dispatch path.
        return dispatch(
            &state,
            &headers,
            &body,
            &provider,
            RequestFormat::OpenAIChat,
        )
        .await;
    }

    // Force non-streaming for internal loop iterations.
    let was_streaming = current_body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    if let Some(obj) = current_body.as_object_mut() {
        obj.insert("stream".to_string(), json!(false));
    }

    let max_iter = state.mcp.mcp_max_iterations.max(1);

    for iteration in 0..max_iter {
        let response = dispatch(
            &state,
            &headers,
            &current_body,
            &provider,
            RequestFormat::OpenAIChat,
        )
        .await;

        let (status, response_body) = match extract_response_json(response).await {
            Ok(parts) => parts,
            Err(fallback) => {
                // Could not parse JSON (likely an upstream error response).
                // Return as-is rather than swallowing it.
                tracing::warn!(
                    iteration,
                    "MCP loop: response was not JSON, returning as-is"
                );
                return fallback;
            }
        };

        let mcp_calls = mcp_tools::detect_mcp_tool_calls(&response_body);
        if mcp_calls.is_empty() {
            // No further MCP tool calls — return the final response.
            if iteration > 0 {
                tracing::info!(
                    iteration,
                    "MCP tool loop completed, returning final response"
                );
            }
            if was_streaming {
                tracing::info!(iteration, "MCP loop completed, returning as SSE stream");
                return crate::proxy::stream::sse_single_chunk_response(
                    status,
                    &response_body.to_string(),
                );
            }
            return json_response(status, response_body.to_string());
        }

        tracing::info!(
            iteration,
            calls = mcp_calls.len(),
            "MCP tool calls detected, executing"
        );
        let tool_results =
            mcp_tools::execute_mcp_tool_calls(&mcp_calls, &state.mcp.mcp_manager).await;
        current_body =
            mcp_tools::build_followup_request(&current_body, &response_body, &tool_results);
    }

    tracing::warn!(
        max_iter,
        "MCP tool loop exhausted iterations without a terminal response; dispatching final without tools"
    );
    // Strip tools to nudge the model towards a text response.
    if let Some(obj) = current_body.as_object_mut() {
        obj.remove("tools");
        obj.remove("tool_choice");
        // Restore the client's original streaming preference so the final
        // response is delivered as SSE if they asked for it.
        if was_streaming {
            obj.insert("stream".to_string(), json!(true));
        }
    }
    dispatch(
        &state,
        &headers,
        &current_body,
        &provider,
        RequestFormat::OpenAIChat,
    )
    .await
}

/// Buffer an axum `Response` body and parse it as JSON.
///
/// Returns `Ok((status, body))` on success, or `Err(fallback_response)`
/// when the body cannot be collected or parsed. The caller should
/// return the fallback response untouched.
async fn extract_response_json(
    response: axum::response::Response,
) -> Result<(reqwest::StatusCode, Value), axum::response::Response> {
    let status = response.status();
    let fallback_status = status;
    let bytes = match axum::body::to_bytes(
        response.into_body(),
        mcp_tools::max_response_body_bytes(),
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "MCP loop: failed to buffer response body");
            let body = json!({
                "error": {
                    "message": "MCP loop: failed to buffer upstream response",
                    "type": "mcp_loop_error",
                }
            });
            return Err(json_response(
                reqwest::StatusCode::BAD_GATEWAY,
                body.to_string(),
            ));
        }
    };

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            // Not JSON — reconstruct a fallback response carrying the raw body.
            let raw = String::from_utf8_lossy(&bytes).to_string();
            return Err(json_response(fallback_status, raw));
        }
    };

    Ok((status, value))
}

/// Convert model capabilities to a JSON object suitable for API responses.
fn capabilities_to_json(caps: &ModelCapabilities) -> serde_json::Value {
    serde_json::json!({
        "supports_vision": caps.supports_vision,
        "supports_tools": caps.supports_tools,
        "supports_thinking": caps.supports_thinking,
        "max_context_tokens": caps.max_context_tokens,
        "thinking_format": format!("{:?}", caps.thinking_format).to_lowercase(),
    })
}

/// Infer the owning provider from a model ID prefix.
///
/// Used for registry-only models that are not mapped to any specific channel.
fn infer_owner(model_id: &str) -> &'static str {
    if model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
    {
        "openai"
    } else if model_id.starts_with("claude-") {
        "anthropic"
    } else if model_id.starts_with("gemini-") {
        "google"
    } else if model_id.starts_with("deepseek") {
        "deepseek"
    } else {
        "unknown"
    }
}

/// List available models from all enabled channels and the model registry.
///
/// Aggregates model names from:
/// 1. Channel `model_mapping` keys (static config)
/// 2. `ModelRegistry` entries (includes dynamically discovered models from
///    `models_endpoint` and built-in capability profiles)
///
/// Each model entry includes a `capabilities` extension when capability data
/// is available. Pass-through channels (empty `model_mapping`) are covered
/// via the registry, which holds discovered models.
pub async fn handle_list_models(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let channels = state.channel_mgr.list().await;
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<serde_json::Value> = Vec::new();

    // Acquire registry read lock once for all lookups.
    let registry = state.model_registry.read();

    // 1. Models from channel model_mapping keys (static config).
    for ch in &channels {
        if !ch.enabled {
            continue;
        }
        for alias in ch.model_mapping.keys() {
            if seen.insert(alias.clone()) {
                let caps = registry.get(alias);
                models.push(serde_json::json!({
                    "id": alias,
                    "object": "model",
                    "created": ch.created_at.timestamp(),
                    "owned_by": ch.provider.as_str(),
                    "capabilities": capabilities_to_json(&caps),
                }));
            }
        }
    }

    // 2. Models from the registry (dynamically discovered + built-in profiles).
    //    This covers pass-through channels whose model_mapping is empty.
    for model_id in registry.list_models() {
        if seen.insert(model_id.clone()) {
            let caps = registry.get(&model_id);
            models.push(serde_json::json!({
                "id": model_id,
                "object": "model",
                "created": 0,
                "owned_by": infer_owner(&model_id),
                "capabilities": capabilities_to_json(&caps),
            }));
        }
    }

    let body = serde_json::json!({
        "object": "list",
        "data": models,
    });
    crate::proxy::stream::json_response(axum::http::StatusCode::OK, body.to_string())
}

/// Get detailed information for a single model.
///
/// `GET /v1/models/{model_id}`
///
/// Returns the model ID, object type, and capability metadata from the
/// model registry. Capabilities are resolved via exact match then
/// longest-prefix match, so dated variants like `gpt-4o-2024-08-06`
/// resolve to the correct profile.
pub async fn handle_get_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> axum::response::Response {
    let registry = state.model_registry.read();
    let caps = registry.get(&model_id);

    let body = serde_json::json!({
        "id": model_id,
        "object": "model",
        "created": 0,
        "owned_by": infer_owner(&model_id),
        "capabilities": capabilities_to_json(&caps),
    });

    crate::proxy::stream::json_response(axum::http::StatusCode::OK, body.to_string())
}

/// List all MCP tools exposed to LLM clients in OpenAI function-calling format.
///
/// `GET /v1/tools`
///
/// Aggregates tools from every running MCP server whose config has
/// `expose_tools == true`, namespacing each tool's name as
/// `mcp__{server_id}__{original_name}`. Returns an OpenAI-style list
/// envelope so any client that supports custom function calling can
/// discover the available MCP tools without a separate discovery protocol.
pub async fn handle_list_tools(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let mut tools = crate::mcp::aggregator::aggregate_all_tools(&state.mcp.mcp_manager).await;

    // Respect the per-server `expose_tools` flag so admins can run private
    // MCP servers without leaking their tools to LLM clients. Resolve the
    // flag once per unique server_id (closures can't be async, so we
    // precompute the decision map before the synchronous retain).
    let unique_server_ids: std::collections::HashSet<String> =
        tools.iter().map(|t| t.server_id.clone()).collect();
    let mut exposed: std::collections::HashMap<String, bool> =
        std::collections::HashMap::with_capacity(unique_server_ids.len());
    for id in unique_server_ids {
        let is_exposed = state
            .mcp
            .mcp_manager
            .get_config(&id)
            .await
            .map(|c| c.expose_tools)
            .unwrap_or(false);
        exposed.insert(id, is_exposed);
    }
    tools.retain(|t| *exposed.get(&t.server_id).unwrap_or(&false));

    let openai_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(crate::mcp::translator::to_openai_function)
        .collect();

    let body = serde_json::json!({
        "object": "list",
        "data": openai_tools,
    });

    crate::proxy::stream::json_response(axum::http::StatusCode::OK, body.to_string())
}

/// Health check endpoint returning JSON with version, uptime, and channel stats.
pub async fn health_check(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let channels = state.channel_mgr.list().await;
    let total = channels.len();
    let healthy = channels
        .iter()
        .filter(|c| c.status == crate::channel::ChannelStatus::Healthy && c.enabled)
        .count();
    let uptime_secs = state.started_at.elapsed().as_secs();

    let body = serde_json::json!({
        "status": if healthy > 0 { "ok" } else { "degraded" },
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
        "channels": {
            "total": total,
            "healthy": healthy,
        }
    });
    crate::proxy::stream::json_response(axum::http::StatusCode::OK, body.to_string())
}
