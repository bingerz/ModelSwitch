use crate::proxy::openai::AppState;
use crate::proxy::{dispatch, AuthStyle, ProxyConfig};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::Value;
use std::sync::Arc;

/// Handle Anthropic-compatible /v1/messages requests.
pub async fn handle_messages(
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
            default_model: "claude-3-5-sonnet-20241022",
            upstream_path: "v1/messages",
            auth_style: AuthStyle::Anthropic,
        },
    )
    .await
}
