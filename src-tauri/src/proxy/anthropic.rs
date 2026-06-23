use crate::proxy::dispatch;
use crate::proxy::provider::AnthropicAdaptor;
use crate::proxy::AppState;
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
    let provider = AnthropicAdaptor;
    dispatch(&state, &headers, &body, &provider).await
}
