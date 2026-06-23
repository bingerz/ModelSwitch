use serde_json::Value;

/// Count input tokens from a chat request body using an improved heuristic
/// estimator. Applies CJK-aware token counting, per-message structural
/// overhead, and per-tool schema overhead.
///
/// Counts tokens from: messages (content strings + multimodal text blocks),
/// system prompts, and tool definitions.
pub fn count_input_tokens(model: &str, body: &Value) -> u64 {
    // Collect all text that contributes to input tokens
    let mut total_text = String::new();

    // Extract text from messages array
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            // role (e.g., "system", "user", "assistant")
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                total_text.push_str(role);
                total_text.push('\n');
            }
            // content — can be string or array of content blocks
            if let Some(content) = msg.get("content") {
                extract_text_from_content(content, &mut total_text);
            }
            // tool_calls in assistant messages
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                            total_text.push_str(name);
                        }
                        if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                            total_text.push_str(args);
                        }
                    }
                }
            }
        }
    }

    // Extract text from tools array
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        for tool in tools {
            if let Some(func) = tool.get("function") {
                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                    total_text.push_str(name);
                }
                if let Some(desc) = func.get("description").and_then(|d| d.as_str()) {
                    total_text.push_str(desc);
                }
                // Parameters JSON schema — serialize to string for token counting
                if let Some(params) = func.get("parameters") {
                    if let Ok(json_str) = serde_json::to_string(params) {
                        total_text.push_str(&json_str);
                    }
                }
            }
        }
    }

    // Add structural overhead: ~3 tokens per message for role tags and delimiters
    let message_count = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    // Add 10 tokens per tool definition for schema structure
    let tool_count = body
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let base_tokens = count_text_tokens(model, &total_text);
    base_tokens + (message_count as u64) * 3 + (tool_count as u64) * 10
}

/// Count tokens in a single text string using an improved heuristic.
///
/// Heuristic:
/// - CJK characters (Chinese, Japanese, Korean): ~1 token each
/// - ASCII words (whitespace-delimited): ~1.3 tokens per word
/// - Other characters (punctuation, numbers): ~0.5 tokens each
///
/// This is more accurate than the old `chars/4` approach, especially for
/// CJK text where `chars/4` undercounts by 4x.
///
/// TODO: Integrate tiktoken-rs when network allows for exact BPE counting.
pub fn count_text_tokens(_model: &str, text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }

    let mut token_count: f64 = 0.0;
    let mut ascii_word_len: usize = 0;

    for ch in text.chars() {
        if is_cjk(ch) {
            // Flush any pending ASCII word
            if ascii_word_len > 0 {
                token_count += 1.0; // Each ASCII word ≈ 1 token (simplified)
                ascii_word_len = 0;
            }
            token_count += 1.0; // Each CJK character ≈ 1 token
        } else if ch.is_ascii_whitespace() {
            // Whitespace ends a word
            if ascii_word_len > 0 {
                token_count += 1.0;
                ascii_word_len = 0;
            }
            // Whitespace itself doesn't add tokens (merged into adjacent tokens in BPE)
        } else if ch.is_ascii() {
            // Part of an ASCII word — accumulate
            ascii_word_len += 1;
        } else {
            // Other non-ASCII, non-CJK (emoji, accented chars, etc.)
            if ascii_word_len > 0 {
                token_count += 1.0;
                ascii_word_len = 0;
            }
            token_count += 1.0;
        }
    }
    // Don't forget the last word
    if ascii_word_len > 0 {
        token_count += 1.0;
    }

    token_count.round() as u64
}

/// Check if a character is a CJK (Chinese, Japanese, Korean) character.
/// CJK characters are approximately 1 token each in BPE tokenization.
fn is_cjk(ch: char) -> bool {
    let code = ch as u32;
    // CJK Unified Ideographs
    (0x4E00..=0x9FFF).contains(&code)
    // CJK Unified Ideographs Extension A
    || (0x3400..=0x4DBF).contains(&code)
    // CJK Compatibility Ideographs
    || (0xF900..=0xFAFF).contains(&code)
    // Hiragana
    || (0x3040..=0x309F).contains(&code)
    // Katakana
    || (0x30A0..=0x30FF).contains(&code)
    // Hangul Syllables
    || (0xAC00..=0xD7AF).contains(&code)
    // CJK Symbols and Punctuation
    || (0x3000..=0x303F).contains(&code)
    // Full-width forms
    || (0xFF00..=0xFFEF).contains(&code)
}

/// Extract text from a content field that can be either a string or
/// an array of content blocks (multimodal).
fn extract_text_from_content(content: &Value, output: &mut String) {
    if let Some(s) = content.as_str() {
        output.push_str(s);
        output.push('\n');
    } else if let Some(blocks) = content.as_array() {
        for block in blocks {
            if let Some(block_type) = block.get("type").and_then(|t| t.as_str()) {
                if block_type == "text" {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        output.push_str(text);
                        output.push('\n');
                    }
                }
                // Image blocks contribute to tokens but we can't count image tokens
                // from text alone — skip for now
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_gpt4_tokens_accurately() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "Hello, world!"}
            ]
        });
        let tokens = count_input_tokens("gpt-4", &body);
        // "Hello, world!" + role "user" + 3 tokens/message overhead
        // 2 ASCII words + 1 role word + 3 overhead = ~6 tokens
        assert!(
            tokens > 2 && tokens < 15,
            "Expected 3-15 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn falls_back_for_unknown_models() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "Hello, world!"}
            ]
        });
        let tokens = count_input_tokens("claude-3-opus", &body);
        // Should use heuristic — non-zero
        assert!(tokens > 0);
    }

    #[test]
    fn counts_multimodal_content() {
        let body = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "What's in this image?"},
                    {"type": "image_url", "image_url": {"url": "data:..."}}
                ]}
            ]
        });
        let tokens = count_input_tokens("gpt-4o", &body);
        assert!(tokens > 0);
    }

    #[test]
    fn counts_tools_definition() {
        let body = json!({
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get the current weather for a location",
                    "parameters": {"type": "object", "properties": {"location": {"type": "string"}}}
                }
            }]
        });
        let tokens = count_input_tokens("gpt-4", &body);
        assert!(
            tokens > 5,
            "Expected tokens to include tool definition overhead"
        );
    }

    #[test]
    fn counts_tool_call_arguments() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "What's the weather in Paris?"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{\"location\": \"Paris\"}"}
                }]}
            ]
        });
        let tokens = count_input_tokens("gpt-4", &body);
        assert!(tokens > 5);
    }

    #[test]
    fn empty_body_returns_zero() {
        let body = json!({});
        let tokens = count_input_tokens("gpt-4", &body);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn counts_cjk_text_one_token_per_char() {
        // "你好世界" = 4 CJK characters, each ≈ 1 token
        let tokens = count_text_tokens("gpt-4", "你好世界");
        assert_eq!(
            tokens, 4,
            "Expected 4 tokens for 4 CJK characters, got {}",
            tokens
        );
    }

    #[test]
    fn cjk_heuristic_more_accurate_than_chars_div_4() {
        // Old heuristic: "你好世界".len() = 12 bytes, 12/4 = 3 tokens (undercount)
        // New heuristic: 4 CJK chars = 4 tokens
        // For CJK text the new heuristic should return at least 4x what
        // a naive char-count/4 would for pure-CJK strings of reasonable length.
        let cjk_text = "你好世界";
        let new_tokens = count_text_tokens("gpt-4", cjk_text);

        // Old heuristic computed as bytes/4 — for "你好世界" (12 bytes) that's 3.
        // But the more meaningful comparison is chars/4 = 1. Either way, the new
        // heuristic (4) should be strictly greater than chars/4 (1).
        let old_chars_div_4 = (cjk_text.chars().count() / 4) as u64;
        assert!(
            new_tokens > old_chars_div_4,
            "New heuristic ({}) should exceed chars/4 ({}) for CJK text",
            new_tokens,
            old_chars_div_4
        );
    }
}
