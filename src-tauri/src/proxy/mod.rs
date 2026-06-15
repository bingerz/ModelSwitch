pub mod anthropic;
pub mod cache;
pub mod gemini;
pub mod mcp_tools;
pub mod openai;
pub mod payload_rules;
pub mod rate_limiter;
pub mod stream;
pub mod translate;

mod attempt;
mod dispatch;
mod request_meta;
mod response;
mod usage;

#[cfg(test)]
mod dispatch_tests;

use crate::channel::Channel;
use crate::log::DispatchLog;
use crate::proxy::stream::json_response;
use axum::response::Response;
use chrono::Utc;
use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

/// Build upstream URL for a channel given a path suffix.
pub fn upstream_url(channel: &Channel, path: &str) -> String {
    let base = channel.base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{}/{}", base, path)
}

/// Build a DispatchLog entry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_log(
    model: &str,
    channel_id: Uuid,
    channel_name: &str,
    channel_priority: u8,
    retry_count: u32,
    reason: Option<&str>,
    latency_ms: u64,
    success: bool,
    estimated_cost: Option<f64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_hit_tokens: Option<u64>,
    cache_miss_tokens: Option<u64>,
    request_id: Option<&str>,
) -> DispatchLog {
    DispatchLog {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        request_model: model.to_string(),
        channel_id,
        channel_name: channel_name.to_string(),
        channel_priority,
        retry_count: retry_count as u8,
        trigger_reason: reason.map(|s| s.to_string()),
        latency_ms,
        success,
        estimated_cost,
        input_tokens,
        output_tokens,
        cache_hit_tokens,
        cache_miss_tokens,
        request_id: request_id.map(|s| s.to_string()),
    }
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

/// Estimate token count from request body for cost calculation.
/// Uses a rough heuristic: ~4 chars per token for English, reads max_tokens if present.
fn estimate_tokens(body: &Value, _is_stream: bool) -> u64 {
    let output_tokens = body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);

    let input_chars = body
        .get("messages")
        .and_then(|m| {
            m.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|msg| {
                        msg.get("content").map(|c| {
                            if let Some(s) = c.as_str() {
                                s.len()
                            } else {
                                c.as_array()
                                    .map(|blocks| {
                                        blocks
                                            .iter()
                                            .filter_map(|b| {
                                                b.get("text")
                                                    .and_then(|t| t.as_str())
                                                    .map(|t| t.len())
                                            })
                                            .sum::<usize>()
                                    })
                                    .unwrap_or(0)
                            }
                        })
                    })
                    .sum::<usize>()
            })
        })
        .unwrap_or(0);
    let input_tokens = (input_chars as u64) / 4;

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

/// Upstream response headers to pass through to the client.
const PASSTHROUGH_RESPONSE_HEADERS: &[&str] = &[
    "x-ratelimit-remaining",
    "x-ratelimit-limit",
    "x-ratelimit-reset",
    "x-ratelimit-limit-requests",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-reset-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-tokens",
    "x-ratelimit-reset-tokens",
    "anthropic-ratelimit-requests-limit",
    "anthropic-ratelimit-requests-remaining",
    "anthropic-ratelimit-requests-reset",
    "anthropic-ratelimit-tokens-limit",
    "anthropic-ratelimit-tokens-remaining",
    "anthropic-ratelimit-tokens-reset",
    "x-request-id",
];

/// Provider-specific authentication style.
#[derive(Clone)]
pub(crate) enum AuthStyle {
    OpenAI,
    Anthropic,
    Cookie,
    GeminiUrl,
}

/// Configuration for a proxy dispatch.
pub(crate) struct ProxyConfig {
    pub default_model: &'static str,
    pub upstream_path: &'static str,
    pub auth_style: AuthStyle,
}

/// Categorized reason for a channel dispatch failure.
#[derive(Debug, Clone)]
pub(super) enum FailureReason {
    RateLimited,
    ServerError,
    ConnectionError,
    NoCredential,
    Timeout,
    #[allow(dead_code)]
    ModelFallback,
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
            Self::ModelFallback => "model_fallback".into(),
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
        // 0 input chars, default max_tokens = 4096
        assert_eq!(tokens, 4096);
    }

    #[test]
    fn estimate_tokens_with_content() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world test"}]
        });
        let tokens = estimate_tokens(&body, false);
        // "hello world test" = 16 chars / 4 = 4 input tokens + 4096 default = 4100
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
        // "hi" = 2 chars / 4 = 0 input tokens + 100 max_tokens = 100
        assert_eq!(tokens, 100);
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
        // "hello world" (11) + "foo bar baz" (11) = 22 chars / 4 = 5 input tokens + 4096 = 4101
        assert!(tokens > 4096);
    }

    #[test]
    fn estimate_tokens_max_tokens_not_present() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}]
        });
        let tokens = estimate_tokens(&body, false);
        // "test" = 4 chars / 4 = 1 input token + 4096 default max_tokens = 4097
        assert_eq!(tokens, 4097);
    }
}
