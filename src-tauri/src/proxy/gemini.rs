use crate::proxy::openai::AppState;
use crate::proxy::provider::GeminiAdaptor;
use crate::proxy::dispatch;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::Value;
use std::sync::Arc;

/// Handle Gemini-compatible /v1beta/models/{model}:generateContent requests.
///
/// The incoming request is in OpenAI format. The `GeminiAdaptor` trait
/// implementation handles format translation (`transform_request` /
/// `transform_response`) inside the dispatch pipeline, so the handler
/// passes the original body and lets the provider adaptor handle the rest.
pub async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if let Err(resp) = crate::proxy::validate_chat_request(&body) {
        return resp;
    }
    let provider = GeminiAdaptor;
    dispatch(&state, &headers, &body, &provider).await
}
