pub mod anthropic;
pub mod cache;
pub mod embeddings;
pub mod gemini;
pub mod images;
pub mod mcp_tools;
pub mod openai;
pub mod payload_rules;
pub mod rate_limiter;
pub mod redis_rate_limit;
pub mod responses;
pub mod state;
pub mod stream;
pub mod thinking;
pub mod translate;

mod attempt;
mod dispatch;
pub(crate) mod provider;
mod request_meta;
mod response;
mod token_counter;
mod usage;

#[cfg(test)]
mod dispatch_tests;

pub use state::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};

use crate::log::DispatchLog;
use crate::proxy::stream::json_response;
use axum::response::Response;
use chrono::Utc;
use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

/// The wire-format of a proxy request/response.
///
/// Used by the translation registry to detect when the incoming request
/// format differs from the upstream provider format, triggering automatic
/// protocol translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestFormat {
    OpenAIChat,
    AnthropicMessages,
    Gemini,
    OpenAIResponses,
}

/// Input fields for constructing a DispatchLog entry.
///
/// Bundles the 15 positional parameters of the former `make_log` signature into
/// a single struct, reducing the risk of argument-order mistakes at call sites.
pub(crate) struct DispatchLogInput<'a> {
    pub model: &'a str,
    pub channel_id: Uuid,
    pub channel_name: &'a str,
    pub channel_priority: u8,
    pub retry_count: u32,
    pub reason: Option<&'a str>,
    pub latency_ms: u64,
    pub success: bool,
    pub estimated_cost: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
    pub request_id: Option<&'a str>,
    pub virtual_key_id: Option<String>,
}

/// Build a DispatchLog entry.
pub(crate) fn make_log(input: &DispatchLogInput<'_>) -> DispatchLog {
    DispatchLog {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        request_model: input.model.to_string(),
        channel_id: input.channel_id,
        channel_name: input.channel_name.to_string(),
        channel_priority: input.channel_priority,
        retry_count: input.retry_count as u8,
        trigger_reason: input.reason.map(|s| s.to_string()),
        latency_ms: input.latency_ms,
        success: input.success,
        estimated_cost: input.estimated_cost,
        input_tokens: input.input_tokens,
        output_tokens: input.output_tokens,
        cache_hit_tokens: input.cache_hit_tokens,
        cache_miss_tokens: input.cache_miss_tokens,
        request_id: input.request_id.map(|s| s.to_string()),
        virtual_key_id: input.virtual_key_id.clone(),
    }
}

/// Build a JSON error response with the standard OpenAI-compatible error envelope.
#[allow(clippy::result_large_err)]
pub(super) fn error_response(
    status: reqwest::StatusCode,
    message: &str,
    error_type: &str,
    code: &str,
) -> Response {
    let error_body = serde_json::json!({
        "error": {
            "message": message,
            "type": error_type,
            "code": code,
        }
    });
    json_response(status, error_body.to_string())
}

/// Validate required fields in a chat completion request.
/// Returns OpenAI-compatible 400 error if validation fails.
#[allow(clippy::result_large_err)]
pub(crate) fn validate_chat_request(body: &Value) -> Result<(), Response> {
    if body
        .get("model")
        .and_then(|m| m.as_str())
        .is_none_or(|s| s.is_empty())
    {
        return Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({
            "error": { "message": "Missing required field: model", "type": "invalid_request_error", "code": "missing_model" }
        }).to_string()));
    }
    if body
        .get("messages")
        .and_then(|m| m.as_array())
        .is_none_or(|a| a.is_empty())
    {
        return Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({
            "error": { "message": "Missing required field: messages", "type": "invalid_request_error", "code": "missing_messages" }
        }).to_string()));
    }
    Ok(())
}

/// Estimate token count from request body for rate limiting and cost calculation.
/// Uses accurate BPE tokenization for OpenAI models, falls back to character heuristic.
fn estimate_tokens(body: &Value, _is_stream: bool) -> u64 {
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
    let input_tokens = token_counter::count_input_tokens(model, body);
    let output_tokens = body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);
    input_tokens + output_tokens
}

/// Headers to skip when forwarding from client to upstream.
const SKIP_HEADERS: &[&str] = &[
    // Hop-by-hop headers
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "content-length",
    // Auth headers managed by dispatch
    "authorization",
    "x-api-key",
    "anthropic-version",
    // Content headers managed by dispatch
    "content-type",
];

/// Categorized reason for a channel dispatch failure.
#[derive(Debug, Clone)]
pub(super) enum FailureReason {
    RateLimited,
    ServerError,
    ConnectionError,
    NoCredential,
    Timeout,
    ContextOverflow,
    AllExhausted,
    ClientError(u16),
}

impl FailureReason {
    pub fn log_str(&self) -> String {
        match self {
            Self::RateLimited => "429".into(),
            Self::ServerError => "5xx".into(),
            Self::ConnectionError => "connection_error".into(),
            Self::Timeout => "ttft_timeout".into(),
            Self::NoCredential => "no_credential".into(),
            Self::ContextOverflow => "context_overflow".into(),
            Self::AllExhausted => "all_exhausted".into(),
            Self::ClientError(code) => code.to_string(),
        }
    }
}

// Re-export the dispatch entry point for provider handlers.
pub(crate) use dispatch::dispatch;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_tokens_empty_messages() {
        let body = json!({
            "model": "gpt-4",
            "messages": []
        });
        let tokens = estimate_tokens(&body, false);
        // 0 messages → 0 input tokens, default max_tokens = 4096
        assert_eq!(tokens, 4096);
    }

    #[test]
    fn estimate_tokens_with_content() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world test"}]
        });
        let tokens = estimate_tokens(&body, false);
        // 3 content words + 1 role word = 4 tokens + 3 msg overhead = 7 input + 4096 default
        assert!(tokens > 0);
    }

    #[test]
    fn estimate_tokens_respects_max_tokens() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100
        });
        let tokens = estimate_tokens(&body, false);
        // "hi" = 1 word + "user" role = 1 word = 2 tokens + 3 msg overhead = 5 input + 100 = 105
        assert_eq!(tokens, 105);
    }

    #[test]
    fn estimate_tokens_with_content_blocks() {
        let body = json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "hello world"},
                        {"type": "text", "text": "foo bar baz"}
                    ]
                }
            ]
        });
        let tokens = estimate_tokens(&body, false);
        // 5 content words + 1 role word = 6 tokens + 3 msg overhead = 9 input + 4096
        assert!(tokens > 4096);
    }

    #[test]
    fn estimate_tokens_max_tokens_not_present() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}]
        });
        let tokens = estimate_tokens(&body, false);
        // "test" = 1 word + "user" role = 1 word = 2 tokens + 3 msg overhead = 5 input + 4096 = 4101
        assert_eq!(tokens, 4101);
    }
}
