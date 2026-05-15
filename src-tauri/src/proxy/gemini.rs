use crate::proxy::openai::AppState;
use crate::proxy::translate::openai_to_gemini;
use crate::proxy::{dispatch, AuthStyle, ProxyConfig};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::Value;
use std::sync::Arc;

/// Handle Gemini-compatible /v1beta/models/{model}:generateContent requests.
/// Translates OpenAI-format incoming requests to Gemini format, forwards via
/// shared dispatch (which handles channel selection, retry, circuit breaking,
/// rate limiting, session affinity, caching, and payload rules), then
/// the response is returned in Gemini format from upstream.
pub async fn handle_gemini(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if let Err(resp) = crate::proxy::validate_chat_request(&body) {
        return resp;
    }
    // Translate request body to Gemini format
    let gemini_body = openai_to_gemini(&body);

    // Call shared dispatch with GeminiUrl auth style (key embedded in URL)
    dispatch(
        &state,
        &headers,
        &gemini_body,
        &ProxyConfig {
            default_model: "gemini-pro",
            upstream_path: "",
            auth_style: AuthStyle::GeminiUrl,
        },
    )
    .await
}
