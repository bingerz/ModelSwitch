//! OpenAI Responses API (`/v1/responses`) handler.
//!
//! Translates between the Responses API format and the internal chat-completions
//! pipeline so that dispatch, caching, retries, and quota tracking remain shared.

use crate::proxy::provider::OpenAIAdaptor;
use crate::proxy::state::AppState;
use crate::proxy::stream::json_response;
use crate::proxy::{dispatch, validate_chat_request, RequestFormat};
use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Json;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Maximum response body size when buffering the upstream chat-completions
/// response for translation. Mirrors the MCP loop limit.
const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Handle `/v1/responses` requests.
///
/// The Responses API is translated to an internal chat-completions dispatch
/// (forced non-streaming) and then translated back. When the client requested
/// `stream: true`, the final response is emitted as four Responses API SSE
/// events synthesized from the non-streaming payload.
pub async fn handle_responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let provider = OpenAIAdaptor;

    // Translate the Responses API request into a chat-completions body.
    let was_streaming = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let chat_body = match translate_request(&body) {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if let Err(resp) = validate_chat_request(&chat_body) {
        return resp;
    }

    // Force non-streaming internally so we can translate the full payload.
    let chat_body_non_stream = {
        let mut b = chat_body;
        if let Some(obj) = b.as_object_mut() {
            obj.insert("stream".to_string(), json!(false));
        }
        b
    };

    let upstream = dispatch(
        &state,
        &headers,
        &chat_body_non_stream,
        &provider,
        RequestFormat::OpenAIResponses,
    )
    .await;

    let (status, chat_response) = match extract_response_json(upstream).await {
        Ok(parts) => parts,
        Err(fallback) => return fallback,
    };

    // If the upstream returned an error status, pass it through as-is.
    if !status.is_success() {
        return json_response(status, chat_response.to_string());
    }

    let responses_payload = translate_response(&chat_response);

    if was_streaming {
        sse_responses_stream(&responses_payload)
    } else {
        json_response(StatusCode::OK, responses_payload.to_string())
    }
}

// ── Request translation ─────────────────────────────────────────────────────

/// Translate a Responses API request body into a chat-completions body.
///
/// Returns `Err(response)` with a 400 JSON error when the request is malformed.
fn translate_request(body: &Value) -> Result<Value, Response> {
    let mut chat = serde_json::Map::new();

    // model — pass through (required for downstream validation).
    if let Some(model) = body.get("model") {
        chat.insert("model".to_string(), model.clone());
    }

    // messages — built from `instructions` + `input`.
    let messages = build_messages(body)?;
    chat.insert("messages".to_string(), Value::Array(messages));

    // max_output_tokens → max_tokens
    if let Some(max_output) = body.get("max_output_tokens") {
        chat.insert("max_tokens".to_string(), max_output.clone());
    }

    // Pass-through fields.
    for key in &["temperature", "top_p", "tools", "tool_choice", "stream"] {
        if let Some(v) = body.get(*key) {
            chat.insert((*key).to_string(), v.clone());
        }
    }

    Ok(Value::Object(chat))
}

/// Build the chat-completions `messages` array from the Responses API
/// `instructions` and `input` fields.
///
/// - `instructions` (if present) becomes the first system message.
/// - `input` as a string becomes a single user message.
/// - `input` as an array is treated as a conversation and copied as-is after
///   any system message from `instructions`.
fn build_messages(body: &Value) -> Result<Vec<Value>, Response> {
    let mut messages = Vec::new();

    if let Some(instructions) = body.get("instructions") {
        let text = instructions
            .as_str()
            .ok_or_else(|| invalid_request("instructions must be a string"))?;
        messages.push(json!({"role": "system", "content": text}));
    }

    match body.get("input") {
        Some(Value::String(s)) => {
            messages.push(json!({"role": "user", "content": s}));
        }
        Some(Value::Array(arr)) => {
            // The Responses API `input` array items already carry role/content.
            messages.extend(arr.iter().cloned());
        }
        Some(_) => {
            return Err(invalid_request("input must be a string or an array"));
        }
        None => {
            // `input` is not strictly required when the caller passes `instructions`,
            // but downstream chat validation needs at least one message.
        }
    }

    if messages.is_empty() {
        return Err(invalid_request("Missing required field: input"));
    }

    Ok(messages)
}

// ── Response translation ────────────────────────────────────────────────────

/// Translate a chat-completions non-streaming response into a Responses API
/// payload.
fn translate_response(chat: &Value) -> Value {
    let id = format!("resp_{}", Uuid::new_v4().simple());
    let created_at = chat.get("created").cloned().unwrap_or(json!(0));
    let model = chat
        .get("model")
        .cloned()
        .unwrap_or_else(|| json!("unknown"));

    let (status, output) = build_output(chat);

    let usage = translate_usage(chat);

    json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "model": model,
        "status": status,
        "output": output,
        "usage": usage,
    })
}

/// Build the `output` array and top-level `status` from the first choice.
fn build_output(chat: &Value) -> (&'static str, Vec<Value>) {
    let Some(choice) = chat.get("choices").and_then(|c| c.get(0)) else {
        return ("completed", Vec::new());
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .unwrap_or("stop");

    let status = match finish_reason {
        "length" => "incomplete",
        _ => "completed",
    };

    let msg_id = format!("msg_{}", Uuid::new_v4().simple());
    let message = choice.get("message").cloned().unwrap_or(json!({}));
    let role = message
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("assistant");

    let mut content_items = Vec::new();

    // Text content.
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content_items.push(json!({
                "type": "output_text",
                "text": text,
            }));
        }
    }

    // Tool calls → function_call items.
    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let function = tc.get("function").cloned().unwrap_or(json!({}));
            let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("");
            content_items.push(json!({
                "type": "function_call",
                "id": id,
                "call_id": id,
                "name": name,
                "arguments": arguments,
            }));
        }
    }

    let output_item = json!({
        "type": "message",
        "id": msg_id,
        "role": role,
        "status": status,
        "content": content_items,
    });

    (status, vec![output_item])
}

/// Translate chat-completions usage to Responses API usage field names.
fn translate_usage(chat: &Value) -> Value {
    let usage = chat.get("usage").cloned().unwrap_or(json!({}));
    json!({
        "input_tokens": usage.get("prompt_tokens").cloned().unwrap_or(json!(0)),
        "output_tokens": usage.get("completion_tokens").cloned().unwrap_or(json!(0)),
        "total_tokens": usage.get("total_tokens").cloned().unwrap_or(json!(0)),
    })
}

// ── Streaming synthesis ─────────────────────────────────────────────────────

/// Build a synthetic SSE stream from a completed Responses payload.
///
/// Emits four events: `response.created`, `response.output_text.delta`,
/// `response.output_text.done`, and `response.completed`.
fn sse_responses_stream(payload: &Value) -> Response {
    let text = payload
        .get("output")
        .and_then(|o| o.get(0))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.get(0))
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    // `response.created` — the in_progress variant.
    let created = {
        let mut v = payload.clone();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("status".to_string(), json!("in_progress"));
        }
        v
    };

    // `response.completed` — the payload as-is.
    let completed = payload.clone();

    let body = format!(
        "event: response.created\ndata: {created}\n\n\
         event: response.output_text.delta\ndata: {delta}\n\n\
         event: response.output_text.done\ndata: {done}\n\n\
         event: response.completed\ndata: {completed}\n\n",
        created = created.to_string(),
        delta = json!({"delta": text}).to_string(),
        done = json!({"text": text}).to_string(),
        completed = completed.to_string(),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(Body::from(body))
        .expect("valid HTTP response construction")
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Build a 400 invalid-request JSON response.
fn invalid_request(message: &str) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
            }
        })
        .to_string(),
    )
}

/// Buffer an axum `Response` body and parse it as JSON.
///
/// Mirrors the helper in `proxy/openai.rs` but scoped to this module to keep
/// the Responses handler self-contained.
async fn extract_response_json(response: Response) -> Result<(StatusCode, Value), Response> {
    let status = response.status();
    let bytes = match axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "responses: failed to buffer upstream response");
            return Err(json_response(
                StatusCode::BAD_GATEWAY,
                json!({
                    "error": {
                        "message": "responses: failed to buffer upstream response",
                        "type": "upstream_error",
                    }
                })
                .to_string(),
            ));
        }
    };

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            let raw = String::from_utf8_lossy(&bytes).to_string();
            return Err(json_response(status, raw));
        }
    };

    Ok((status, value))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Request translation --

    #[test]
    fn translate_request_string_input() {
        let body = json!({
            "model": "gpt-4o",
            "input": "Tell me a joke",
            "temperature": 0.7,
        });
        let chat = translate_request(&body).expect("valid request");
        assert_eq!(chat["model"], "gpt-4o");
        assert_eq!(
            chat["messages"],
            json!([{"role": "user", "content": "Tell me a joke"}])
        );
        assert_eq!(chat["temperature"], 0.7);
        assert!(chat.get("instructions").is_none());
        assert!(chat.get("input").is_none());
    }

    #[test]
    fn translate_request_array_input_passthrough() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there"},
                {"role": "user", "content": "How are you?"}
            ],
        });
        let chat = translate_request(&body).expect("valid request");
        assert_eq!(chat["messages"].as_array().unwrap().len(), 3);
        assert_eq!(chat["messages"][0]["role"], "user");
        assert_eq!(chat["messages"][1]["role"], "assistant");
        assert_eq!(chat["messages"][2]["content"], "How are you?");
    }

    #[test]
    fn translate_request_instructions_becomes_system_message() {
        let body = json!({
            "model": "gpt-4o",
            "instructions": "You are a helpful assistant.",
            "input": "Hi",
        });
        let chat = translate_request(&body).expect("valid request");
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are a helpful assistant.");
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn translate_request_instructions_plus_array_input_prepends_system() {
        let body = json!({
            "model": "gpt-4o",
            "instructions": "Be concise.",
            "input": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"}
            ],
        });
        let chat = translate_request(&body).expect("valid request");
        let messages = chat["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
    }

    #[test]
    fn translate_request_max_output_tokens_to_max_tokens() {
        let body = json!({
            "model": "gpt-4o",
            "input": "Hi",
            "max_output_tokens": 1024,
        });
        let chat = translate_request(&body).expect("valid request");
        assert_eq!(chat["max_tokens"], 1024);
        assert!(chat.get("max_output_tokens").is_none());
    }

    #[test]
    fn translate_request_passes_through_tools_and_top_p() {
        let body = json!({
            "model": "gpt-4o",
            "input": "Hi",
            "top_p": 0.9,
            "tools": [{"type": "function", "function": {"name": "foo"}}],
        });
        let chat = translate_request(&body).expect("valid request");
        assert_eq!(chat["top_p"], 0.9);
        assert!(chat["tools"].is_array());
        assert_eq!(chat["tools"][0]["function"]["name"], "foo");
    }

    #[test]
    fn translate_request_rejects_invalid_input_type() {
        let body = json!({
            "model": "gpt-4o",
            "input": 42,
        });
        let resp = translate_request(&body);
        assert!(resp.is_err());
    }

    #[test]
    fn translate_request_rejects_missing_input_without_instructions() {
        let body = json!({"model": "gpt-4o"});
        let resp = translate_request(&body);
        assert!(resp.is_err());
    }

    // -- Response translation --

    #[test]
    fn translate_response_basic_text() {
        let chat = json!({
            "id": "chatcmpl-abc",
            "object": "chat.completion",
            "created": 1234567890_i64,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Why did the..."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
        });
        let resp = translate_response(&chat);
        assert_eq!(resp["object"], "response");
        assert!(resp["id"].as_str().unwrap().starts_with("resp_"));
        assert_eq!(resp["created_at"], 1234567890_i64);
        assert_eq!(resp["model"], "gpt-4o");
        assert_eq!(resp["status"], "completed");

        let output = resp["output"].as_array().unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["role"], "assistant");
        assert_eq!(output[0]["status"], "completed");
        let content = output[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "Why did the...");
    }

    #[test]
    fn translate_response_length_finish_maps_to_incomplete() {
        let chat = json!({
            "model": "gpt-4o",
            "created": 100,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Truncated..."},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
        });
        let resp = translate_response(&chat);
        assert_eq!(resp["status"], "incomplete");
        assert_eq!(resp["output"][0]["status"], "incomplete");
    }

    #[test]
    fn translate_response_tool_calls() {
        let chat = json!({
            "model": "gpt-4o",
            "created": 100,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"city\":\"SF\"}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
        });
        let resp = translate_response(&chat);
        let content = resp["output"][0]["content"].as_array().unwrap();
        // null content should not produce an output_text item.
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "function_call");
        assert_eq!(content[0]["id"], "call_abc");
        assert_eq!(content[0]["call_id"], "call_abc");
        assert_eq!(content[0]["name"], "get_weather");
        assert_eq!(content[0]["arguments"], "{\"city\":\"SF\"}");
        assert_eq!(resp["status"], "completed");
    }

    #[test]
    fn translate_response_usage_mapping() {
        let chat = json!({
            "model": "gpt-4o",
            "created": 1,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 42, "completion_tokens": 8, "total_tokens": 50}
        });
        let resp = translate_response(&chat);
        assert_eq!(resp["usage"]["input_tokens"], 42);
        assert_eq!(resp["usage"]["output_tokens"], 8);
        assert_eq!(resp["usage"]["total_tokens"], 50);
    }

    #[test]
    fn translate_response_id_is_unique() {
        let chat = json!({
            "model": "gpt-4o",
            "created": 1,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        });
        let r1 = translate_response(&chat);
        let r2 = translate_response(&chat);
        assert_ne!(r1["id"], r2["id"]);
    }
}
