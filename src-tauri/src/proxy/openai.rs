use crate::channel::manager::ChannelManager;
use crate::credential::SharedCredentialStore;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::proxy::cache::{InFlightRequests, RequestCache};
use crate::proxy::mcp_tools;
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::stream::json_response;
use crate::proxy::{dispatch, provider::OpenAIAdaptor};
use crate::quota::SharedQuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::virtual_key::SharedVirtualKeyStore;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Gateway parameters (timeouts, retries, fallback strategy).
pub struct GatewayParams {
    pub request_timeout_secs: Option<u64>,
    pub stream_keepalive_secs: Option<u64>,
    pub stream_ttft_timeout_secs: Option<u64>,
    pub max_retries: u32,
    pub model_fallbacks: HashMap<String, Vec<String>>,
    pub routing_strategy: String,
    pub retry_base_ms: u64,
    pub retry_max_ms: u64,
    pub model_retry_overrides: HashMap<String, crate::config::ModelRetryConfig>,
}

/// Router state (session affinity, active request tracking, latency tracking).
pub struct RouterState {
    pub session_affinity: SessionAffinity,
    pub active_requests: Arc<ActiveRequests>,
    pub latency_tracker: Arc<crate::router::latency_tracker::LatencyTracker>,
}

/// Cache state (request cache + coalescing).
pub struct CacheState {
    pub request_cache: Arc<RequestCache>,
    pub in_flight: Arc<InFlightRequests>,
}

/// Limits state (rate limiter + payload rules).
pub struct LimitsState {
    pub payload_rules: Arc<ChannelPayloadRules>,
    pub rate_limiter: Arc<RateLimiter>,
}

/// Billing state (quota + virtual key + provider budget tracking).
pub struct BillingState {
    pub quota_store: SharedQuotaStore,
    pub virtual_key_store: SharedVirtualKeyStore,
    pub provider_budgets: crate::provider_budget::SharedProviderBudgetStore,
}

/// MCP integration state.
pub struct McpState {
    pub mcp_manager: Arc<McpManager>,
    pub mcp_max_iterations: u32,
    pub mcp_auto_inject: bool,
    pub mcp_gateway_enabled: bool,
}

/// Security state (auth + sanitizer).
pub struct SecurityState {
    pub admin_token: Option<String>,
    pub sanitizer_config: crate::config::SanitizerConfig,
}

/// Shared application state for the proxy.
pub struct AppState {
    pub channel_mgr: Arc<ChannelManager>,
    pub credential_store: SharedCredentialStore,
    pub logger: Arc<DispatchLogger>,
    pub http_pool: crate::http_pool::HttpPool,
    pub gateway: GatewayParams,
    pub router: RouterState,
    pub cache: CacheState,
    pub limits: LimitsState,
    pub billing: BillingState,
    pub mcp: McpState,
    pub security: SecurityState,
    pub started_at: std::time::Instant,
}

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
        return dispatch(&state, &headers, &body, &provider).await;
    }

    let (mut current_body, injected) =
        mcp_tools::inject_mcp_tools(&body, &state.mcp.mcp_manager).await;
    if injected.is_empty() {
        // Nothing to intercept — normal dispatch path.
        return dispatch(&state, &headers, &body, &provider).await;
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
        let response = dispatch(&state, &headers, &current_body, &provider).await;

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
    dispatch(&state, &headers, &current_body, &provider).await
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

/// List available models from all enabled channels.
/// Aggregates model names from channel model_mapping keys, deduplicates,
/// and returns OpenAI-format model list.
pub async fn handle_list_models(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let channels = state.channel_mgr.list().await;
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<serde_json::Value> = Vec::new();

    for ch in &channels {
        if !ch.enabled {
            continue;
        }
        for alias in ch.model_mapping.keys() {
            if seen.insert(alias.clone()) {
                models.push(serde_json::json!({
                    "id": alias,
                    "object": "model",
                    "created": ch.created_at.timestamp(),
                    "owned_by": ch.provider.as_str(),
                }));
            }
        }
    }

    let body = serde_json::json!({
        "object": "list",
        "data": models,
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
