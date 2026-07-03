pub mod anthropic;
pub mod gemini;

pub use anthropic::*;
pub use gemini::*;

use serde_json::Value;

use crate::proxy::RequestFormat;

// ── Translation registry / dispatch ─────────────────────────────────────────

/// Translate a request body from one wire format to another.
///
/// When `from == to` or no translation is available, returns a clone of the
/// input. Logs the translation event at info level.
pub(crate) fn translate_request(body: &Value, from: RequestFormat, to: RequestFormat) -> Value {
    if from == to {
        return body.clone();
    }
    let result = match (from, to) {
        (RequestFormat::OpenAIChat, RequestFormat::AnthropicMessages) => {
            openai_to_anthropic_request(body)
        }
        (RequestFormat::AnthropicMessages, RequestFormat::OpenAIChat) => {
            anthropic_to_openai_request(body)
        }
        _ => body.clone(),
    };
    tracing::info!(from = ?from, to = ?to, "Translated request format");
    result
}

/// Translate a non-streaming response body from one wire format to another.
///
/// When `from == to` or no translation is available, returns a clone of the
/// input. Logs the translation event at info level.
pub(crate) fn translate_response(
    body: &Value,
    from: RequestFormat,
    to: RequestFormat,
    model: &str,
) -> Value {
    if from == to {
        return body.clone();
    }
    let result = match (from, to) {
        (RequestFormat::AnthropicMessages, RequestFormat::OpenAIChat) => {
            anthropic_to_openai_response(body, model)
        }
        (RequestFormat::OpenAIChat, RequestFormat::AnthropicMessages) => {
            openai_to_anthropic_response(body)
        }
        _ => body.clone(),
    };
    tracing::info!(from = ?from, to = ?to, "Translated response format");
    result
}

/// Translate a single SSE stream chunk from one wire format to another.
///
/// When `from == to` or no translation is available, returns `None` (the
/// chunk passes through unchanged).
pub(crate) fn translate_stream_chunk(
    chunk: &Value,
    from: RequestFormat,
    to: RequestFormat,
    model: &str,
) -> Option<String> {
    if from == to {
        return None;
    }
    match (from, to) {
        (RequestFormat::AnthropicMessages, RequestFormat::OpenAIChat) => {
            anthropic_to_openai_stream_chunk(chunk, model)
        }
        (RequestFormat::OpenAIChat, RequestFormat::AnthropicMessages) => {
            openai_to_anthropic_stream_chunk(chunk)
        }
        _ => None,
    }
}
