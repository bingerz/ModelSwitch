//! Content moderation guardrails for screening requests and responses.
//!
//! Provides configurable pattern-matching-based content screening. Blocked
//! patterns can be overridden by an allowlist, and requests can be rejected
//! for exceeding a configurable maximum length.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Configuration for content moderation guardrails.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuardrailsConfig {
    /// Enable request content screening.
    #[serde(default)]
    pub enabled: bool,
    /// List of blocked patterns (substring match, case-insensitive).
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
    /// List of allowed patterns that override blocked patterns (allowlist).
    #[serde(default)]
    pub allowed_patterns: Vec<String>,
    /// Maximum request size in characters (0 = no limit).
    #[serde(default)]
    pub max_request_chars: usize,
    /// Custom error message returned when content is blocked.
    #[serde(default = "default_block_message")]
    pub block_message: String,
}

fn default_block_message() -> String {
    "Request blocked by content moderation policy".to_string()
}

/// Result of a guardrails check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailAction {
    /// Content passed all checks.
    Allow,
    /// Content was blocked with a reason.
    Block(String),
}

/// Content moderation checker with runtime-mutable configuration.
///
/// Configuration is stored behind a `RwLock` so that [`GuardrailsChecker::update_config`]
/// can be invoked with only a shared reference, enabling hot-reload from a
/// separate task without requiring `&mut self`.
pub struct GuardrailsChecker {
    config: RwLock<GuardrailsConfig>,
}

impl GuardrailsChecker {
    /// Create a new checker from the given configuration.
    pub fn new(config: GuardrailsConfig) -> Self {
        Self {
            config: RwLock::new(config),
        }
    }

    /// Check if content should be allowed or blocked.
    ///
    /// Evaluation order:
    /// 1. If guardrails are disabled, [`GuardrailAction::Allow`] is returned.
    /// 2. If `max_request_chars > 0` and the content exceeds that length,
    ///    the request is blocked.
    /// 3. Each blocked pattern is matched as a case-insensitive substring.
    ///    On the first match, the allowlist is consulted — if any allowed
    ///    pattern also matches the content, the request is allowed.
    /// 4. Otherwise the matched blocked pattern is reported.
    pub fn check(&self, content: &str) -> GuardrailAction {
        let config = self.config.read().expect("guardrails config lock poisoned");

        if !config.enabled {
            return GuardrailAction::Allow;
        }

        if config.max_request_chars > 0 && content.chars().count() > config.max_request_chars {
            return GuardrailAction::Block("Request exceeds maximum length".to_string());
        }

        let content_lower = content.to_lowercase();
        for pattern in &config.blocked_patterns {
            let pattern_lower = pattern.to_lowercase();
            if content_lower.contains(&pattern_lower) {
                let is_allowed = config.allowed_patterns.iter().any(|allowed| {
                    let allowed_lower = allowed.to_lowercase();
                    content_lower.contains(&allowed_lower)
                });

                if is_allowed {
                    return GuardrailAction::Allow;
                }

                return GuardrailAction::Block(format!(
                    "Content matches blocked pattern: {}",
                    pattern
                ));
            }
        }

        GuardrailAction::Allow
    }

    /// Check a chat completion request body.
    ///
    /// Extracts text from the `messages` array (concatenating all `content`
    /// fields) and runs [`GuardrailsChecker::check`] on the combined text.
    /// Bodies without a `messages` array are treated as empty content.
    pub fn check_request(&self, body: &serde_json::Value) -> GuardrailAction {
        let mut combined = String::new();
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            for message in messages {
                if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                    combined.push_str(content);
                    combined.push('\n');
                }
            }
        }
        self.check(&combined)
    }

    /// Check a non-streaming chat completion response body.
    ///
    /// Extracts text from `choices[].message.content` (concatenating all
    /// choices) and runs [`GuardrailsChecker::check`] on the combined text.
    /// Bodies without a `choices` array are treated as empty content and
    /// allowed. Streaming SSE responses are not scanned here — each chunk
    /// is incomplete and cannot be matched reliably.
    pub fn check_response(&self, body: &serde_json::Value) -> GuardrailAction {
        let mut combined = String::new();
        if let Some(choices) = body.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(content) = choice
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    combined.push_str(content);
                    combined.push('\n');
                }
            }
        }
        self.check(&combined)
    }

    /// Return the current configuration.
    pub fn get_config(&self) -> GuardrailsConfig {
        self.config
            .read()
            .expect("guardrails config lock poisoned")
            .clone()
    }

    /// Update configuration at runtime.
    ///
    /// Uses interior mutability so the checker can be shared (e.g. behind an
    /// `Arc`) while still allowing hot-reload of the configuration.
    pub fn update_config(&self, config: GuardrailsConfig) {
        let mut guard = self
            .config
            .write()
            .expect("guardrails config lock poisoned");
        *guard = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allows_content_when_disabled() {
        let config = GuardrailsConfig {
            enabled: false,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);
        assert_eq!(
            checker.check("this contains forbidden text"),
            GuardrailAction::Allow
        );
    }

    #[test]
    fn blocks_blocked_pattern() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);
        let result = checker.check("this contains FORBIDDEN text");
        match result {
            GuardrailAction::Block(reason) => {
                assert!(
                    reason.contains("forbidden"),
                    "expected reason to mention the blocked pattern, got: {reason}"
                );
            }
            GuardrailAction::Allow => panic!("expected block, got allow"),
        }
    }

    #[test]
    fn allows_overridden_pattern() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["secret".to_string()],
            allowed_patterns: vec!["public-override".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);
        // Content matches both the blocked and allowed patterns → Allow.
        assert_eq!(
            checker.check("this is a public-override for secret data"),
            GuardrailAction::Allow
        );
    }

    #[test]
    fn blocks_oversized_content() {
        let config = GuardrailsConfig {
            enabled: true,
            max_request_chars: 10,
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);
        let result = checker.check("this content is way too long");
        match result {
            GuardrailAction::Block(reason) => {
                assert!(
                    reason.contains("maximum length"),
                    "expected reason to mention maximum length, got: {reason}"
                );
            }
            GuardrailAction::Allow => panic!("expected block, got allow"),
        }
    }

    #[test]
    fn check_request_extracts_messages() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);

        let body = json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Tell me about the forbidden topic."},
            ]
        });

        let result = checker.check_request(&body);
        match result {
            GuardrailAction::Block(reason) => {
                assert!(
                    reason.contains("forbidden"),
                    "expected reason to mention forbidden, got: {reason}"
                );
            }
            GuardrailAction::Allow => panic!("expected block, got allow"),
        }
    }

    #[test]
    fn check_response_extracts_choices_content() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);

        let body = json!({
            "choices": [
                {"message": {"role": "assistant", "content": "here is the forbidden answer"}},
            ]
        });

        let result = checker.check_response(&body);
        match result {
            GuardrailAction::Block(reason) => {
                assert!(
                    reason.contains("forbidden"),
                    "expected reason to mention forbidden, got: {reason}"
                );
            }
            GuardrailAction::Allow => panic!("expected block, got allow"),
        }
    }

    #[test]
    fn check_response_allows_clean_content() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);

        let body = json!({
            "choices": [
                {"message": {"role": "assistant", "content": "a perfectly safe reply"}},
            ]
        });

        assert_eq!(checker.check_response(&body), GuardrailAction::Allow);
    }

    #[test]
    fn check_response_allows_when_choices_missing() {
        let config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["forbidden".to_string()],
            ..Default::default()
        };
        let checker = GuardrailsChecker::new(config);

        // Bodies without a choices array are treated as empty content.
        assert_eq!(checker.check_response(&json!({})), GuardrailAction::Allow);
    }
}
