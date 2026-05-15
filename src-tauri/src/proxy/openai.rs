use crate::channel::manager::ChannelManager;
use crate::credential::SharedCredentialStore;
use crate::log::DispatchLogger;
use crate::proxy::cache::{InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::{dispatch, AuthStyle, ProxyConfig};
use crate::quota::SharedQuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Shared application state for the proxy.
pub struct AppState {
    pub channel_mgr: Arc<ChannelManager>,
    pub credential_store: SharedCredentialStore,
    pub admin_token: Option<String>,
    pub request_timeout_secs: Option<u64>,
    pub stream_keepalive_secs: Option<u64>,
    pub logger: Arc<DispatchLogger>,
    pub http_client: reqwest::Client,
    pub max_retries: u32,
    pub model_fallbacks: HashMap<String, Vec<String>>,
    pub routing_strategy: String,
    pub session_affinity: SessionAffinity,
    pub active_requests: Arc<ActiveRequests>,
    pub request_cache: Arc<RequestCache>,
    pub payload_rules: Arc<ChannelPayloadRules>,
    pub rate_limiter: Arc<RateLimiter>,
    pub quota_store: SharedQuotaStore,
    pub in_flight: Arc<InFlightRequests>,
    pub started_at: std::time::Instant,
}

/// Handle OpenAI-compatible /v1/chat/completions requests.
pub async fn handle_chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if let Err(resp) = crate::proxy::validate_chat_request(&body) {
        return resp;
    }
    dispatch(
        &state,
        &headers,
        &body,
        &ProxyConfig {
            default_model: "gpt-4",
            upstream_path: "v1/chat/completions",
            auth_style: AuthStyle::OpenAI,
        },
    )
    .await
}

/// List available models from all enabled channels.
/// Aggregates model names from channel model_mapping keys, deduplicates,
/// and returns OpenAI-format model list.
pub async fn handle_list_models(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let channels = state.channel_mgr.list().await;
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<serde_json::Value> = Vec::new();

    for ch in &channels {
        if !ch.enabled {
            continue;
        }
        for (alias, _) in &ch.model_mapping {
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
