//! Translation between MCP tool schemas and OpenAI/Anthropic function-calling formats.
//!
//! MCP tools use JSON Schema for their `input_schema`. OpenAI expects a
//! `parameters` field with a JSON Schema object, while Anthropic uses
//! `input_schema`. Both providers tolerate (and Anthropic actively requires)
//! a root `"type": "object"` and reject some metadata fields.

use crate::mcp::aggregator::AggregatedTool;
use serde_json::{json, Map, Value};

/// Parsed MCP tool-call routing parameters returned by [`parse_mcp_tool_call`].
///
/// `(server_id, original_name, arguments)` — `arguments` is `None` when the
/// LLM produced empty or `"null"` arguments.
pub type ParsedMcpToolCall = (String, String, Option<Map<String, Value>>);

/// Translate an `AggregatedTool` to OpenAI function calling format.
///
/// Produces:
/// ```json
/// {
///   "type": "function",
///   "function": {
///     "name": "mcp__fs__read",
///     "description": "...",
///     "parameters": { ... sanitized JSON Schema ... }
///   }
/// }
/// ```
pub fn to_openai_function(tool: &AggregatedTool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.namespaced_name,
            "description": tool.description.as_deref().unwrap_or(""),
            "parameters": sanitize_schema(&tool.input_schema),
        }
    })
}

/// Translate an `AggregatedTool` to Anthropic tool format.
///
/// Produces:
/// ```json
/// {
///   "name": "mcp__fs__read",
///   "description": "...",
///   "input_schema": { ... sanitized JSON Schema ... }
/// }
/// ```
#[allow(dead_code)] // consumed in Phase 3.3 (tool-call interception)
pub fn to_anthropic_tool(tool: &AggregatedTool) -> Value {
    json!({
        "name": tool.namespaced_name,
        "description": tool.description.as_deref().unwrap_or(""),
        "input_schema": sanitize_schema(&tool.input_schema),
    })
}

/// Sanitize a JSON Schema for LLM provider compatibility.
///
/// - Removes `$schema`, `$id`, `title` (metadata some providers reject).
/// - Ensures `"type": "object"` is present at the root.
/// - Non-object schemas are replaced with an empty object schema.
///
/// This function clones the input; it never mutates the caller's `Value`.
pub fn sanitize_schema(schema: &Value) -> Value {
    let mut result = match schema {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({"type": "object", "properties": {}}),
    };

    if let Some(obj) = result.as_object_mut() {
        obj.remove("$schema");
        obj.remove("$id");
        obj.remove("title");

        if !obj.contains_key("type") {
            obj.insert("type".to_string(), json!("object"));
        }
    }

    result
}

/// Parse an OpenAI/Anthropic tool call back into MCP routing parameters.
///
/// `namespaced_name` is the tool name returned by the LLM (e.g.
/// `mcp__fs__read_file`). `arguments` is the raw JSON arguments string
/// from the LLM response (OpenAI sends it as a string; Anthropic sends
/// a parsed object — callers should serialize Anthropic arguments before
/// calling this function).
///
/// Returns `Some((server_id, original_name, arguments))` if the call
/// targets an MCP tool, or `None` if the name is not namespaced.
/// `arguments` is `None` when the LLM produced empty or `"null"` arguments.
#[allow(dead_code)] // consumed in Phase 3.3 (tool-call interception)
pub fn parse_mcp_tool_call(namespaced_name: &str, arguments: &str) -> Option<ParsedMcpToolCall> {
    let (server_id, original_name) = AggregatedTool::parse_namespaced(namespaced_name)?;

    let arguments = if arguments.trim().is_empty() || arguments.trim() == "null" {
        None
    } else {
        match serde_json::from_str::<Value>(arguments) {
            Ok(Value::Object(map)) => Some(map),
            Ok(_) => None,
            Err(_) => None,
        }
    };

    Some((server_id.to_string(), original_name.to_string(), arguments))
}

/// Flatten an MCP `CallToolResult` into a string suitable for returning to
/// an LLM as tool-call output.
///
/// MCP tools can return multiple content blocks (text, image, audio,
/// resource). For LLM tool results we concatenate text content verbatim
/// and replace non-text blocks with short placeholders so the model knows
/// something was returned but cannot consume raw binary.
///
/// If `is_error` is set on the result, the body is prefixed with `"Error: "`.
#[allow(dead_code)] // consumed in Phase 3.3 (tool-call interception)
pub fn flatten_tool_result(result: &rmcp::model::CallToolResult) -> String {
    use rmcp::model::RawContent;

    let mut parts: Vec<String> = Vec::new();

    for content in &result.content {
        match &content.raw {
            RawContent::Text(t) => parts.push(t.text.clone()),
            RawContent::Image(_) => parts.push("[image content omitted]".to_string()),
            RawContent::Audio(_) => parts.push("[audio content omitted]".to_string()),
            RawContent::Resource(_) => parts.push("[resource content]".to_string()),
            RawContent::ResourceLink(_) => parts.push("[resource link]".to_string()),
        }
    }

    let body = if parts.is_empty() {
        "[empty result]".to_string()
    } else {
        parts.join("\n")
    };

    if result.is_error.unwrap_or(false) {
        format!("Error: {}", body)
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tool() -> AggregatedTool {
        AggregatedTool {
            namespaced_name: "mcp__fs__read".into(),
            original_name: "read".into(),
            server_id: "fs".into(),
            description: Some("Read a file".into()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        }
    }

    #[test]
    fn openai_function_format() {
        let tool = sample_tool();
        let result = to_openai_function(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "mcp__fs__read");
        assert_eq!(result["function"]["description"], "Read a file");
        assert_eq!(result["function"]["parameters"]["type"], "object");
        assert!(result["function"]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn openai_function_uses_empty_description_when_missing() {
        let mut tool = sample_tool();
        tool.description = None;
        let result = to_openai_function(&tool);
        assert_eq!(result["function"]["description"], "");
    }

    #[test]
    fn anthropic_tool_format() {
        let tool = sample_tool();
        let result = to_anthropic_tool(&tool);
        assert_eq!(result["name"], "mcp__fs__read");
        assert_eq!(result["description"], "Read a file");
        assert_eq!(result["input_schema"]["type"], "object");
    }

    #[test]
    fn sanitization_strips_metadata() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://example.com/schema.json",
            "title": "MySchema",
            "type": "object",
            "properties": {}
        });
        let result = sanitize_schema(&schema);
        assert!(result.get("$schema").is_none());
        assert!(result.get("$id").is_none());
        assert!(result.get("title").is_none());
        assert_eq!(result["type"], "object");
        assert!(result["properties"].is_object());
    }

    #[test]
    fn sanitization_adds_type_if_missing() {
        let schema = json!({"properties": {}});
        let result = sanitize_schema(&schema);
        assert_eq!(result["type"], "object");
    }

    #[test]
    fn sanitization_replaces_non_object_with_empty_object_schema() {
        let schema = json!("not an object");
        let result = sanitize_schema(&schema);
        assert_eq!(result["type"], "object");
        assert!(result["properties"].is_object());
    }

    #[test]
    fn sanitization_preserves_existing_type() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "number"}}});
        let result = sanitize_schema(&schema);
        assert_eq!(result["type"], "object");
        assert!(result["properties"]["x"].is_object());
    }

    #[test]
    fn parse_tool_call_extracts_server_and_name() {
        let (server, name, args) =
            parse_mcp_tool_call("mcp__fs__read_file", r#"{"path": "/tmp/test"}"#).unwrap();
        assert_eq!(server, "fs");
        assert_eq!(name, "read_file");
        assert_eq!(args.unwrap().get("path").unwrap(), "/tmp/test");
    }

    #[test]
    fn parse_tool_call_handles_empty_arguments() {
        let (server, name, args) = parse_mcp_tool_call("mcp__fs__noop", "").unwrap();
        assert_eq!(server, "fs");
        assert_eq!(name, "noop");
        assert!(args.is_none());
    }

    #[test]
    fn parse_tool_call_handles_null_arguments() {
        let (_server, _name, args) = parse_mcp_tool_call("mcp__fs__noop", "null").unwrap();
        assert!(args.is_none());
    }

    #[test]
    fn parse_tool_call_handles_whitespace_arguments() {
        let (_server, _name, args) = parse_mcp_tool_call("mcp__fs__noop", "   ").unwrap();
        assert!(args.is_none());
    }

    #[test]
    fn parse_tool_call_non_mcp_returns_none() {
        assert!(parse_mcp_tool_call("regular_function", "{}").is_none());
    }

    #[test]
    fn parse_tool_call_non_object_arguments_become_none() {
        // Array / primitive arguments are not valid tool inputs in MCP semantics.
        let (_s, _n, args) = parse_mcp_tool_call("mcp__fs__t", "[1,2,3]").unwrap();
        assert!(args.is_none());
    }

    #[test]
    fn flatten_tool_result_concatenates_text() {
        use rmcp::model::{CallToolResult, Content};

        let result =
            CallToolResult::success(vec![Content::text("line 1"), Content::text("line 2")]);

        assert_eq!(flatten_tool_result(&result), "line 1\nline 2");
    }

    #[test]
    fn flatten_tool_result_marks_image_and_audio() {
        use rmcp::model::{AnnotateAble, CallToolResult, Content, RawAudioContent, RawContent};

        let audio_content = RawContent::Audio(RawAudioContent {
            data: "base64...".into(),
            mime_type: "audio/wav".into(),
        })
        .no_annotation();

        let result = CallToolResult::success(vec![
            Content::text("ok"),
            Content::image("base64...", "image/png"),
            audio_content,
        ]);

        let body = flatten_tool_result(&result);
        assert!(body.contains("ok"));
        assert!(body.contains("[image content omitted]"));
        assert!(body.contains("[audio content omitted]"));
    }

    #[test]
    fn flatten_tool_result_prefixes_error() {
        use rmcp::model::{CallToolResult, Content};

        let result = CallToolResult::error(vec![Content::text("file not found")]);

        assert_eq!(flatten_tool_result(&result), "Error: file not found");
    }

    #[test]
    fn flatten_tool_result_empty_content() {
        use rmcp::model::CallToolResult;

        // CallToolResult::default gives empty content and is_error=None.
        let result = CallToolResult::default();

        assert_eq!(flatten_tool_result(&result), "[empty result]");
    }
}
