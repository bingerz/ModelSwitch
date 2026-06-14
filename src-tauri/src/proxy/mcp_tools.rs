//! MCP tool injection and response interception.
//!
//! This module wires MCP tools into the OpenAI chat-completions request
//! pipeline. When an LLM responds with `tool_calls` whose function names
//! use the `mcp__{server_id}__{tool}` namespace, the handler executes
//! them locally via [`crate::mcp::McpManager`] and re-submits the
//! conversation with the tool results appended, creating a transparent
//! agent loop.
//!
//! The loop runs at the handler level (see
//! [`crate::proxy::openai::handle_chat_completions`]); the lower-level
//! [`crate::proxy::dispatch`] function is unaware of MCP tools.

use crate::mcp::aggregator::{aggregate_all_tools, AggregatedTool};
use crate::mcp::translator;
use crate::mcp::McpManager;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Maximum bytes we will buffer when extracting a JSON response body.
const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024;

/// A detected MCP tool call extracted from an LLM response.
///
/// Only calls whose function name uses the `mcp__` namespace are
/// surfaced here. Non-MCP tool calls are left in the response untouched
/// and become the caller's responsibility.
#[derive(Debug, Clone)]
pub struct McpToolCall {
    /// OpenAI tool_call_id, echoed back in the tool result message.
    pub tool_call_id: String,
    /// MCP server that owns the tool.
    pub server_id: String,
    /// Original tool name on the MCP server (without namespace prefix).
    pub tool_name: String,
    /// Parsed arguments object, or `None` if the LLM emitted empty args.
    pub arguments: Option<Map<String, Value>>,
}

/// Inject MCP tool definitions into the request body's `tools` array.
///
/// Returns `(modified_body, injected_tools)`. If no MCP servers are
/// running (or they expose no tools), the body is returned unchanged
/// with an empty list, signalling the caller to skip the tool loop.
///
/// This function does NOT filter by the per-server `expose_tools` flag.
/// Filtering happens server-side: tools from servers that refuse to
/// list tools will simply not appear. If a stricter boundary is needed,
/// the caller can filter `injected_tools` afterwards.
pub async fn inject_mcp_tools(
    body: &Value,
    mcp_manager: &Arc<McpManager>,
) -> (Value, Vec<AggregatedTool>) {
    let mcp_tools = aggregate_all_tools(mcp_manager).await;
    if mcp_tools.is_empty() {
        return (body.clone(), Vec::new());
    }

    let mut new_body = body.clone();
    let mut tools = new_body
        .get("tools")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    for tool in &mcp_tools {
        tools.push(translator::to_openai_function(tool));
    }

    new_body["tools"] = json!(tools);
    (new_body, mcp_tools)
}

/// Scan a JSON response body for tool_calls targeting MCP tools.
///
/// Walks `choices[].message.tool_calls[]` and uses
/// [`translator::parse_mcp_tool_call`] to decide whether each call is
/// MCP-namespaced. Returns only the MCP calls — non-MCP calls are
/// ignored so the client can still handle them.
///
/// Tolerates malformed shapes (missing fields, non-string arguments) by
/// skipping individual entries rather than failing the whole call.
pub fn detect_mcp_tool_calls(response_body: &Value) -> Vec<McpToolCall> {
    let mut found: Vec<McpToolCall> = Vec::new();

    let choices = match response_body.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return found,
    };

    for choice in choices {
        let message = match choice.get("message") {
            Some(m) => m,
            None => continue,
        };
        let tool_calls = match message.get("tool_calls").and_then(|t| t.as_array()) {
            Some(t) => t,
            None => continue,
        };

        for call in tool_calls {
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }

            let function = match call.get("function") {
                Some(f) => f,
                None => continue,
            };

            let name = match function.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let arguments_str = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("");

            let parsed = match translator::parse_mcp_tool_call(name, arguments_str) {
                Some(p) => p,
                None => continue, // non-MCP tool, ignore
            };

            let (server_id, tool_name, arguments) = parsed;
            found.push(McpToolCall {
                tool_call_id: id,
                server_id,
                tool_name,
                arguments,
            });
        }
    }

    found
}

/// Execute MCP tool calls and return OpenAI-format tool result messages.
///
/// Each result is shaped as:
/// ```json
/// { "role": "tool", "tool_call_id": "<id>", "content": "<flattened>" }
/// ```
///
/// Tool-call failures are surfaced to the LLM as a `content` string
/// prefixed with `"Error: "` rather than dropping the call. This lets
/// the model react (e.g. retry with different arguments) instead of
/// hallucinating a result.
pub async fn execute_mcp_tool_calls(
    calls: &[McpToolCall],
    mcp_manager: &Arc<McpManager>,
) -> Vec<Value> {
    let mut results: Vec<Value> = Vec::with_capacity(calls.len());

    for call in calls {
        let outcome = mcp_manager
            .call_tool(&call.server_id, &call.tool_name, call.arguments.clone())
            .await;

        let content = match outcome {
            Ok(result) => translator::flatten_tool_result(&result),
            Err(e) => {
                tracing::warn!(
                    server_id = %call.server_id,
                    tool = %call.tool_name,
                    tool_call_id = %call.tool_call_id,
                    error = %e,
                    "MCP tool call failed"
                );
                format!("Error: MCP tool call failed: {e}")
            }
        };

        results.push(json!({
            "role": "tool",
            "tool_call_id": call.tool_call_id,
            "content": content,
        }));
    }

    results
}

/// Build a follow-up request body by appending the assistant message
/// (with its tool_calls) and the tool result messages.
///
/// Also forces `"stream": false` on the returned body: the MCP tool
/// loop processes responses as JSON, and the handler is responsible for
/// surfacing the final non-streaming response to clients that originally
/// requested streaming (see `handle_chat_completions` documentation).
pub fn build_followup_request(
    original_body: &Value,
    response_body: &Value,
    tool_results: &[Value],
) -> Value {
    let mut new_body = original_body.clone();

    let mut messages = new_body
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    // Append the assistant message verbatim (contains tool_calls)
    let mut assistant_appended = false;
    if let Some(choices) = response_body.get("choices").and_then(|c| c.as_array()) {
        if let Some(message) = choices.first().and_then(|c| c.get("message")).cloned() {
            messages.push(message);
            assistant_appended = true;
        }
    }

    if !assistant_appended {
        tracing::warn!(
            "build_followup_request: response had no choices[0].message; appending empty assistant"
        );
        messages.push(json!({"role": "assistant", "content": ""}));
    }

    // Append tool result messages
    for result in tool_results {
        messages.push(result.clone());
    }

    new_body["messages"] = json!(messages);

    // The tool loop never streams internally.
    new_body["stream"] = json!(false);

    new_body
}

/// Maximum response body size used when buffering an upstream response
/// for MCP inspection. Re-exported here for handler convenience.
pub fn max_response_body_bytes() -> usize {
    MAX_RESPONSE_BODY_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with_tool_calls(calls: &[Value]) -> Value {
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": calls,
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        })
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> Value {
        json!({
            "id": id,
            "type": "function",
            "function": {
                "name": name,
                "arguments": arguments,
            }
        })
    }

    #[test]
    fn detect_single_mcp_tool_call() {
        let body =
            response_with_tool_calls(&[tool_call("call_1", "mcp__fs__read", r#"{"path":"/tmp"}"#)]);

        let found = detect_mcp_tool_calls(&body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tool_call_id, "call_1");
        assert_eq!(found[0].server_id, "fs");
        assert_eq!(found[0].tool_name, "read");
        let args = found[0].arguments.as_ref().expect("arguments parsed");
        assert_eq!(args.get("path").and_then(|v| v.as_str()), Some("/tmp"));
    }

    #[test]
    fn detect_ignores_non_mcp_tool_calls() {
        let body =
            response_with_tool_calls(&[tool_call("call_1", "search_web", r#"{"query":"rust"}"#)]);

        let found = detect_mcp_tool_calls(&body);
        assert!(found.is_empty(), "non-MCP tool calls must be ignored");
    }

    #[test]
    fn detect_returns_only_mcp_calls_when_mixed() {
        let body = response_with_tool_calls(&[
            tool_call("call_a", "mcp__fs__read", r#"{"path":"/a"}"#),
            tool_call("call_b", "native_tool", "{}"),
            tool_call("call_c", "mcp__git__status", "{}"),
        ]);

        let found = detect_mcp_tool_calls(&body);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].tool_call_id, "call_a");
        assert_eq!(found[1].tool_call_id, "call_c");
        assert_eq!(found[1].server_id, "git");
    }

    #[test]
    fn detect_handles_missing_choices_or_tool_calls() {
        // No choices
        assert!(detect_mcp_tool_calls(&json!({"id":"x"})).is_empty());
        // Empty choices
        assert!(detect_mcp_tool_calls(&json!({"choices":[]})).is_empty());
        // Message without tool_calls
        assert!(detect_mcp_tool_calls(&json!({
            "choices":[{"message":{"role":"assistant","content":"hi"}}]
        }))
        .is_empty());
    }

    #[test]
    fn detect_skips_entries_with_missing_id_or_name() {
        let body = response_with_tool_calls(&[
            // Missing id
            json!({
                "type": "function",
                "function": {"name": "mcp__fs__read", "arguments": "{}"}
            }),
            // Missing name
            json!({
                "id": "call_x",
                "function": {"arguments": "{}"}
            }),
        ]);
        let found = detect_mcp_tool_calls(&body);
        assert!(found.is_empty());
    }

    #[test]
    fn detect_handles_null_arguments() {
        let body = response_with_tool_calls(&[tool_call("call_1", "mcp__fs__noop", "null")]);
        let found = detect_mcp_tool_calls(&body);
        assert_eq!(found.len(), 1);
        assert!(found[0].arguments.is_none());
    }

    #[test]
    fn build_followup_request_appends_assistant_and_tool_results() {
        let original = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        });

        let response =
            response_with_tool_calls(&[tool_call("call_1", "mcp__fs__read", r#"{"path":"/tmp"}"#)]);

        let tool_results = vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "file contents"
        })];

        let next = build_followup_request(&original, &response, &tool_results);

        let messages = next
            .get("messages")
            .and_then(|m| m.as_array())
            .expect("messages present");
        // 1 original + 1 assistant + 1 tool result
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert!(messages[1].get("tool_calls").is_some());
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn build_followup_request_forces_stream_false() {
        let original = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
        });

        let response =
            response_with_tool_calls(&[tool_call("call_1", "mcp__fs__read", r#"{"path":"/tmp"}"#)]);

        let next = build_followup_request(&original, &response, &[]);
        assert_eq!(
            next.get("stream").and_then(|s| s.as_bool()),
            Some(false),
            "follow-up body must set stream=false"
        );
    }

    #[test]
    fn build_followup_request_preserves_model_and_other_fields() {
        let original = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.7,
            "max_tokens": 512,
        });

        let response =
            response_with_tool_calls(&[tool_call("call_1", "mcp__fs__read", r#"{"path":"/x"}"#)]);

        let next = build_followup_request(&original, &response, &[]);
        assert_eq!(next["model"], "gpt-4o");
        assert_eq!(next["temperature"], 0.7);
        assert_eq!(next["max_tokens"], 512);
    }
}
