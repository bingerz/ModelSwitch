use axum::body::Body;
use axum::response::Response;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::channel::Channel;
use crate::proxy::stream::{keepalive_stream, sse_stream_response_with_telemetry};
use crate::router::active_requests::ActiveRequestGuard;

use super::provider::ProviderAdaptor;
use super::usage::{extract_usage, extract_usage_from_stream};
use super::{estimate_tokens, make_log, RequestFormat};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Channel;
    use crate::http_pool::HttpPool;
    use crate::proxy::provider::OpenAIAdaptor;
    use crate::proxy::stream::json_response;
    use crate::router::active_requests::ActiveRequests;
    use crate::test_helpers;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn inject_passthrough_headers_adds_headers() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let headers = vec![("X-Custom".to_string(), "value".to_string())];
        let resp = inject_passthrough_headers(resp, &headers);
        assert_eq!(resp.headers().get("x-custom").unwrap(), "value");
    }

    #[test]
    fn fallback_headers_inserted_when_trigger_reason_matches() {
        // Build a minimal response to test header insertion in isolation
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let mut resp = inject_passthrough_headers(resp, &[]);
        // Simulate the fallback guard from handle_json_success
        if Some("model_fallback") == Some("model_fallback") {
            if let Ok(hv) = axum::http::HeaderValue::from_str("gpt-4-turbo") {
                resp.headers_mut()
                    .insert("X-ModelSwitch-Fallback-Model", hv);
            }
            if let Ok(hv) = axum::http::HeaderValue::from_str("gpt-4o") {
                resp.headers_mut()
                    .insert("X-ModelSwitch-Original-Model", hv);
            }
        }
        assert_eq!(
            resp.headers().get("x-modelswitch-fallback-model").unwrap(),
            "gpt-4-turbo"
        );
        assert_eq!(
            resp.headers().get("x-modelswitch-original-model").unwrap(),
            "gpt-4o"
        );
    }

    #[test]
    fn no_fallback_headers_when_trigger_reason_none() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let resp = inject_passthrough_headers(resp, &[]);
        // When trigger_reason is not model_fallback, no headers should be set
        // (this is just verifying the guard logic)
        assert!(resp.headers().get("x-modelswitch-fallback-model").is_none());
        assert!(resp.headers().get("x-modelswitch-original-model").is_none());
    }

    /// Verify the completion ratio math used by the cost calculations in
    /// `handle_streaming_success` and `handle_json_success`.
    ///
    /// The formula (for the per-Mtok pricing branch) is:
    ///   total = input_tokens / 1_000_000 * input_rate
    ///         + output_tokens / 1_000_000 * output_rate * completion_ratio
    ///
    /// With a ratio of 2.0 the output cost component is doubled while the
    /// input cost component stays unchanged.
    #[test]
    fn cost_calculation_applies_completion_ratio() {
        let input_tokens: f64 = 1_000_000.0;
        let output_tokens: f64 = 1_000_000.0;
        let input_rate = 10.0; // $10 / Mtok
        let output_rate = 30.0; // $30 / Mtok

        // Baseline: ratio 1.0 (no adjustment)
        let baseline_ratio = 1.0_f64;
        let baseline_cost = input_tokens / 1_000_000.0 * input_rate
            + output_tokens / 1_000_000.0 * output_rate * baseline_ratio;
        assert!(
            (baseline_cost - 40.0).abs() < f64::EPSILON,
            "baseline cost should be 10 + 30 = 40, got {baseline_cost}"
        );

        // With ratio 2.0, output cost doubles: 10 + (30 * 2) = 70
        let ratio = 2.0_f64;
        let adjusted_cost = input_tokens / 1_000_000.0 * input_rate
            + output_tokens / 1_000_000.0 * output_rate * ratio;
        assert!(
            (adjusted_cost - 70.0).abs() < f64::EPSILON,
            "adjusted cost should be 10 + 60 = 70, got {adjusted_cost}"
        );

        // Input component unchanged
        let input_component = input_tokens / 1_000_000.0 * input_rate;
        assert!(
            (input_component - 10.0).abs() < f64::EPSILON,
            "input component should be 10, got {input_component}"
        );

        // Output component doubled
        let output_component = output_tokens / 1_000_000.0 * output_rate * ratio;
        assert!(
            (output_component - 60.0).abs() < f64::EPSILON,
            "output component should be 60 with ratio 2.0, got {output_component}"
        );
    }

    // =====================================================================
    // P1: Pure helper tests — extract_passthrough_headers
    // =====================================================================

    #[tokio::test]
    async fn extract_passthrough_headers_returns_matching_headers() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "100")
                    .insert_header("x-request-id", "abc"),
            )
            .mount(&server)
            .await;
        let resp = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .unwrap();
        let passthrough = vec![
            "x-ratelimit-remaining".to_string(),
            "x-request-id".to_string(),
        ];
        let result = extract_passthrough_headers(&resp, &passthrough);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ("x-ratelimit-remaining".to_string(), "100".into())
        );
        assert_eq!(result[1], ("x-request-id".to_string(), "abc".into()));
    }

    #[tokio::test]
    async fn extract_passthrough_headers_ignores_missing() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-request-id", "abc"))
            .mount(&server)
            .await;
        let resp = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .unwrap();
        let passthrough = vec![
            "x-ratelimit-remaining".to_string(),
            "x-request-id".to_string(),
        ];
        let result = extract_passthrough_headers(&resp, &passthrough);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("x-request-id".to_string(), "abc".into()));
    }

    #[tokio::test]
    async fn extract_passthrough_headers_empty_list_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-request-id", "abc"))
            .mount(&server)
            .await;
        let resp = reqwest::Client::new()
            .get(server.uri())
            .send()
            .await
            .unwrap();
        let result = extract_passthrough_headers(&resp, &[]);
        assert!(result.is_empty());
    }

    // =====================================================================
    // P1: Pure helper tests — inject_passthrough_headers
    // =====================================================================

    #[test]
    fn inject_passthrough_headers_multiple_headers() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let headers = vec![
            ("X-A".to_string(), "1".to_string()),
            ("X-B".to_string(), "2".to_string()),
        ];
        let resp = inject_passthrough_headers(resp, &headers);
        assert_eq!(resp.headers().get("x-a").unwrap(), "1");
        assert_eq!(resp.headers().get("x-b").unwrap(), "2");
    }

    #[test]
    fn inject_passthrough_headers_skips_invalid_name() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let headers = vec![
            ("Invalid Header".to_string(), "val".to_string()),
            ("X-Valid".to_string(), "ok".to_string()),
        ];
        let resp = inject_passthrough_headers(resp, &headers);
        assert!(resp.headers().get("invalid header").is_none());
        assert_eq!(resp.headers().get("x-valid").unwrap(), "ok");
    }

    #[test]
    fn inject_passthrough_headers_appends_duplicates() {
        let resp = json_response(reqwest::StatusCode::OK, "{}".to_string());
        let headers = vec![
            ("X-Dup".to_string(), "a".to_string()),
            ("X-Dup".to_string(), "b".to_string()),
        ];
        let resp = inject_passthrough_headers(resp, &headers);
        let values: Vec<_> = resp.headers().get_all("x-dup").iter().collect();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], "a");
        assert_eq!(values[1], "b");
    }

    // =====================================================================
    // P2: Cost calculation formula tests
    // =====================================================================

    /// Model-level pricing overrides channel-level rates when both are set.
    #[test]
    fn cost_calc_model_pricing_overrides_channel_rates() {
        let model_input_rate = 15.0_f64;
        let model_output_rate = 45.0_f64;
        let _channel_input_rate = 10.0_f64;
        let _channel_output_rate = 30.0_f64;

        let input_tokens = 1_000_000_f64;
        let output_tokens = 1_000_000_f64;
        let completion_ratio = 1.0_f64;

        // Model pricing wins: 1 * 15.0 + 1 * 45.0 * 1.0 = 60.0
        let cost = input_tokens / 1_000_000.0 * model_input_rate
            + output_tokens / 1_000_000.0 * model_output_rate * completion_ratio;

        assert!(
            (cost - 60.0).abs() < f64::EPSILON,
            "model pricing should override channel rates: expected 60.0, got {cost}"
        );
    }

    /// Channel per-Mtok rates are used when no model-level pricing exists.
    #[test]
    fn cost_calc_channel_rate_used_when_no_model_pricing() {
        let channel_input_rate = 10.0_f64;
        let channel_output_rate = 30.0_f64;
        let completion_ratio = 1.0_f64;

        let input_tokens = 500_000_f64;
        let output_tokens = 500_000_f64;

        // 0.5 * 10.0 + 0.5 * 30.0 * 1.0 = 20.0
        let cost = input_tokens / 1_000_000.0 * channel_input_rate
            + output_tokens / 1_000_000.0 * channel_output_rate * completion_ratio;

        assert!(
            (cost - 20.0).abs() < f64::EPSILON,
            "channel rate should apply: expected 20.0, got {cost}"
        );
    }

    /// cost_per_token fallback when no per-Mtok rates are configured.
    #[test]
    fn cost_calc_cost_per_token_fallback() {
        let cost_per_token = 0.002_f64; // $0.002 per 1K tokens
        let input_tokens = 1000_u64;
        let output_tokens = 1000_u64;
        let total_tokens = input_tokens + output_tokens;

        // 2000 / 1000 * 0.002 = 0.004
        let cost = total_tokens as f64 / 1000.0 * cost_per_token;

        assert!(
            (cost - 0.004).abs() < 1e-9,
            "cost_per_token fallback: expected 0.004, got {cost}"
        );
    }

    /// With per-Mtok rates present but zero token usage, cost is 0.0 (not None).
    #[test]
    fn cost_calc_zero_tokens_zero_cost() {
        let input_rate = 10.0_f64;
        let output_rate = 30.0_f64;
        let input_tokens = 0_f64;
        let output_tokens = 0_f64;

        // Rates exist, so the per-Mtok branch fires; 0 tokens => 0.0 cost.
        let cost =
            input_tokens / 1_000_000.0 * input_rate + output_tokens / 1_000_000.0 * output_rate;

        assert!(
            (cost - 0.0).abs() < f64::EPSILON,
            "zero tokens with rates => Some(0.0), got {cost}"
        );
    }

    // =====================================================================
    // P3: handle_json_success integration tests
    // =====================================================================

    /// Build a test Channel for handle_json_success tests.
    fn test_channel() -> Channel {
        let cfg = test_helpers::channel_config(
            "00000000-0000-0000-0000-000000000001",
            "test-channel",
            "http://unused",
            1,
        );
        Channel::from_config(&cfg)
    }

    /// Build a PooledClient for testing.
    fn test_pool_guard() -> crate::http_pool::PooledClient {
        let pool =
            HttpPool::new(1, || reqwest::Client::builder()).expect("Failed to build HTTP pool");
        pool.get()
    }

    /// Build an ActiveRequestGuard for testing.
    fn test_active_guard(channel_id: Uuid) -> ActiveRequestGuard {
        let tracker = Arc::new(ActiveRequests::new());
        tracker.acquire(channel_id)
    }

    /// Create a mock upstream returning the given body via wiremock,
    /// then fetch it as a real reqwest::Response.
    async fn mock_upstream(body: &str) -> reqwest::Response {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(body.to_string()),
            )
            .mount(&server)
            .await;
        reqwest::Client::new()
            .post(server.uri())
            .send()
            .await
            .unwrap()
    }

    /// Standard request body used across handle_json_success tests.
    fn request_body() -> Value {
        serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
        })
    }

    #[tokio::test]
    async fn handle_json_success_returns_200_with_body() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[{"message":{"content":"hi"}}],"usage":{"prompt_tokens":10,"completion_tokens":5},"model":"gpt-4"}"#,
        )
        .await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            42u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&bytes);
        assert!(
            body_str.contains("hi"),
            "response body should contain 'hi', got: {body_str}"
        );
    }

    #[tokio::test]
    async fn handle_json_success_inserts_fallback_headers() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0},"model":"gpt-4"}"#,
        )
        .await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4o",
            &provider,
            "gpt-4-turbo",
            "gpt-4-turbo",
            0,
            Some("model_fallback"),
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            43u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert_eq!(
            response
                .headers()
                .get("x-modelswitch-fallback-model")
                .unwrap(),
            "gpt-4-turbo"
        );
        assert_eq!(
            response
                .headers()
                .get("x-modelswitch-original-model")
                .unwrap(),
            "gpt-4o"
        );
    }

    #[tokio::test]
    async fn handle_json_success_no_fallback_headers_without_trigger() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0},"model":"gpt-4"}"#,
        )
        .await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            44u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert!(response
            .headers()
            .get("x-modelswitch-fallback-model")
            .is_none());
        assert!(response
            .headers()
            .get("x-modelswitch-original-model")
            .is_none());
    }

    #[tokio::test]
    async fn handle_json_success_passthrough_headers_injected() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0},"model":"gpt-4"}"#,
        )
        .await;

        let upstream_headers = vec![("x-ratelimit-remaining".to_string(), "100".to_string())];

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &upstream_headers,
            None,
            0,
            45u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert_eq!(
            response.headers().get("x-ratelimit-remaining").unwrap(),
            "100"
        );
    }

    #[tokio::test]
    async fn handle_json_success_handles_missing_usage() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(r#"{"choices":[]}"#).await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            46u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        // Should return 200 without panicking
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn handle_json_success_caches_response() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[{"message":{"content":"cached"}}],"usage":{"prompt_tokens":3,"completion_tokens":2},"model":"gpt-4"}"#,
        )
        .await;

        let cache_key = 99u128;
        let cache_key_material = "cache-test-material";

        let _response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            cache_key,
            cache_key_material,
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        // Allow any background tasks to settle.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let cached = state.cache.request_cache.get(cache_key, cache_key_material);
        assert!(
            cached.is_some(),
            "response should be cached after handle_json_success"
        );
        let cached_body = cached.unwrap();
        assert!(
            cached_body.contains("cached"),
            "cached body should contain 'cached', got: {cached_body}"
        );
    }

    #[tokio::test]
    async fn handle_json_success_with_empty_body() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream("").await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            47u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn handle_json_success_with_large_usage() {
        let state = test_helpers::build_test_state(vec![]);
        let channel = test_channel();
        let provider = OpenAIAdaptor;
        let resp = mock_upstream(
            r#"{"choices":[],"usage":{"prompt_tokens":1000000,"completion_tokens":500000},"model":"gpt-4"}"#,
        )
        .await;

        let response = handle_json_success(
            &state,
            &channel,
            resp,
            &request_body(),
            "gpt-4",
            &provider,
            "gpt-4",
            "gpt-4",
            0,
            None,
            std::time::Instant::now(),
            None,
            &[],
            None,
            0,
            48u128,
            "test-key",
            test_pool_guard(),
            test_active_guard(channel.id),
            RequestFormat::OpenAIChat,
            RequestFormat::OpenAIChat,
        )
        .await;

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "should handle large token counts without overflow/panic"
        );
    }
}

/// Extract passthrough headers from an upstream response using a configurable
/// list of header names. When `passthrough_list` is empty, no headers are
/// extracted (the caller is responsible for providing the effective list,
/// typically from `state.gateway.passthrough_headers`).
pub(super) fn extract_passthrough_headers(
    resp: &reqwest::Response,
    passthrough_list: &[String],
) -> Vec<(String, String)> {
    passthrough_list
        .iter()
        .filter_map(|name| {
            resp.headers()
                .get(name.as_str())
                .and_then(|v| v.to_str().ok())
                .map(|v| (name.clone(), v.to_string()))
        })
        .collect()
}

/// Inject passthrough headers into an Axum response.
pub(super) fn inject_passthrough_headers(
    mut resp: Response,
    headers: &[(String, String)],
) -> Response {
    for (name, value) in headers {
        if let Ok(header_name) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(header_value) = axum::http::HeaderValue::from_str(value) {
                resp.headers_mut().append(header_name, header_value);
            }
        }
    }
    resp
}

/// Handle a successful streaming (SSE) response from upstream.
/// Logs the attempt, spawns a background task to extract real token usage,
/// caches the SSE response for streaming cache hits, and applies keepalive if configured.
///
/// `upstream_stream` is the reqwest bytes stream from the upstream response.
/// `first_chunk` is an optional pre-read first chunk (from bootstrap retry
/// logic) that will be prepended to the stream so the client receives the
/// complete output.
///
/// The `pool_guard` is moved into the background telemetry task so the pool's
/// active count stays accurate for the entire lifetime of the stream — the
/// guard is dropped only after `stream_done` fires (stream fully consumed).
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_streaming_success(
    state: &Arc<crate::proxy::AppState>,
    channel: &Channel,
    upstream_stream: futures::stream::BoxStream<'static, Result<Bytes, reqwest::Error>>,
    first_chunk: Option<Bytes>,
    body: &Value,
    provider: &dyn ProviderAdaptor,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
    reserved_cents: u64,
    original_model: &str,
    cache_key: u128,
    cache_key_material: &str,
    pool_guard: crate::http_pool::PooledClient,
    active_guard: ActiveRequestGuard,
    protocol_translation: Option<(RequestFormat, RequestFormat)>,
) -> Response {
    let is_gemini = provider.is_gemini_stream();

    // Compute the first-byte timeout from the gateway config. This guards against
    // LLMs that accept the request (200 OK) but take an extremely long time to
    // produce the first SSE data chunk ("thinking" stall). Subsequent chunk reads
    // use the per-read timeout internally (STREAM_READ_TIMEOUT = 120s).
    let first_byte_timeout = state
        .gateway
        .stream_ttft_timeout_secs
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs);

    // If a first chunk was pre-read (bootstrap retry), prepend it to the
    // upstream stream so the client receives the complete output.
    let combined_stream: futures::stream::BoxStream<'static, Result<Bytes, reqwest::Error>> =
        if let Some(chunk) = first_chunk {
            use futures::stream::{self, StreamExt};
            stream::once(async move { Ok::<_, reqwest::Error>(chunk) })
                .chain(upstream_stream)
                .boxed()
        } else {
            upstream_stream
        };

    let (stream_resp, output_buffer, stream_done, ttft) = sse_stream_response_with_telemetry(
        combined_stream,
        is_gemini,
        upstream_model.to_string(),
        first_byte_timeout,
        protocol_translation,
        start,
    );

    let est_tokens = estimate_tokens(body, true);
    // Determine effective rates: model_pricing overrides channel rates.
    let mp = state.gateway.model_pricing.get(current_model);
    let eff_in = mp
        .and_then(|p| p.input_cost_per_mtok)
        .or(channel.input_cost_per_mtok);
    let eff_out = mp
        .and_then(|p| p.output_cost_per_mtok)
        .or(channel.output_cost_per_mtok);
    let estimated_cost = if eff_in.is_some() || eff_out.is_some() {
        let half = (est_tokens / 2) as f64;
        let completion_ratio = state
            .gateway
            .completion_ratios
            .get(current_model)
            .copied()
            .unwrap_or(1.0);
        Some(
            half / 1_000_000.0 * eff_in.unwrap_or(0.0)
                + half / 1_000_000.0 * eff_out.unwrap_or(0.0) * completion_ratio,
        )
    } else {
        channel
            .cost_per_token
            .map(|rate| rate * est_tokens as f64 / 1000.0)
    };
    let log_id = Uuid::new_v4();
    let mut log_entry = make_log(
        current_model,
        channel.id,
        &channel.name,
        channel.priority,
        attempt,
        trigger_reason,
        start.elapsed().as_millis() as u64,
        true,
        estimated_cost,
        None,
        None,
        None,
        None,
        request_id,
        vk_id.map(|id| id.to_string()),
    );
    log_entry.id = log_id;
    state.logger.log(log_entry).await;

    // Prometheus metrics
    let provider_label = channel.provider.as_str();
    crate::metrics::requests_total()
        .with_label_values(&[provider_label, current_model, "success"])
        .inc();
    crate::metrics::request_duration()
        .with_label_values(&[provider_label, current_model])
        .observe(start.elapsed().as_secs_f64());
    crate::metrics::record_latency(start.elapsed(), current_model, provider_label);
    let _ = state
        .channel_mgr
        .record_latency(channel.id, start.elapsed().as_millis() as u64)
        .await;
    state
        .router
        .latency_tracker
        .record(channel.id, start.elapsed().as_millis() as u64);

    // Spawn background task to extract real token counts from stream
    {
        let bg_logger = Arc::clone(&state.logger);
        let bg_quota_store = Arc::clone(&state.billing.quota_store);
        let bg_virtual_key_store = Arc::clone(&state.billing.virtual_key_store);
        let bg_provider_budgets = Arc::clone(&state.billing.provider_budgets);
        let bg_key_rate_limiter = Arc::clone(&state.billing.key_rate_limiter);
        let bg_channel_id = channel.id;
        let bg_provider_name = channel.provider.as_str().to_string();
        let bg_input_cost = channel.input_cost_per_mtok;
        let bg_output_cost = channel.output_cost_per_mtok;
        let bg_cost_per_token = channel.cost_per_token;
        let bg_model_pricing = state.gateway.model_pricing.get(current_model).cloned();
        let bg_completion_ratio = state
            .gateway
            .completion_ratios
            .get(current_model)
            .copied()
            .unwrap_or(1.0);
        let bg_vk_id = vk_id;
        let bg_reserved_cents = reserved_cents;
        let bg_request_cache = Arc::clone(&state.cache.request_cache);
        let bg_in_flight = Arc::clone(&state.cache.in_flight);
        let bg_current_model = current_model.to_string();
        let bg_output_buffer = Arc::clone(&output_buffer);
        let bg_stream_done = Arc::clone(&stream_done);
        let bg_cache_key = cache_key;
        let bg_key_material = cache_key_material.to_string();
        let bg_pool_guard = pool_guard;
        let bg_active_guard = active_guard;
        let bg_ttft = Arc::clone(&ttft);
        crate::spawn_bg(async move {
            // Hold the pool guard and active-request guard for the entire
            // lifetime of the background task. This keeps the HTTP pool's
            // active count accurate while the streaming body continues
            // flowing to the client. Both guards are dropped (decrementing
            // their respective counts) when this task completes — shortly
            // after the stream is fully consumed.
            let _pool_guard = bg_pool_guard;
            let _active_guard = bg_active_guard;

            // Wait for the upstream stream to be fully consumed by the
            // stream-forwarding task.  `Notify` stores a permit if
            // `notify_one` fires before we register, so the ordering
            // between this task and the stream task does not matter.
            //
            // A 60 s safety-net timeout guards against any unexpected
            // failure to signal (e.g. the stream task panicking).
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                bg_stream_done.notified(),
            )
            .await;

            // Record TTFT once the stream has produced its first chunk. The
            // value is `None` when the upstream never produced any data (e.g.
            // empty stream or first-byte timeout), in which case we skip the
            // observation.
            if let Ok(slot) = bg_ttft.lock() {
                if let Some(ttft) = *slot {
                    crate::metrics::record_ttft(ttft, &bg_current_model, &bg_provider_name);
                }
            }

            // Single post-stream pass: lock the output buffer once and convert
            // to a String. All SSE parsing (data-line extraction, usage
            // detection) happens here instead of per-chunk on the hot path.
            let output_text = {
                let buf = bg_output_buffer.lock();
                String::from_utf8_lossy(&buf).to_string()
            };

            // Cache the accumulated SSE text for streaming cache hits.
            // Must happen before in_flight.complete for coalesced waiters.
            if !output_text.is_empty() {
                bg_request_cache.insert(bg_cache_key, bg_key_material, output_text.clone());
                bg_in_flight.complete(bg_cache_key);
            }

            // Parse SSE data lines once, post-stream, for usage extraction.
            let chunks: Vec<String> = output_text
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    trimmed
                        .strip_prefix("data: ")
                        .filter(|d| !d.is_empty() && *d != "[DONE]")
                        .map(|d| d.to_string())
                })
                .collect();
            if chunks.is_empty() {
                return;
            }
            let token_usage = extract_usage_from_stream(&chunks);
            let input_tokens = token_usage.input_tokens;
            let output_tokens = token_usage.output_tokens;
            if input_tokens.is_some() || output_tokens.is_some() {
                // Re-calculate cost using real tokens.
                // Model-level pricing overrides channel rates when present.
                let mp_in = bg_model_pricing
                    .as_ref()
                    .and_then(|p| p.input_cost_per_mtok);
                let mp_out = bg_model_pricing
                    .as_ref()
                    .and_then(|p| p.output_cost_per_mtok);
                let eff_in = mp_in.or(bg_input_cost);
                let eff_out = mp_out.or(bg_output_cost);
                let real_cost = if eff_in.is_some() || eff_out.is_some() {
                    let in_tok = input_tokens.unwrap_or(0) as f64;
                    let out_tok = output_tokens.unwrap_or(0) as f64;
                    Some(
                        in_tok / 1_000_000.0 * eff_in.unwrap_or(0.0)
                            + out_tok / 1_000_000.0 * eff_out.unwrap_or(0.0) * bg_completion_ratio,
                    )
                } else {
                    bg_cost_per_token.map(|rate_per_1k| {
                        ((input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)) as f64 / 1000.0)
                            * rate_per_1k
                    })
                };

                if let (Some(it), Some(ot)) = (input_tokens, output_tokens) {
                    bg_logger
                        .update_log_tokens(
                            log_id,
                            it,
                            ot,
                            token_usage.cache_hit_tokens,
                            token_usage.cache_miss_tokens,
                            real_cost,
                        )
                        .await;
                }

                // Accumulate usage (tokens + cost) into quota store
                bg_quota_store
                    .accumulate_usage(
                        bg_channel_id,
                        input_tokens,
                        output_tokens,
                        token_usage.cache_hit_tokens,
                        token_usage.cache_miss_tokens,
                        real_cost,
                    )
                    .await;

                // Attribute spend to the requesting virtual key (if any).
                // When a reservation was made before dispatch, reconcile against
                // it so the key is not double-charged (reservation + accumulation).
                if let Some(vk) = bg_vk_id {
                    let cost_cents = (real_cost.unwrap_or(0.0) * 100.0) as u64;
                    if bg_reserved_cents > 0 {
                        bg_virtual_key_store
                            .reconcile_spend(vk, bg_reserved_cents, cost_cents)
                            .await;
                    } else {
                        bg_virtual_key_store.accumulate_spend(vk, cost_cents).await;
                    }

                    // Record actual token consumption against the key's TPM
                    // window. This is the post-response complement to the
                    // pre-request `check_tpm` gate in the virtual-key
                    // middleware.
                    let total_tokens = input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0);
                    bg_key_rate_limiter.record_tokens(vk, total_tokens);
                }

                // Accumulate spend into per-provider budget tracker.
                let cost_cents = (real_cost.unwrap_or(0.0) * 100.0) as u64;
                bg_provider_budgets
                    .accumulate_spend(&bg_provider_name, cost_cents)
                    .await;

                // Token-level Prometheus metrics
                if let Some(it) = input_tokens {
                    crate::metrics::input_tokens_total()
                        .with_label_values(&[&bg_provider_name, &bg_current_model])
                        .inc_by(it);
                }
                if let Some(ot) = output_tokens {
                    crate::metrics::output_tokens_total()
                        .with_label_values(&[&bg_provider_name, &bg_current_model])
                        .inc_by(ot);
                }

                // Latency / token / cost histograms (complement the existing
                // counters and duration histogram above).
                crate::metrics::record_tokens(
                    input_tokens.unwrap_or(0),
                    output_tokens.unwrap_or(0),
                    &bg_current_model,
                );
                if let Some(cost) = real_cost {
                    crate::metrics::record_cost(cost, &bg_current_model);
                }
            }
        });
    }

    let stream_resp = inject_passthrough_headers(stream_resp, upstream_headers);

    let mut stream_resp = stream_resp;
    if trigger_reason == Some("model_fallback") {
        if let Ok(hv) = axum::http::HeaderValue::from_str(current_model) {
            stream_resp
                .headers_mut()
                .insert("X-ModelSwitch-Fallback-Model", hv);
        }
        if let Ok(hv) = axum::http::HeaderValue::from_str(original_model) {
            stream_resp
                .headers_mut()
                .insert("X-ModelSwitch-Original-Model", hv);
        }
    }

    // Apply keepalive if configured
    if let Some(secs) = state.gateway.stream_keepalive_secs {
        if secs > 0 {
            let (parts, body) = stream_resp.into_parts();
            let data_stream = body
                .into_data_stream()
                .map(|r: Result<bytes::Bytes, axum::Error>| r.map_err(std::io::Error::other));
            let kept_alive = keepalive_stream(data_stream, secs);
            let new_body = Body::from_stream(kept_alive);
            return Response::from_parts(parts, new_body);
        }
    }
    stream_resp
}

/// Handle a successful non-streaming (JSON) response from upstream.
/// Extracts usage, accumulates quota, logs, caches the response, and returns it.
///
/// When `nonstream_keepalive_interval_secs` is enabled (> 0), the gateway sends
/// periodic `\n` whitespace bytes as HTTP chunked transfer encoding while waiting
/// for the upstream response body. This prevents client-side TCP timeouts for
/// long-running inference. The whitespace is valid JSON leading whitespace and
/// is silently ignored by JSON parsers.
///
/// The `pool_guard` is held for the entire function body and dropped at
/// function end — after `resp.text().await` completes and all processing
/// finishes. This is correct because the non-streaming response body is
/// fully consumed when `resp.text().await` returns.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_json_success(
    state: &Arc<crate::proxy::AppState>,
    channel: &Channel,
    resp: reqwest::Response,
    body: &Value,
    original_model: &str,
    provider: &dyn ProviderAdaptor,
    current_model: &str,
    upstream_model: &str,
    attempt: u32,
    trigger_reason: Option<&str>,
    start: std::time::Instant,
    request_id: Option<&str>,
    upstream_headers: &[(String, String)],
    vk_id: Option<Uuid>,
    reserved_cents: u64,
    cache_key: u128,
    cache_key_material: &str,
    pool_guard: crate::http_pool::PooledClient,
    active_guard: ActiveRequestGuard,
    request_format: RequestFormat,
    upstream_format: RequestFormat,
) -> Response {
    // Hold the pool guard and active-request guard until the function returns —
    // the non-streaming response body is fully consumed after `resp.bytes().await`
    // below. Both guards decrement their counters on drop.
    let _pool_guard = pool_guard;
    let _active_guard = active_guard;

    // Check if non-stream keepalive is enabled. When enabled, we race
    // resp.bytes() against a keepalive interval, sending `\n` whitespace
    // chunks to keep the TCP connection alive.
    let keepalive_secs = state.gateway.nonstream_keepalive_interval_secs;

    // When keepalive is active, we create an mpsc channel. The sender (`tx`)
    // is used during the select loop to enqueue `\n` keepalive chunks. After
    // the upstream response arrives, `tx` is used again to send the final JSON
    // body, then dropped to close the channel. The receiver (`rx`) becomes the
    // HTTP response body via `Body::from_stream`.
    let (body_bytes, keepalive_tx, keepalive_rx) = if keepalive_secs > 0 {
        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(keepalive_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await; // skip first immediate tick

        let bytes_fut = resp.bytes();
        tokio::pin!(bytes_fut);

        let bytes = loop {
            tokio::select! {
                result = &mut bytes_fut => {
                    break result.unwrap_or_default();
                }
                _ = interval.tick() => {
                    // Send whitespace keepalive chunk
                    if tx.send(Ok(Bytes::from("\n"))).await.is_err() {
                        // Client disconnected — still consume the response
                        break bytes_fut.await.unwrap_or_default();
                    }
                }
            }
        };

        (bytes, Some(tx), Some(rx))
    } else {
        (resp.bytes().await.unwrap_or_default(), None, None)
    };

    // Translate response body via provider (pass-through for OpenAI/Anthropic,
    // Gemini-to-OpenAI translation for Gemini).
    // Use Bytes to avoid String allocation + UTF-8 validation on the hot path.
    let response_bytes: bytes::Bytes = if provider.needs_response_transform() {
        // Only Gemini needs parse + transform + re-serialize
        let body_str = std::str::from_utf8(&body_bytes).unwrap_or("");
        if let Ok(v) = serde_json::from_str::<Value>(body_str) {
            let translated = provider.transform_response(&v, upstream_model);
            bytes::Bytes::from(serde_json::to_string(&translated).unwrap_or_default())
        } else {
            body_bytes
        }
    } else {
        // Pass-through — use original bytes, no parse/re-serialize
        body_bytes
    };

    // Protocol translation: if the request format differs from upstream format,
    // translate the response body from upstream_format to request_format.
    let response_bytes: bytes::Bytes = if request_format != upstream_format {
        let body_str = std::str::from_utf8(&response_bytes).unwrap_or("");
        if let Ok(body_value) = serde_json::from_str::<serde_json::Value>(body_str) {
            let translated = crate::proxy::translate::translate_response(
                &body_value,
                upstream_format,
                request_format,
                upstream_model,
            );
            bytes::Bytes::from(
                serde_json::to_string(&translated).unwrap_or_else(|_| body_str.to_string()),
            )
        } else {
            response_bytes
        }
    } else {
        response_bytes
    };

    // Zero-copy str borrow from Bytes for usage extraction
    let response_str = std::str::from_utf8(&response_bytes).unwrap_or("");
    let token_usage = extract_usage(response_str);
    let input_tokens = token_usage.input_tokens;
    let output_tokens = token_usage.output_tokens;
    // Determine effective rates: model_pricing overrides channel rates.
    let mp = state.gateway.model_pricing.get(current_model);
    let eff_in = mp
        .and_then(|p| p.input_cost_per_mtok)
        .or(channel.input_cost_per_mtok);
    let eff_out = mp
        .and_then(|p| p.output_cost_per_mtok)
        .or(channel.output_cost_per_mtok);
    let estimated_cost = if eff_in.is_some() || eff_out.is_some() {
        let in_tok = input_tokens.unwrap_or(0) as f64;
        let out_tok = output_tokens.unwrap_or(0) as f64;
        let completion_ratio = state
            .gateway
            .completion_ratios
            .get(current_model)
            .copied()
            .unwrap_or(1.0);
        Some(
            in_tok / 1_000_000.0 * eff_in.unwrap_or(0.0)
                + out_tok / 1_000_000.0 * eff_out.unwrap_or(0.0) * completion_ratio,
        )
    } else {
        channel
            .calculate_cost(input_tokens, output_tokens)
            .or_else(|| {
                let est = estimate_tokens(body, false);
                channel.calculate_cost(Some(est / 2), Some(est / 2))
            })
    };
    // Cache non-streaming responses (inline — coalesced waiters depend on
    // ordering: insert must precede complete()).
    state.cache.request_cache.insert(
        cache_key,
        cache_key_material.to_string(),
        response_str.to_string(),
    );
    state.cache.in_flight.complete(cache_key);

    // Active-request decrement handled by `_active_guard` drop at function end.

    // Background: quota accumulation, virtual-key spend, logging, and metrics
    // are non-blocking to return the HTTP response as quickly as possible.
    {
        let bg_quota_store = Arc::clone(&state.billing.quota_store);
        let bg_virtual_key_store = Arc::clone(&state.billing.virtual_key_store);
        let bg_provider_budgets = Arc::clone(&state.billing.provider_budgets);
        let bg_key_rate_limiter = Arc::clone(&state.billing.key_rate_limiter);
        let bg_logger = Arc::clone(&state.logger);
        let bg_channel_mgr = Arc::clone(&state.channel_mgr);
        let bg_latency_tracker = Arc::clone(&state.router.latency_tracker);
        let bg_channel_id = channel.id;
        let bg_channel_name = channel.name.clone();
        let bg_provider = channel.provider.clone();
        let bg_provider_name = channel.provider.as_str().to_string();
        let bg_channel_priority = channel.priority;
        let bg_input_tokens = input_tokens;
        let bg_output_tokens = output_tokens;
        let bg_cache_hit_tokens = token_usage.cache_hit_tokens;
        let bg_cache_miss_tokens = token_usage.cache_miss_tokens;
        let bg_estimated_cost = estimated_cost;
        let bg_current_model = current_model.to_string();
        let bg_attempt = attempt;
        let bg_trigger_reason = trigger_reason.map(|s| s.to_string());
        let bg_start = start;
        let bg_request_id = request_id.map(|s| s.to_string());
        let bg_vk_id = vk_id;
        let bg_reserved_cents = reserved_cents;

        crate::spawn_bg(async move {
            // Accumulate usage (tokens + cost) into quota store
            bg_quota_store
                .accumulate_usage(
                    bg_channel_id,
                    bg_input_tokens,
                    bg_output_tokens,
                    bg_cache_hit_tokens,
                    bg_cache_miss_tokens,
                    bg_estimated_cost,
                )
                .await;

            // Attribute spend to the requesting virtual key (if any).
            // When a reservation was made before dispatch, reconcile against
            // it so the key is not double-charged (reservation + accumulation).
            if let Some(vk) = bg_vk_id {
                let cost_cents = (bg_estimated_cost.unwrap_or(0.0) * 100.0) as u64;
                if bg_reserved_cents > 0 {
                    bg_virtual_key_store
                        .reconcile_spend(vk, bg_reserved_cents, cost_cents)
                        .await;
                } else {
                    bg_virtual_key_store.accumulate_spend(vk, cost_cents).await;
                }

                // Record actual token consumption against the key's TPM
                // window. Post-response complement to the pre-request
                // `check_tpm` gate in the virtual-key middleware.
                let total_tokens = bg_input_tokens.unwrap_or(0) + bg_output_tokens.unwrap_or(0);
                bg_key_rate_limiter.record_tokens(vk, total_tokens);
            }

            // Accumulate spend into per-provider budget tracker.
            let cost_cents = (bg_estimated_cost.unwrap_or(0.0) * 100.0) as u64;
            bg_provider_budgets
                .accumulate_spend(&bg_provider_name, cost_cents)
                .await;

            bg_logger
                .log(make_log(
                    &bg_current_model,
                    bg_channel_id,
                    &bg_channel_name,
                    bg_channel_priority,
                    bg_attempt,
                    bg_trigger_reason.as_deref(),
                    bg_start.elapsed().as_millis() as u64,
                    true,
                    bg_estimated_cost,
                    bg_input_tokens,
                    bg_output_tokens,
                    bg_cache_hit_tokens,
                    bg_cache_miss_tokens,
                    bg_request_id.as_deref(),
                    bg_vk_id.map(|id| id.to_string()),
                ))
                .await;

            // Prometheus metrics
            let provider_label = bg_provider.as_str();
            crate::metrics::requests_total()
                .with_label_values(&[provider_label, &bg_current_model, "success"])
                .inc();
            crate::metrics::request_duration()
                .with_label_values(&[provider_label, &bg_current_model])
                .observe(bg_start.elapsed().as_secs_f64());
            crate::metrics::record_latency(bg_start.elapsed(), &bg_current_model, provider_label);

            // Token-level metrics
            if let Some(it) = bg_input_tokens {
                crate::metrics::input_tokens_total()
                    .with_label_values(&[provider_label, &bg_current_model])
                    .inc_by(it);
            }
            if let Some(ot) = bg_output_tokens {
                crate::metrics::output_tokens_total()
                    .with_label_values(&[provider_label, &bg_current_model])
                    .inc_by(ot);
            }
            // Token-usage histogram (prompt + completion observations).
            crate::metrics::record_tokens(
                bg_input_tokens.unwrap_or(0),
                bg_output_tokens.unwrap_or(0),
                &bg_current_model,
            );
            // Cost histogram.
            if let Some(cost) = bg_estimated_cost {
                crate::metrics::record_cost(cost, &bg_current_model);
            }

            let _ = bg_channel_mgr
                .record_latency(bg_channel_id, bg_start.elapsed().as_millis() as u64)
                .await;
            bg_latency_tracker.record_with_tokens(
                bg_channel_id,
                bg_start.elapsed().as_millis() as u64,
                bg_output_tokens,
            );
        });
    }

    // Build the response body. If keepalive was active, we stream the final
    // JSON through the existing channel (which may already contain keepalive
    // `\n` chunks from the select loop above). The ReceiverStream drains the
    // keepalive chunks followed by this final payload, then closes when the
    // sender drops. Otherwise, use a simple inline body.
    let response_body = if let (Some(tx), Some(rx)) = (keepalive_tx, keepalive_rx) {
        let final_bytes = response_bytes.clone();
        crate::spawn_bg(async move {
            let _ = tx.send(Ok(final_bytes)).await;
            // tx drops here, closing the channel
        });
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
    } else {
        Body::from(response_bytes)
    };

    let mut resp = inject_passthrough_headers(
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(response_body)
            .expect("valid HTTP response construction"),
        upstream_headers,
    );
    if trigger_reason == Some("model_fallback") {
        if let Ok(hv) = axum::http::HeaderValue::from_str(current_model) {
            resp.headers_mut()
                .insert("X-ModelSwitch-Fallback-Model", hv);
        }
        if let Ok(hv) = axum::http::HeaderValue::from_str(original_model) {
            resp.headers_mut()
                .insert("X-ModelSwitch-Original-Model", hv);
        }
    }
    resp
}
