use serde_json::Value;

/// Default budget used when Claude `thinking` is enabled without an explicit budget.
const DEFAULT_ENABLED_BUDGET: u64 = 8192;

/// Thinking budget to level conversion.
///
/// Maps a numeric token budget to the closest discrete reasoning level used by
/// OpenAI-compatible providers (`reasoning_effort`).
///
/// | Budget     | Level   |
/// |------------|---------|
/// | 0          | "none"  |
/// | 1-1024     | "low"   |
/// | 1025-8192  | "medium"|
/// | 8193+      | "high"  |
pub fn budget_to_level(budget: u64) -> &'static str {
    match budget {
        0 => "none",
        1..=1024 => "low",
        1025..=8192 => "medium",
        _ => "high",
    }
}

/// Map a reasoning level string to a representative token budget.
///
/// Returns `None` for unrecognized levels. Matching is case-insensitive.
///
/// | Level     | Budget  |
/// |-----------|---------|
/// | "none"    | 0       |
/// | "minimal" | 512     |
/// | "low"     | 1024    |
/// | "medium"  | 8192    |
/// | "high"    | 24576   |
/// | "xhigh"   | 32768   |
/// | "max"     | 128000  |
pub fn level_to_budget(level: &str) -> Option<u64> {
    match level.to_ascii_lowercase().as_str() {
        "none" => Some(0),
        "minimal" => Some(512),
        "low" => Some(1024),
        "medium" => Some(8192),
        "high" => Some(24576),
        "xhigh" => Some(32768),
        "max" => Some(128000),
        _ => None,
    }
}

/// Detect and extract a thinking budget from any supported request format.
///
/// Checks, in order:
/// 1. Claude `thinking.budget_tokens` (u64)
/// 2. Claude `thinking.type == "enabled"` (default budget when no explicit amount)
/// 3. OpenAI `reasoning_effort` (string) → `level_to_budget`
/// 4. OpenAI responses `reasoning.effort` (string) → `level_to_budget`
/// 5. Gemini `generationConfig.thinkingConfig.thinkingBudget` (u64)
///
/// Returns `Some(budget)` if a thinking parameter is found, `None` otherwise.
pub fn extract_thinking_budget(body: &Value) -> Option<u64> {
    // 1. Claude: thinking.budget_tokens
    if let Some(budget) = body
        .get("thinking")
        .and_then(|t| t.get("budget_tokens"))
        .and_then(|b| b.as_u64())
    {
        return Some(budget);
    }

    // 2. Claude: thinking.type == "enabled" (no explicit budget)
    if let Some(thinking_type) = body
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(|ty| ty.as_str())
    {
        if thinking_type == "enabled" {
            return Some(DEFAULT_ENABLED_BUDGET);
        }
    }

    // 3. OpenAI chat: reasoning_effort (string level)
    if let Some(effort) = body.get("reasoning_effort").and_then(|r| r.as_str()) {
        if let Some(budget) = level_to_budget(effort) {
            return Some(budget);
        }
    }

    // 4. OpenAI responses: reasoning.effort (string level)
    if let Some(effort) = body
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(|e| e.as_str())
    {
        if let Some(budget) = level_to_budget(effort) {
            return Some(budget);
        }
    }

    // 5. Gemini: generationConfig.thinkingConfig.thinkingBudget
    if let Some(budget) = body
        .get("generationConfig")
        .and_then(|gc| gc.get("thinkingConfig"))
        .and_then(|tc| tc.get("thinkingBudget"))
        .and_then(|tb| tb.as_u64())
    {
        return Some(budget);
    }

    None
}

/// Remove all known thinking-related fields from the request body.
///
/// Strips: `thinking` (Claude), `reasoning_effort` (OpenAI chat),
/// `reasoning` (OpenAI responses), and `generationConfig.thinkingConfig` (Gemini).
fn remove_thinking_fields(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("thinking");
        map.remove("reasoning_effort");
        map.remove("reasoning");
    }
    // Remove nested Gemini thinkingConfig while preserving other generationConfig keys.
    if let Some(gen_config) = body
        .get_mut("generationConfig")
        .and_then(|gc| gc.as_object_mut())
    {
        gen_config.remove("thinkingConfig");
    }
}

/// Normalize thinking parameters for a target provider format.
///
/// 1. Extracts the thinking budget from whichever source format the body uses.
/// 2. Removes all source-format thinking fields.
/// 3. If the budget is `None` or `0`, no thinking parameters are injected.
/// 4. Otherwise injects the target provider's native thinking field.
///
/// `target_provider` is matched case-insensitively and must be one of
/// `"openai"`, `"anthropic"`, or `"gemini"`. Unknown providers leave the body
/// cleaned of source thinking fields but inject nothing.
pub fn normalize_for_provider(body: &mut Value, target_provider: &str) {
    let budget = extract_thinking_budget(body);

    // Always strip source-format thinking fields.
    remove_thinking_fields(body);

    // Skip injection when no thinking was requested or thinking is disabled (0).
    let budget = match budget {
        Some(b) if b > 0 => b,
        _ => return,
    };

    match target_provider.to_ascii_lowercase().as_str() {
        "anthropic" => {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": budget
            });
        }
        "openai" => {
            body["reasoning_effort"] = Value::String(budget_to_level(budget).to_string());
        }
        "gemini" => {
            // Ensure generationConfig exists as an object.
            if body.get("generationConfig").is_none() {
                body["generationConfig"] = Value::Object(serde_json::Map::new());
            }
            body["generationConfig"]["thinkingConfig"] =
                serde_json::json!({ "thinkingBudget": budget });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── budget_to_level ──────────────────────────────────────────────────────

    #[test]
    fn budget_to_level_boundaries() {
        assert_eq!(budget_to_level(0), "none");
        assert_eq!(budget_to_level(1), "low");
        assert_eq!(budget_to_level(1024), "low");
        assert_eq!(budget_to_level(1025), "medium");
        assert_eq!(budget_to_level(8192), "medium");
        assert_eq!(budget_to_level(8193), "high");
        assert_eq!(budget_to_level(100000), "high");
    }

    // ── level_to_budget ──────────────────────────────────────────────────────

    #[test]
    fn level_to_budget_all_levels() {
        assert_eq!(level_to_budget("none"), Some(0));
        assert_eq!(level_to_budget("minimal"), Some(512));
        assert_eq!(level_to_budget("low"), Some(1024));
        assert_eq!(level_to_budget("medium"), Some(8192));
        assert_eq!(level_to_budget("high"), Some(24576));
        assert_eq!(level_to_budget("xhigh"), Some(32768));
        assert_eq!(level_to_budget("max"), Some(128000));
    }

    #[test]
    fn level_to_budget_case_insensitive() {
        assert_eq!(level_to_budget("LOW"), Some(1024));
        assert_eq!(level_to_budget("Medium"), Some(8192));
        assert_eq!(level_to_budget("HIGH"), Some(24576));
    }

    #[test]
    fn level_to_budget_unknown_returns_none() {
        assert_eq!(level_to_budget("ultra"), None);
        assert_eq!(level_to_budget(""), None);
    }

    // ── extract_thinking_budget ──────────────────────────────────────────────

    #[test]
    fn extract_from_claude_with_budget_tokens() {
        let body = json!({
            "thinking": { "type": "enabled", "budget_tokens": 16384 }
        });
        assert_eq!(extract_thinking_budget(&body), Some(16384));
    }

    #[test]
    fn extract_from_claude_type_enabled_no_budget() {
        let body = json!({
            "thinking": { "type": "enabled" }
        });
        assert_eq!(extract_thinking_budget(&body), Some(DEFAULT_ENABLED_BUDGET));
    }

    #[test]
    fn extract_from_openai_reasoning_effort() {
        let body = json!({ "reasoning_effort": "high" });
        assert_eq!(extract_thinking_budget(&body), Some(24576));
    }

    #[test]
    fn extract_from_openai_responses_reasoning_effort() {
        let body = json!({
            "reasoning": { "effort": "low" }
        });
        assert_eq!(extract_thinking_budget(&body), Some(1024));
    }

    #[test]
    fn extract_from_gemini_thinking_config() {
        let body = json!({
            "generationConfig": {
                "thinkingConfig": { "thinkingBudget": 4096 }
            }
        });
        assert_eq!(extract_thinking_budget(&body), Some(4096));
    }

    #[test]
    fn extract_returns_none_when_no_thinking_present() {
        let body = json!({
            "model": "gpt-4",
            "messages": [{ "role": "user", "content": "hi" }]
        });
        assert_eq!(extract_thinking_budget(&body), None);
    }

    // ── normalize_for_provider ───────────────────────────────────────────────

    #[test]
    fn normalize_claude_to_openai() {
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{ "role": "user", "content": "hi" }],
            "thinking": { "type": "enabled", "budget_tokens": 24576 }
        });
        normalize_for_provider(&mut body, "openai");
        assert!(body.get("thinking").is_none());
        assert_eq!(body["reasoning_effort"], "high");
        // Other fields preserved.
        assert_eq!(body["model"], "gpt-4");
    }

    #[test]
    fn normalize_openai_to_claude() {
        let mut body = json!({
            "model": "claude-3-sonnet",
            "messages": [{ "role": "user", "content": "hi" }],
            "reasoning_effort": "medium"
        });
        normalize_for_provider(&mut body, "anthropic");
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn normalize_openai_to_gemini() {
        let mut body = json!({
            "model": "gemini-pro",
            "reasoning_effort": "low"
        });
        normalize_for_provider(&mut body, "gemini");
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            1024
        );
    }

    #[test]
    fn normalize_gemini_to_openai() {
        let mut body = json!({
            "model": "gpt-4",
            "generationConfig": {
                "thinkingConfig": { "thinkingBudget": 8192 },
                "temperature": 0.7
            }
        });
        normalize_for_provider(&mut body, "openai");
        // thinkingConfig removed but other generationConfig keys preserved.
        assert!(body["generationConfig"].get("thinkingConfig").is_none());
        assert_eq!(body["generationConfig"]["temperature"], 0.7);
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn normalize_with_budget_zero_removes_all_and_injects_nothing() {
        let mut body = json!({
            "model": "gpt-4",
            "thinking": { "type": "enabled", "budget_tokens": 0 }
        });
        normalize_for_provider(&mut body, "openai");
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn normalize_with_no_thinking_preserves_body() {
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{ "role": "user", "content": "hi" }],
            "temperature": 0.5
        });
        normalize_for_provider(&mut body, "openai");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
        assert_eq!(body["model"], "gpt-4");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn normalize_preserves_other_fields() {
        let mut body = json!({
            "model": "claude-3",
            "messages": [{ "role": "user", "content": "hello" }],
            "max_tokens": 1024,
            "temperature": 0.7,
            "top_p": 0.9,
            "stream": true,
            "reasoning_effort": "high"
        });
        normalize_for_provider(&mut body, "anthropic");
        assert_eq!(body["model"], "claude-3");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["stream"], true);
        assert_eq!(body["thinking"]["budget_tokens"], 24576);
    }

    #[test]
    fn normalize_claude_to_gemini() {
        let mut body = json!({
            "thinking": { "type": "enabled", "budget_tokens": 4096 }
        });
        normalize_for_provider(&mut body, "gemini");
        assert!(body.get("thinking").is_none());
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            4096
        );
    }

    #[test]
    fn normalize_openai_responses_to_claude() {
        let mut body = json!({
            "reasoning": { "effort": "high" }
        });
        normalize_for_provider(&mut body, "anthropic");
        assert!(body.get("reasoning").is_none());
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 24576);
    }

    #[test]
    fn normalize_unknown_provider_cleans_but_does_not_inject() {
        let mut body = json!({
            "reasoning_effort": "high"
        });
        normalize_for_provider(&mut body, "mistral");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn normalize_provider_case_insensitive() {
        let mut body = json!({
            "reasoning_effort": "high"
        });
        normalize_for_provider(&mut body, "Anthropic");
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 24576);
    }
}
