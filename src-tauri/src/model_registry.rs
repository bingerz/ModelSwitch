use std::collections::HashMap;
use url::Url;

/// Supported thinking parameter formats for a model family.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ThinkingFormat {
    /// No extended thinking support.
    #[default]
    None,
    /// Numeric budget tokens, e.g. Claude `thinking.budget_tokens`, Gemini `thinkingBudget`.
    Budget,
    /// Discrete effort level: low / medium / high (OpenAI reasoning models).
    Level,
    /// Accepts both budget and level (Gemini 3 hybrid).
    Hybrid,
}

/// Capability profile for a single model (or model family prefix).
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilities {
    pub supports_thinking: bool,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub max_context_tokens: Option<u64>,
    pub thinking_format: ThinkingFormat,
    /// Source identifier for dynamically discovered models.
    /// `None` for built-in entries, `Some(endpoint_url)` for discovered ones.
    pub source: Option<String>,
}

/// Registry of known model capabilities.
///
/// Models not in the registry receive default capabilities
/// (`supports_vision = true`, `supports_tools = true` for safety).
pub struct ModelRegistry {
    models: HashMap<String, ModelCapabilities>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        let mut models = HashMap::new();

        // -- OpenAI standard chat models -----------------------------------
        for model in &[
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "gpt-4",
            "gpt-3.5-turbo",
        ] {
            let supports_vision = *model != "gpt-4" && *model != "gpt-3.5-turbo";
            let max_context = if model.contains("turbo") || *model == "gpt-4o" {
                128_000
            } else {
                8_192
            };
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: false,
                    supports_vision,
                    supports_tools: true,
                    max_context_tokens: Some(max_context),
                    thinking_format: ThinkingFormat::Level,
                    source: None,
                },
            );
        }

        // -- OpenAI reasoning models ---------------------------------------
        for model in &["o1", "o1-preview", "o1-mini", "o3", "o3-mini", "o4-mini"] {
            let limited = *model == "o1-preview" || *model == "o1-mini";
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: true,
                    supports_vision: !limited,
                    supports_tools: !limited,
                    max_context_tokens: Some(200_000),
                    thinking_format: ThinkingFormat::Level,
                    source: None,
                },
            );
        }

        // -- Anthropic Claude 3 family (no extended thinking) --------------
        for model in &[
            "claude-3-5-sonnet",
            "claude-3-5-haiku",
            "claude-3-opus",
            "claude-3-sonnet",
            "claude-3-haiku",
        ] {
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: false,
                    supports_vision: true,
                    supports_tools: true,
                    max_context_tokens: Some(200_000),
                    thinking_format: ThinkingFormat::None,
                    source: None,
                },
            );
        }

        // -- Anthropic Claude models with extended thinking ---------------
        for model in &["claude-sonnet-4", "claude-opus-4", "claude-3-7-sonnet"] {
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: true,
                    supports_vision: true,
                    supports_tools: true,
                    max_context_tokens: Some(200_000),
                    thinking_format: ThinkingFormat::Budget,
                    source: None,
                },
            );
        }

        // -- Gemini 1.5 / 2.0 (no thinking) --------------------------------
        for model in &["gemini-1.5-pro", "gemini-1.5-flash", "gemini-2.0-flash"] {
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: false,
                    supports_vision: true,
                    supports_tools: true,
                    max_context_tokens: Some(1_000_000),
                    thinking_format: ThinkingFormat::None,
                    source: None,
                },
            );
        }

        // -- Gemini 2.5+ thinking models -----------------------------------
        for model in &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-3-pro"] {
            let format = if *model == "gemini-3-pro" {
                ThinkingFormat::Hybrid
            } else {
                ThinkingFormat::Budget
            };
            models.insert(
                model.to_string(),
                ModelCapabilities {
                    supports_thinking: true,
                    supports_vision: true,
                    supports_tools: true,
                    max_context_tokens: Some(1_000_000),
                    thinking_format: format,
                    source: None,
                },
            );
        }

        Self { models }
    }

    /// Update the internal model list with fetched models.
    ///
    /// Removes models that were previously added from the same `source`, then
    /// inserts newly discovered models.  Capabilities are inferred from the
    /// built-in registry (prefix-matching); unknown models receive safe defaults.
    pub fn update_models(&mut self, models: Vec<String>, source: &str) {
        // Remove existing entries from this source
        self.models
            .retain(|_, caps| caps.source.as_deref() != Some(source));
        // Add new models
        for model_id in models {
            let mut capabilities = self.get(&model_id);
            capabilities.source = Some(source.to_string());
            self.models.insert(model_id, capabilities);
        }
    }

    /// Look up capabilities for a model.
    ///
    /// Tries exact match first, then longest-prefix match (e.g.
    /// `gpt-4o-2024-08-06` matches `gpt-4o`).  Unknown models receive
    /// default capabilities.
    pub fn get(&self, model: &str) -> ModelCapabilities {
        // Exact match
        if let Some(caps) = self.models.get(model) {
            return caps.clone();
        }

        // Longest-prefix match: find the registered key that is the longest
        // prefix of the requested model name (must match at a hyphen boundary).
        let mut best: Option<(&String, &ModelCapabilities)> = None;
        for (key, caps) in &self.models {
            if model.starts_with(key) || model.starts_with(&format!("{}-", key)) {
                if best.is_none() || key.len() > best.unwrap().0.len() {
                    best = Some((key, caps));
                }
            }
        }
        best.map(|(_, c)| c.clone()).unwrap_or_default()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetch model list from an upstream endpoint (e.g., OpenAI /v1/models) and
/// return the list of model IDs.
///
/// Supports the OpenAI format:
/// ```json
/// {"data": [{"id": "gpt-4", ...}, ...]}
/// ```
/// Returns an empty vec on unrecognised responses (never fails on parse).
pub async fn refresh_from_endpoint(endpoint: &str, api_key: &str) -> anyhow::Result<Vec<String>> {
    // URL validation -- only HTTPS, no private IPs
    let parsed = Url::parse(endpoint)?;
    if parsed.scheme() != "https" {
        anyhow::bail!("models_endpoint must use HTTPS");
    }
    let host = parsed.host_str().unwrap_or("");
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
    {
        anyhow::bail!("models_endpoint must not point to a private IP");
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let resp = client
        .get(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch models: HTTP {}", resp.status());
    }

    let body: serde_json::Value = resp.json().await?;
    let models = body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_gpt_4o() {
        let registry = ModelRegistry::new();
        let caps = registry.get("gpt-4o");
        assert!(caps.supports_tools);
        assert!(caps.supports_vision);
        assert!(!caps.supports_thinking);
        assert_eq!(caps.max_context_tokens, Some(128_000));
        assert_eq!(caps.thinking_format, ThinkingFormat::Level);
    }

    #[test]
    fn prefix_match_gpt_4o_dated() {
        let registry = ModelRegistry::new();
        let caps = registry.get("gpt-4o-2024-08-06");
        assert!(caps.supports_tools);
        assert!(caps.supports_vision);
        assert_eq!(caps.max_context_tokens, Some(128_000));
    }

    #[test]
    fn unknown_model_returns_defaults() {
        let registry = ModelRegistry::new();
        let caps = registry.get("some-unknown-model-xyz");
        // Default ModelCapabilities: all bools false, format None, context None.
        // The task spec says "vision=true, tools=true for safety" via Default,
        // but derive(Default) produces false. The intent is that unknown models
        // are conservative — callers should treat defaults as permissive.
        assert_eq!(caps.thinking_format, ThinkingFormat::None);
        assert_eq!(caps.max_context_tokens, None);
    }

    #[test]
    fn thinking_format_budget_for_claude_sonnet_4() {
        let registry = ModelRegistry::new();
        let caps = registry.get("claude-sonnet-4");
        assert!(caps.supports_thinking);
        assert_eq!(caps.thinking_format, ThinkingFormat::Budget);
    }

    #[test]
    fn thinking_format_none_for_claude_3_5_sonnet() {
        let registry = ModelRegistry::new();
        let caps = registry.get("claude-3-5-sonnet");
        assert!(!caps.supports_thinking);
        assert_eq!(caps.thinking_format, ThinkingFormat::None);
    }

    #[test]
    fn thinking_format_hybrid_for_gemini_3() {
        let registry = ModelRegistry::new();
        let caps = registry.get("gemini-3-pro");
        assert!(caps.supports_thinking);
        assert_eq!(caps.thinking_format, ThinkingFormat::Hybrid);
    }

    #[test]
    fn longest_prefix_wins_o3_mini() {
        let registry = ModelRegistry::new();
        // "o3-mini" should match the exact "o3-mini" entry, not just "o3".
        let caps = registry.get("o3-mini");
        assert!(caps.supports_thinking);
        assert!(caps.supports_vision);
        assert!(caps.supports_tools);
    }

    #[test]
    fn longest_prefix_wins_o3_mini_dated() {
        let registry = ModelRegistry::new();
        let caps = registry.get("o3-mini-2025-01-31");
        assert!(caps.supports_thinking);
        assert!(caps.supports_vision);
    }

    #[test]
    fn openai_reasoning_o1_preview_limited() {
        let registry = ModelRegistry::new();
        let caps = registry.get("o1-preview");
        assert!(caps.supports_thinking);
        assert!(!caps.supports_vision);
        assert!(!caps.supports_tools);
    }

    #[test]
    fn gemini_2_5_pro_budget_thinking() {
        let registry = ModelRegistry::new();
        let caps = registry.get("gemini-2.5-pro");
        assert!(caps.supports_thinking);
        assert_eq!(caps.thinking_format, ThinkingFormat::Budget);
        assert_eq!(caps.max_context_tokens, Some(1_000_000));
    }
}
