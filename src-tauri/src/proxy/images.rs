use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;

use crate::channel::Provider;
use crate::proxy::stream::json_response;
use crate::proxy::AppState;
use crate::proxy::SKIP_HEADERS;

/// Apply the appropriate authorization header based on the provider.
fn apply_auth(
    mut builder: reqwest::RequestBuilder,
    provider: &Provider,
    api_key: &str,
) -> reqwest::RequestBuilder {
    builder = builder.header("Content-Type", "application/json");
    match provider {
        Provider::OpenAI => {
            builder = builder.header("Authorization", format!("Bearer {}", api_key));
        }
        Provider::Anthropic => {
            builder = builder.header("x-api-key", api_key);
        }
        Provider::Gemini => {
            builder = builder.header("x-goog-api-key", api_key);
        }
        _ => {
            builder = builder.header("Authorization", format!("Bearer {}", api_key));
        }
    }
    builder
}

/// Forward original request headers (skip hop-by-hop and auth).
fn forward_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
) -> reqwest::RequestBuilder {
    for (name, value) in headers.iter() {
        if !SKIP_HEADERS.contains(&name.as_str()) {
            if let (Ok(hn), Ok(hv)) = (
                axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
                axum::http::HeaderValue::from_str(value.to_str().unwrap_or("")),
            ) {
                builder = builder.header(hn, hv);
            }
        }
    }
    builder
}

/// Handle /v1/images/generations — forwards to an image-capable channel upstream.
/// Supports OpenAI DALL-E format: { model, prompt, n, size, quality, response_format }
pub async fn handle_image_generation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    // Validate JSON body is an object
    if !body.is_object() {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": {
                    "message": "Request body must be a JSON object",
                    "type": "invalid_request_error",
                    "code": "invalid_body"
                }
            })
            .to_string(),
        );
    }

    // Check if image generation is disabled
    if state.gateway.disable_image_generation {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "error": {
                    "message": "Image generation is disabled",
                    "type": "invalid_request_error",
                    "code": "image_generation_disabled"
                }
            })
            .to_string(),
        );
    }

    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("dall-e-3");

    // Find a channel that supports this model via model_mapping
    let (channel, api_key) = match find_image_channel(&state, model).await {
        Some(result) => result,
        None => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "error": {
                        "message": format!("No available channel for image model '{}'", model),
                        "type": "server_error",
                        "code": "no_available_channel"
                    }
                })
                .to_string(),
            );
        }
    };

    // Find the upstream model name from channel's model_mapping
    let upstream_model = channel
        .model_mapping
        .get(model)
        .cloned()
        .unwrap_or_else(|| model.to_string());

    // Build the upstream URL
    let base_url = channel.base_url.trim_end_matches('/');
    let url = format!("{}/v1/images/generations", base_url);

    // Clone body and insert the upstream model name
    let mut upstream_body = body.clone();
    if let Some(obj) = upstream_body.as_object_mut() {
        obj.insert("model".to_string(), Value::String(upstream_model));
    }

    // Build and send the request
    let client = state.http_pool.get();

    // Build request with headers then send
    let req_builder = client.post(&url).json(&upstream_body);
    let req_builder = apply_auth(req_builder, &channel.provider, &api_key);
    let req_builder = forward_headers(req_builder, &headers);

    // Send request upstream
    let resp = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(channel = %channel.name, error = %e, "Image generation request failed");
            return json_response(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "error": {
                        "message": "Upstream image generation request failed",
                        "type": "server_error",
                        "code": "upstream_error"
                    }
                })
                .to_string(),
            );
        }
    };

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    json_response(status, body_text)
}

/// Handle /v1/images/edits — returns 501 because OpenAI requires multipart/form-data
/// for this endpoint, which is not yet supported.
pub async fn handle_image_edits(
    State(_state): State<Arc<AppState>>,
    _headers: HeaderMap,
    Json(_body): Json<Value>,
) -> axum::response::Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        serde_json::json!({
            "error": {
                "message": "Image edits require multipart/form-data which is not yet supported. Use /v1/images/generations instead.",
                "type": "invalid_request_error",
                "code": "not_implemented"
            }
        })
        .to_string(),
    )
}

/// Find the best available channel for image generation.
async fn find_image_channel(
    state: &Arc<AppState>,
    model: &str,
) -> Option<(crate::channel::Channel, String)> {
    let channels = state.channel_mgr.channels();
    let guard = channels.read().await;

    // Collect matching channel IDs without holding any parking_lot guard across .await
    let mut candidates: Vec<uuid::Uuid> = Vec::new();
    let mut fallback: Vec<uuid::Uuid> = Vec::new();

    for ch_arc in guard.values() {
        let ch = ch_arc.read();
        if !ch.enabled || !ch.is_available() {
            continue;
        }

        let has_model = ch.model_mapping.contains_key(model)
            || ch.model_mapping.values().any(|v| v == model)
            || model.starts_with(ch.name.as_str());

        if has_model {
            candidates.push(ch.id);
        }

        let has_image_mapping = ch
            .model_mapping
            .keys()
            .any(|k| k.contains("dall-e") || k.contains("image") || k.contains("imagen"));

        if has_image_mapping {
            fallback.push(ch.id);
        }
    }

    // Drop the tokio RwLock guard before awaiting credentials
    drop(guard);

    // Try exact-match candidates first
    for id in &candidates {
        if let Some(api_key) = state.channel_mgr.get_credential(*id).await {
            if let Some(channel) = state.channel_mgr.get(*id).await {
                return Some((channel, api_key));
            }
        }
    }

    // Try fallback candidates
    for id in &fallback {
        if let Some(api_key) = state.channel_mgr.get_credential(*id).await {
            if let Some(channel) = state.channel_mgr.get(*id).await {
                return Some((channel, api_key));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Provider;
    use axum::http::{HeaderName, HeaderValue};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── apply_auth tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn apply_auth_openai_sets_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::OpenAI, "sk-test");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    /// Helper — wiremock matcher that succeeds when none of the named headers
    /// are present on the incoming request.
    fn headers_absent(names: &'static [&'static str]) -> impl wiremock::Match {
        struct HeadersAbsent(&'static [&'static str]);
        impl wiremock::Match for HeadersAbsent {
            fn matches(&self, request: &wiremock::Request) -> bool {
                self.0.iter().all(|h| !request.headers.contains_key(*h))
            }
        }
        HeadersAbsent(names)
    }

    #[tokio::test]
    async fn apply_auth_anthropic_sets_x_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("x-api-key", "sk-ant-test"))
            .and(headers_absent(&["authorization"]))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::Anthropic, "sk-ant-test");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn apply_auth_gemini_sets_x_goog_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("x-goog-api-key", "AIza-test"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::Gemini, "AIza-test");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn apply_auth_custom_sets_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::Custom("acme".to_string()), "test-key");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn apply_auth_deepseek_uses_bearer_default() {
        // Any provider not specifically OpenAI/Anthropic/Gemini falls into the
        // catch-all arm and should receive a Bearer token.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("authorization", "Bearer dk-test"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::DeepSeek, "dk-test");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn apply_auth_always_sets_content_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = apply_auth(builder, &Provider::OpenAI, "sk-test");
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    // ── forward_headers tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn forward_headers_includes_custom_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .and(header("x-custom-header", "custom-value"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-custom-header"),
            HeaderValue::from_static("custom-value"),
        );

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = forward_headers(builder, &headers);
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn forward_headers_skips_hop_by_hop() {
        // Verify via received_requests that hop-by-hop / managed headers from
        // the input HeaderMap are NOT forwarded to the upstream, while custom
        // headers are. We cannot assert absence at the mock-matcher level
        // because reqwest injects its own `host` and `content-type` headers
        // based on the URL and body — we only check the headers we explicitly
        // placed in the input HeaderMap.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/test"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut headers = HeaderMap::new();
        // Headers that must be stripped by forward_headers
        headers.insert(
            HeaderName::from_static("connection"),
            HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            HeaderName::from_static("content-length"),
            HeaderValue::from_static("42"),
        );
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer secret"),
        );
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("sk-secret"),
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
        // A benign custom header that SHOULD be forwarded
        headers.insert(
            HeaderName::from_static("x-trace-id"),
            HeaderValue::from_static("abc"),
        );

        let client = reqwest::Client::new();
        let builder = client.post(format!("{}/test", server.uri()));
        let builder = forward_headers(builder, &headers);
        let resp = builder.send().await.unwrap();
        assert_eq!(resp.status(), 200);

        let received = server.received_requests().await;
        let received = received.expect("should have received requests");
        assert_eq!(received.len(), 1, "exactly one request expected");
        let req = &received[0];

        // Custom header must be present
        let trace = req.headers.get("x-trace-id").and_then(|v| v.to_str().ok());
        assert_eq!(trace, Some("abc"), "custom headers should be forwarded");

        // These managed/hop-by-hop headers from our input must NOT appear in
        // the upstream request. (host/content-type may appear due to reqwest's
        // own defaults, so we check only the ones we controlled.)
        for blocked in &[
            "connection",
            "content-length",
            "authorization",
            "x-api-key",
            "anthropic-version",
        ] {
            assert!(
                !req.headers.contains_key(*blocked),
                "header '{}' should have been stripped by forward_headers",
                blocked
            );
        }
    }

    #[test]
    fn forward_headers_constant_covers_critical_set() {
        // Sanity-check the SKIP_HEADERS list that forward_headers relies on.
        // This guards against accidental removal of a critical filtered header.
        assert!(SKIP_HEADERS.contains(&"host"));
        assert!(SKIP_HEADERS.contains(&"connection"));
        assert!(SKIP_HEADERS.contains(&"content-length"));
        assert!(SKIP_HEADERS.contains(&"authorization"));
        assert!(SKIP_HEADERS.contains(&"x-api-key"));
        assert!(SKIP_HEADERS.contains(&"content-type"));
    }
}
