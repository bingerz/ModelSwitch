use serde_json::Value;

/// Translate an OpenAI-format request body to Gemini format.
/// OpenAI: { model, messages: [{role, content}], stream }
/// Gemini: { contents: [{role: "user"|"model", parts: [{text}]}], generationConfig }
pub fn openai_to_gemini(body: &Value) -> Value {
    let mut config = serde_json::Map::new();

    // Map messages to Gemini contents
    let messages = body.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    let contents: Vec<Value> = messages
        .into_iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let gemini_role = match role {
                "system" => return None, // system handled separately
                "assistant" => "model",
                _ => "user",
            };
            let text = msg.get("content")
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
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(|s| serde_json::json!({ "text": s })))
            .collect();
        if !system_parts.is_empty() {
            config.insert("systemInstruction".to_string(), serde_json::json!({
                "parts": system_parts
            }));
        }
    }

    // Map generation config
    let mut gen_config = serde_json::Map::new();
    if let Some(max_tokens) = body.get("max_tokens").or_else(|| body.get("max_completion_tokens")) {
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
    let candidates = body.get("candidates").and_then(|c| c.as_array()).cloned().unwrap_or_default();

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
        Some(format!("data: {}\n\ndata: [DONE]\n\n", serde_json::to_string(&sse_chunk).ok()?))
    } else {
        Some(format!("data: {}\n\n", serde_json::to_string(&sse_chunk).ok()?))
    }
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
}
