use serde_json::Value;

/// Token usage extracted from an upstream API response.
pub(super) struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
}

/// Extract token usage from an upstream response body.
pub(super) fn extract_usage(body: &str) -> TokenUsage {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return TokenUsage {
                input_tokens: None,
                output_tokens: None,
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            }
        }
    };
    let usage = match v.get("usage") {
        Some(u) => u,
        None => {
            return TokenUsage {
                input_tokens: None,
                output_tokens: None,
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            }
        }
    };
    // OpenAI/DeepSeek: prompt_tokens / completion_tokens
    // Anthropic: input_tokens / output_tokens
    let input = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("input_tokens").and_then(|v| v.as_u64()));
    let output = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("output_tokens").and_then(|v| v.as_u64()));
    // DeepSeek: prompt_cache_hit_tokens / prompt_cache_miss_tokens
    // Anthropic: cache_read_input_tokens / cache_creation_input_tokens
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
        });
    let cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
        });
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_hit_tokens: cache_hit,
        cache_miss_tokens: cache_miss,
    }
}

/// Extract token usage from accumulated SSE stream data.
/// Looks for usage in the last few SSE chunks before [DONE].
pub(super) fn extract_usage_from_stream(chunks: &[String]) -> TokenUsage {
    let mut anthropic_input: Option<u64> = None;
    let mut anthropic_output: Option<u64> = None;
    let mut anthropic_cache_read: Option<u64> = None;
    let mut anthropic_cache_creation: Option<u64> = None;

    for chunk in chunks.iter().rev().take(10) {
        if chunk.trim() == "[DONE]" || chunk.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(chunk) {
            let event_type = v.get("type").and_then(|t| t.as_str());
            // Anthropic message_delta: usage.output_tokens
            if event_type == Some("message_delta") {
                if let Some(usage) = v.get("usage") {
                    anthropic_output = usage.get("output_tokens").and_then(|t| t.as_u64());
                }
                continue;
            }
            // Anthropic message_start: message.usage.input_tokens + cache
            if event_type == Some("message_start") {
                if let Some(message) = v.get("message") {
                    if let Some(usage) = message.get("usage") {
                        anthropic_input = usage.get("input_tokens").and_then(|t| t.as_u64());
                        anthropic_cache_read = usage
                            .get("cache_read_input_tokens")
                            .and_then(|t| t.as_u64());
                        anthropic_cache_creation = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|t| t.as_u64());
                    }
                }
                continue;
            }
        }
        // OpenAI/DeepSeek: use extract_usage for chunks without Anthropic type
        let token_usage = extract_usage(chunk);
        if token_usage.input_tokens.is_some() || token_usage.output_tokens.is_some() {
            return token_usage;
        }
    }
    if anthropic_input.is_some() || anthropic_output.is_some() {
        return TokenUsage {
            input_tokens: anthropic_input,
            output_tokens: anthropic_output,
            cache_hit_tokens: anthropic_cache_read,
            cache_miss_tokens: anthropic_cache_creation,
        };
    }
    TokenUsage {
        input_tokens: None,
        output_tokens: None,
        cache_hit_tokens: None,
        cache_miss_tokens: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_deepseek_usage_with_cache() {
        let body = r#"{"id":"chatcmpl-123","usage":{"prompt_tokens":1500,"completion_tokens":300,"prompt_cache_hit_tokens":1200,"prompt_cache_miss_tokens":300}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(300));
        assert_eq!(usage.cache_hit_tokens, Some(1200));
        assert_eq!(usage.cache_miss_tokens, Some(300));
    }

    #[test]
    fn extract_anthropic_usage_with_cache() {
        let body = r#"{"id":"msg_123","usage":{"input_tokens":2000,"output_tokens":500,"cache_read_input_tokens":1800,"cache_creation_input_tokens":200}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(2000));
        assert_eq!(usage.output_tokens, Some(500));
        assert_eq!(usage.cache_hit_tokens, Some(1800));
        assert_eq!(usage.cache_miss_tokens, Some(200));
    }

    #[test]
    fn extract_openai_usage_no_cache() {
        let body = r#"{"id":"chatcmpl-456","usage":{"prompt_tokens":100,"completion_tokens":50}}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_miss_tokens, None);
    }

    #[test]
    fn extract_usage_no_usage_field() {
        let body = r#"{"id":"chatcmpl-789","choices":[]}"#;
        let usage = extract_usage(body);
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_miss_tokens, None);
    }

    #[test]
    fn extract_stream_deepseek_usage() {
        let chunks = vec![
            r#"{"id":"chatcmpl-1","choices":[{"delta":{"content":"Hi"}}]}"#.to_string(),
            r#"{"id":"chatcmpl-1","usage":{"prompt_tokens":800,"completion_tokens":100,"prompt_cache_hit_tokens":600,"prompt_cache_miss_tokens":200}}"#.to_string(),
        ];
        let usage = extract_usage_from_stream(&chunks);
        assert_eq!(usage.input_tokens, Some(800));
        assert_eq!(usage.output_tokens, Some(100));
        assert_eq!(usage.cache_hit_tokens, Some(600));
        assert_eq!(usage.cache_miss_tokens, Some(200));
    }

    #[test]
    fn extract_stream_anthropic_usage() {
        let chunks = vec![
            r#"{"type":"message_start","message":{"usage":{"input_tokens":3000,"cache_read_input_tokens":2500,"cache_creation_input_tokens":500}}}"#.to_string(),
            r#"{"type":"message_delta","usage":{"output_tokens":400}}"#.to_string(),
        ];
        let usage = extract_usage_from_stream(&chunks);
        assert_eq!(usage.input_tokens, Some(3000));
        assert_eq!(usage.output_tokens, Some(400));
        assert_eq!(usage.cache_hit_tokens, Some(2500));
        assert_eq!(usage.cache_miss_tokens, Some(500));
    }
}
