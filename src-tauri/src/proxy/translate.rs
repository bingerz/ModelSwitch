use serde_json::Value;

use crate::proxy::RequestFormat;

/// Translate a Gemini streaming response chunk to OpenAI SSE format.
/// OpenAI: { model, messages: [{role, content}], stream }
/// Gemini: { contents: [{role: "user"|"model", parts: [{text}]}], generationConfig }
pub fn openai_to_gemini(body: &Value) -> Value {
    let mut config = serde_json::Map::new();

    // Map messages to Gemini contents
    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let contents: Vec<Value> = messages
        .into_iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let gemini_role = match role {
                "system" => return None, // system handled separately
                "assistant" => "model",
                _ => "user",
            };
            let text = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            Some(serde_json::json!({
                "role": gemini_role,
                "parts": [{ "text": text }]
            }))
        })
        .collect();

    config.insert("contents".to_string(), Value::Array(contents));

    // Extract system message
    let messages_arr = body.get("messages").and_then(|m| m.as_array());
    if let Some(msgs) = messages_arr {
        let system_parts: Vec<_> = msgs
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
            .filter_map(|m| {
                m.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| serde_json::json!({ "text": s }))
            })
            .collect();
        if !system_parts.is_empty() {
            config.insert(
                "systemInstruction".to_string(),
                serde_json::json!({
                    "parts": system_parts
                }),
            );
        }
    }

    // Map generation config
    let mut gen_config = serde_json::Map::new();
    if let Some(max_tokens) = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
    {
        gen_config.insert("maxOutputTokens".to_string(), max_tokens.clone());
    }
    if let Some(temp) = body.get("temperature") {
        gen_config.insert("temperature".to_string(), temp.clone());
    }
    if let Some(top_p) = body.get("top_p") {
        gen_config.insert("topP".to_string(), top_p.clone());
    }
    if let Some(stream) = body.get("stream").and_then(|s| s.as_bool()) {
        config.insert("stream".to_string(), Value::Bool(stream));
    }
    if !gen_config.is_empty() {
        config.insert("generationConfig".to_string(), Value::Object(gen_config));
    }

    Value::Object(config)
}

/// Translate a Gemini response to OpenAI-compatible format.
/// Gemini: { candidates: [{content: {parts: [{text}]}, finishReason}], usageMetadata }
/// OpenAI: { id, object, created, model, choices: [{message: {role, content}, finish_reason}], usage }
pub fn gemini_to_openai(body: &Value, model: &str) -> Value {
    let candidates = body
        .get("candidates")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let choices: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let text = c
                .get("content")
                .and_then(|ct| ct.get("parts"))
                .and_then(|p| p.as_array())
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();

            let finish_reason = c
                .get("finishReason")
                .and_then(|f| f.as_str())
                .map(|f| match f {
                    "STOP" => "stop",
                    "MAX_TOKENS" => "length",
                    "SAFETY" => "content_filter",
                    _ => "stop",
                })
                .unwrap_or("stop")
                .to_string();

            serde_json::json!({
                "index": i,
                "message": { "role": "assistant", "content": text },
                "finish_reason": finish_reason
            })
        })
        .collect();

    // Extract usage from Gemini usageMetadata
    let usage = body.get("usageMetadata").map(|u| {
        serde_json::json!({
            "prompt_tokens": u.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0),
            "completion_tokens": u.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0),
            "total_tokens": u.get("totalTokenCount").and_then(|v| v.as_u64()).unwrap_or(0),
        })
    }).unwrap_or_else(|| serde_json::json!({
        "prompt_tokens": 0,
        "completion_tokens": 0,
        "total_tokens": 0,
    }));

    serde_json::json!({
        "id": "gemini-resp",
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": choices,
        "usage": usage
    })
}

/// Translate a Gemini streaming response chunk to OpenAI SSE format.
pub fn gemini_stream_to_openai(chunk: &Value, model: &str) -> Option<String> {
    let candidates = chunk.get("candidates").and_then(|c| c.as_array())?;
    let candidate = candidates.first()?;

    let text = candidate
        .get("content")
        .and_then(|ct| ct.get("parts"))
        .and_then(|p| p.as_array())
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    let finish_reason = candidate
        .get("finishReason")
        .and_then(|f| f.as_str())
        .map(|f| match f {
            "STOP" => "stop",
            "MAX_TOKENS" => "length",
            "SAFETY" => "content_filter",
            _ => "stop",
        })
        .unwrap_or("stop");

    let is_done = finish_reason != "stop" || text.is_empty();

    let sse_chunk = serde_json::json!({
        "id": "gemini-stream",
        "object": "chat.completion.chunk",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": text },
            "finish_reason": if is_done { Some(finish_reason) } else { None }
        }]
    });

    if is_done {
        Some(format!(
            "data: {}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&sse_chunk).ok()?
        ))
    } else {
        Some(format!(
            "data: {}\n\n",
            serde_json::to_string(&sse_chunk).ok()?
        ))
    }
}

// ── Anthropic ↔ OpenAI ──────────────────────────────────────────────────────

/// Convert an Anthropic Messages API request body to OpenAI Chat Completions
/// format, enabling routing of Anthropic clients to OpenAI-compatible upstreams.
///
/// Key transformations:
/// - `system` (string or content-block array) → first `role:"system"` message
/// - Message content blocks (`text`, `image`, `tool_use`, `tool_result`) are
///   mapped to OpenAI equivalents; `tool_result` blocks become standalone
///   `role:"tool"` messages.
/// - `stop_sequences` → `stop` (renamed); `top_k` dropped.
/// - `tools` and `tool_choice` translated to OpenAI function-calling schema.
pub fn anthropic_to_openai_request(body: &Value) -> Value {
    let mut out = serde_json::Map::new();

    // Pass-through scalar fields.
    for key in &["model", "max_tokens", "stream", "temperature", "top_p"] {
        if let Some(v) = body.get(*key) {
            out.insert((*key).to_string(), v.clone());
        }
    }

    // stop_sequences → stop. top_k is intentionally dropped (no OpenAI equiv).
    if let Some(stop) = body.get("stop_sequences") {
        out.insert("stop".to_string(), stop.clone());
    }

    let mut messages: Vec<Value> = Vec::new();

    // Top-level system: may be a string or an array of {type:"text", text:"..."}.
    if let Some(system) = body.get("system") {
        let system_text = match system {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };
        if !system_text.is_empty() {
            messages.push(serde_json::json!({ "role": "system", "content": system_text }));
        }
    }

    // Convert each Anthropic message. Content may be a string or an array of
    // typed blocks. A single Anthropic message can expand into multiple OpenAI
    // messages (tool_result blocks each become a separate role:"tool" entry).
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            match msg.get("content") {
                Some(Value::String(s)) => {
                    messages.push(serde_json::json!({ "role": role, "content": s }));
                }
                Some(Value::Array(blocks)) => {
                    let mut text_parts: Vec<String> = Vec::new();
                    let mut openai_content_parts: Vec<Value> = Vec::new();
                    let mut tool_calls: Vec<Value> = Vec::new();
                    let mut has_multimodal = false;

                    for block in blocks {
                        let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match btype {
                            "text" => {
                                let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                text_parts.push(text.to_string());
                                openai_content_parts
                                    .push(serde_json::json!({ "type": "text", "text": text }));
                            }
                            "image" => {
                                has_multimodal = true;
                                if let Some(source) = block.get("source") {
                                    let media_type = source
                                        .get("media_type")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("image/png");
                                    let data =
                                        source.get("data").and_then(|d| d.as_str()).unwrap_or("");
                                    openai_content_parts.push(serde_json::json!({
                                        "type": "image_url",
                                        "image_url": {
                                            "url": format!("data:{};base64,{}", media_type, data)
                                        }
                                    }));
                                }
                            }
                            "tool_use" => {
                                let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                let input =
                                    block.get("input").cloned().unwrap_or(serde_json::json!({}));
                                let arguments = serde_json::to_string(&input).unwrap_or_default();
                                tool_calls.push(serde_json::json!({
                                    "id": id,
                                    "type": "function",
                                    "function": { "name": name, "arguments": arguments }
                                }));
                            }
                            "tool_result" => {
                                // Each tool_result becomes its own role:"tool" message.
                                let tool_use_id = block
                                    .get("tool_use_id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("");
                                let content =
                                    block.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                messages.push(serde_json::json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content
                                }));
                            }
                            _ => {}
                        }
                    }

                    // Emit the assembled message when non-tool_result content exists.
                    let has_main_content =
                        !text_parts.is_empty() || has_multimodal || !tool_calls.is_empty();
                    if has_main_content {
                        let mut msg_obj = serde_json::Map::new();
                        msg_obj.insert("role".to_string(), Value::String(role.to_string()));

                        if !tool_calls.is_empty() {
                            // Assistant message with tool_calls: content may be
                            // null (OpenAI accepts null when tool_calls present).
                            if has_multimodal {
                                msg_obj.insert(
                                    "content".to_string(),
                                    Value::Array(openai_content_parts),
                                );
                            } else if text_parts.is_empty() {
                                msg_obj.insert("content".to_string(), Value::Null);
                            } else {
                                msg_obj.insert(
                                    "content".to_string(),
                                    Value::String(text_parts.join("")),
                                );
                            }
                            msg_obj.insert("tool_calls".to_string(), Value::Array(tool_calls));
                        } else if has_multimodal {
                            msg_obj
                                .insert("content".to_string(), Value::Array(openai_content_parts));
                        } else {
                            msg_obj
                                .insert("content".to_string(), Value::String(text_parts.join("")));
                        }
                        messages.push(Value::Object(msg_obj));
                    }
                }
                _ => {
                    // Missing or null content: emit with empty string to keep
                    // the message in the conversation.
                    messages.push(serde_json::json!({ "role": role, "content": "" }));
                }
            }
        }
    }

    out.insert("messages".to_string(), Value::Array(messages));

    // Convert tools: {name, description, input_schema} → OpenAI function tool.
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let description = tool
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let parameters = tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({ "type": "object" }));
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                })
            })
            .collect();
        out.insert("tools".to_string(), Value::Array(openai_tools));
    }

    // Convert tool_choice.
    if let Some(tc) = body.get("tool_choice") {
        let tc_type = tc.get("type").and_then(|t| t.as_str());
        let converted = match tc_type {
            Some("auto") => Value::String("auto".to_string()),
            Some("any") => Value::String("required".to_string()),
            Some("tool") => {
                let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                serde_json::json!({ "type": "function", "function": { "name": name } })
            }
            _ => Value::String("auto".to_string()),
        };
        out.insert("tool_choice".to_string(), converted);
    }

    Value::Object(out)
}

/// Convert an OpenAI Chat Completions response to Anthropic Messages format.
///
/// Maps finish_reason, usage tokens, and tool_calls to their Anthropic
/// equivalents. The returned object matches the Anthropic `Message` schema.
pub fn openai_to_anthropic_response(body: &Value) -> Value {
    let choice = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());
    let message = choice
        .and_then(|c| c.get("message"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason").and_then(|f| f.as_str()))
        .unwrap_or("stop");

    let mut content: Vec<Value> = Vec::new();

    // Text content (skip null/empty — OpenAI returns null when tool_calls only).
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content.push(serde_json::json!({ "type": "text", "text": text }));
        }
    }

    // Tool calls → tool_use content blocks.
    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let id = tc
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let function = tc
                .get("function")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let name = function
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let arguments_str = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let input: Value =
                serde_json::from_str(arguments_str).unwrap_or_else(|_| serde_json::json!({}));
            content.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }

    let stop_reason = match finish_reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" => "tool_use",
        "content_filter" => "end_turn",
        _ => "end_turn",
    };

    let usage = body
        .get("usage")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(|p| p.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(|c| c.as_u64())
        .unwrap_or(0);

    let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    serde_json::json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": model,
        "stop_reason": stop_reason,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

/// Convert a single OpenAI SSE chunk to Anthropic SSE format.
///
/// Returns `None` for chunks that should be dropped (e.g. delta with neither
/// content nor finish_reason). The returned string, if any, is a complete SSE
/// event block ready to be written to the client stream.
pub fn openai_to_anthropic_stream_chunk(chunk: &Value) -> Option<String> {
    let choice = chunk
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())?;
    let delta = choice
        .get("delta")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let finish_reason = choice.get("finish_reason").and_then(|f| f.as_str());

    // Text delta → content_block_delta with text_delta.
    if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            let event = serde_json::json!({
                "type": "content_block_delta",
                "delta": { "type": "text_delta", "text": text }
            });
            return Some(format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::to_string(&event).ok()?
            ));
        }
    }

    // Tool-call argument delta → input_json_delta.
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
        let mut parts: Vec<String> = Vec::new();
        for tc in tool_calls {
            if let Some(args) = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
            {
                let event = serde_json::json!({
                    "type": "content_block_delta",
                    "delta": { "type": "input_json_delta", "partial_json": args }
                });
                parts.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    serde_json::to_string(&event).ok()?
                ));
            }
        }
        if !parts.is_empty() {
            return Some(parts.concat());
        }
    }

    // Finish reason → message_delta (usage) + message_stop.
    if finish_reason.is_some() {
        if let Some(usage) = chunk.get("usage") {
            let output_tokens = usage
                .get("completion_tokens")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let usage_event = serde_json::json!({
                "type": "message_delta",
                "usage": { "output_tokens": output_tokens }
            });
            let stop_event = serde_json::json!({ "type": "message_stop" });
            return Some(format!(
                "event: message_delta\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
                serde_json::to_string(&usage_event).ok()?,
                serde_json::to_string(&stop_event).ok()?
            ));
        }
        let stop_event = serde_json::json!({ "type": "message_stop" });
        return Some(format!(
            "event: message_stop\ndata: {}\n\n",
            serde_json::to_string(&stop_event).ok()?
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_openai_to_gemini() {
        let openai = serde_json::json!({
            "model": "gemini-pro",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hello" }
            ],
            "temperature": 0.7,
            "max_tokens": 100
        });
        let gemini = openai_to_gemini(&openai);
        assert!(gemini.get("systemInstruction").is_some());
        let contents = gemini.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(gemini["generationConfig"]["temperature"], 0.7);
    }

    #[test]
    fn translates_gemini_to_openai() {
        let gemini = serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Hi there!" }], "role": "model" },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        });
        let openai = gemini_to_openai(&gemini, "gemini-pro");
        assert_eq!(openai["choices"][0]["message"]["content"], "Hi there!");
        assert_eq!(openai["choices"][0]["finish_reason"], "stop");
        assert_eq!(openai["usage"]["prompt_tokens"], 10);
    }

    // ── Anthropic ↔ OpenAI ──────────────────────────────────────────────────

    #[test]
    fn anthropic_to_openai_basic_request() {
        let anthropic = serde_json::json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello!"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["model"], "claude-3-sonnet");
        assert_eq!(openai["max_tokens"], 1024);
        assert_eq!(openai["messages"][0]["role"], "user");
        assert_eq!(openai["messages"][0]["content"], "Hello!");
    }

    #[test]
    fn anthropic_to_openai_with_system() {
        let anthropic = serde_json::json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": "You are helpful.",
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][0]["content"], "You are helpful.");
        assert_eq!(openai["messages"][1]["role"], "user");
    }

    #[test]
    fn anthropic_to_openai_system_array() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "system": [{"type": "text", "text": "Rule 1."}, {"type": "text", "text": "Rule 2."}],
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][0]["content"], "Rule 1.Rule 2.");
    }

    #[test]
    fn anthropic_to_openai_stop_sequences() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "stop_sequences": ["END"],
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["stop"], serde_json::json!(["END"]));
        assert!(openai.get("stop_sequences").is_none());
    }

    #[test]
    fn anthropic_to_openai_drops_top_k() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "top_k": 40,
            "top_p": 0.9,
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert!(openai.get("top_k").is_none());
        assert_eq!(openai["top_p"], 0.9);
    }

    #[test]
    fn anthropic_to_openai_tools() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object", "properties": {"loc": {"type": "string"}}}
            }],
            "messages": [{"role": "user", "content": "Weather?"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["tools"][0]["type"], "function");
        assert_eq!(openai["tools"][0]["function"]["name"], "get_weather");
        assert!(openai["tools"][0]["function"]["parameters"].is_object());
    }

    #[test]
    fn anthropic_to_openai_tool_choice() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "tool_choice": {"type": "any"},
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        assert_eq!(openai["tool_choice"], "required");

        let anthropic_named = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "tool_choice": {"type": "tool", "name": "get_weather"},
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let openai_named = anthropic_to_openai_request(&anthropic_named);
        assert_eq!(openai_named["tool_choice"]["type"], "function");
        assert_eq!(
            openai_named["tool_choice"]["function"]["name"],
            "get_weather"
        );
    }

    #[test]
    fn anthropic_to_openai_tool_use_blocks() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Calling tool."},
                    {"type": "tool_use", "id": "call_1", "name": "get_weather", "input": {"loc": "NYC"}}
                ]
            }]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        let msg = &openai["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "Calling tool.");
        assert_eq!(msg["tool_calls"][0]["id"], "call_1");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(
            msg["tool_calls"][0]["function"]["arguments"],
            serde_json::to_string(&serde_json::json!({"loc": "NYC"})).unwrap()
        );
    }

    #[test]
    fn anthropic_to_openai_tool_result_becomes_tool_message() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "messages": [
                {"role": "user", "content": "Weather?"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "call_1", "name": "get_weather", "input": {"loc": "NYC"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "Sunny"}]}
            ]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        let messages = openai["messages"].as_array().unwrap();
        // user, assistant, tool
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["content"], "Sunny");
    }

    #[test]
    fn anthropic_to_openai_image_block() {
        let anthropic = serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is this?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBOR..."}}
                ]
            }]
        });
        let openai = anthropic_to_openai_request(&anthropic);
        let content = openai["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn openai_to_anthropic_basic_response() {
        let openai = serde_json::json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let anthropic = openai_to_anthropic_response(&openai);
        assert_eq!(anthropic["type"], "message");
        assert_eq!(anthropic["role"], "assistant");
        assert_eq!(anthropic["content"][0]["type"], "text");
        assert_eq!(anthropic["content"][0]["text"], "Hello!");
        assert_eq!(anthropic["stop_reason"], "end_turn");
        assert_eq!(anthropic["usage"]["input_tokens"], 10);
        assert_eq!(anthropic["usage"]["output_tokens"], 5);
        assert!(anthropic["id"].as_str().unwrap().starts_with("msg_"));
    }

    #[test]
    fn openai_to_anthropic_tool_calls_response() {
        let openai = serde_json::json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"loc\": \"NYC\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20}
        });
        let anthropic = openai_to_anthropic_response(&openai);
        assert_eq!(anthropic["stop_reason"], "tool_use");
        assert_eq!(anthropic["content"][0]["type"], "tool_use");
        assert_eq!(anthropic["content"][0]["name"], "get_weather");
        assert!(anthropic["content"][0]["input"].is_object());
        assert_eq!(anthropic["content"][0]["input"]["loc"], "NYC");
    }

    #[test]
    fn openai_to_anthropic_length_finish() {
        let openai = serde_json::json!({
            "id": "x", "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "..."}, "finish_reason": "length"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10}
        });
        let anthropic = openai_to_anthropic_response(&openai);
        assert_eq!(anthropic["stop_reason"], "max_tokens");
    }

    #[test]
    fn openai_to_anthropic_content_filter_finish() {
        let openai = serde_json::json!({
            "id": "x", "model": "gpt-4",
            "choices": [{"message": {"role": "assistant", "content": "..."}, "finish_reason": "content_filter"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10}
        });
        let anthropic = openai_to_anthropic_response(&openai);
        assert_eq!(anthropic["stop_reason"], "end_turn");
    }

    #[test]
    fn openai_to_anthropic_stream_text_delta() {
        let chunk = serde_json::json!({
            "choices": [{"delta": {"content": "Hello"}, "finish_reason": null}]
        });
        let result = openai_to_anthropic_stream_chunk(&chunk);
        assert!(result.is_some());
        let s = result.unwrap();
        assert!(s.contains("content_block_delta"));
        assert!(s.contains("text_delta"));
        assert!(s.contains("Hello"));
    }

    #[test]
    fn openai_to_anthropic_stream_stop() {
        let chunk = serde_json::json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let result = openai_to_anthropic_stream_chunk(&chunk);
        assert!(result.is_some());
        let s = result.unwrap();
        assert!(s.contains("message_delta"));
        assert!(s.contains("message_stop"));
        assert!(s.contains("\"output_tokens\":5"));
    }

    #[test]
    fn openai_to_anthropic_stream_stop_without_usage() {
        let chunk = serde_json::json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}]
        });
        let result = openai_to_anthropic_stream_chunk(&chunk);
        assert!(result.is_some());
        let s = result.unwrap();
        assert!(s.contains("message_stop"));
        assert!(!s.contains("message_delta"));
    }

    #[test]
    fn openai_to_anthropic_stream_empty_delta_dropped() {
        // Delta with neither content nor finish_reason → None (dropped).
        let chunk = serde_json::json!({
            "choices": [{"delta": {"role": "assistant"}, "finish_reason": null}]
        });
        let result = openai_to_anthropic_stream_chunk(&chunk);
        assert!(result.is_none());
    }

    #[test]
    fn openai_to_anthropic_stream_tool_call_delta() {
        let chunk = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"name": "get_weather", "arguments": "{\"loc\":"}
                    }]
                },
                "finish_reason": null
            }]
        });
        let result = openai_to_anthropic_stream_chunk(&chunk);
        assert!(result.is_some());
        let s = result.unwrap();
        assert!(s.contains("input_json_delta"));
        assert!(s.contains("partial_json"));
    }
}
