//! Struct definitions for payload rules.
//!
//! - `PayloadRules`: channel-level defaults/overrides/strip.
//! - `ModelPayloadRule`: per-model rule with optional model/protocol matching.

use super::path::{delete_path, path_exists, set_path};
use crate::channel::matches_glob;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Rules that modify outgoing request payloads per channel.
///
/// All keys in `defaults`, `overrides`, and `strip` use dotted JSON path
/// notation (e.g., `"generationConfig.thinkingConfig.thinkingBudget"`).
/// Simple top-level keys like `"temperature"` are still supported — they are
/// just single-segment paths.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PayloadRules {
    /// Default parameters to merge into the request if not present.
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    /// Override parameters that always replace existing values.
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
    /// Parameter paths to strip from the outgoing request.
    #[serde(default)]
    pub strip: Vec<String>,
}

impl PayloadRules {
    /// Apply rules to a request body, returning a modified copy.
    ///
    /// Order: defaults -> overrides -> strip. This matches the original
    /// semantics (strip wins over overrides which win over defaults).
    pub fn apply(&self, mut body: serde_json::Value) -> serde_json::Value {
        // Ensure we have an object root so set_path can work for top-level keys.
        if !body.is_object() {
            return body;
        }
        // 1. Defaults: only set if the path is absent.
        for (path, val) in &self.defaults {
            if !path_exists(&body, path) {
                set_path(&mut body, path, val.clone());
            }
        }
        // 2. Overrides: always replace.
        for (path, val) in &self.overrides {
            set_path(&mut body, path, val.clone());
        }
        // 3. Strip: remove.
        for path in &self.strip {
            delete_path(&mut body, path);
        }
        body
    }
}

/// A payload rule with optional model/protocol matching conditions.
///
/// When `models` is non-empty, the rule applies only to requests whose model
/// matches at least one glob pattern (e.g., `"gpt-*"`, `"claude-*"`, `"*"`).
/// When `protocol` is set, the rule additionally filters by provider protocol
/// (`"openai"`, `"anthropic"`, `"gemini"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPayloadRule {
    /// Model name patterns (supports `*` / `?` wildcards). Empty matches all.
    #[serde(default = "default_match_all")]
    pub models: Vec<String>,
    /// Protocol restriction: "openai", "anthropic", "gemini", or None for all.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Default params to merge (using JSON paths).
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    /// Override params that always replace (using JSON paths).
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
    /// Paths to strip.
    #[serde(default)]
    pub strip: Vec<String>,
}

fn default_match_all() -> Vec<String> {
    vec!["*".to_string()]
}

impl ModelPayloadRule {
    /// Returns true if this rule should apply to the given model + protocol.
    pub(crate) fn matches(&self, model: &str, protocol: &str) -> bool {
        if let Some(ref p) = self.protocol {
            if !p.is_empty() && p != protocol {
                return false;
            }
        }
        if self.models.is_empty() {
            return true;
        }
        self.models
            .iter()
            .any(|pattern| matches_glob(pattern, model))
    }

    /// Apply this rule's defaults/overrides/strip to the body.
    pub(crate) fn apply(&self, mut body: serde_json::Value) -> serde_json::Value {
        if !body.is_object() {
            return body;
        }
        for (path, val) in &self.defaults {
            if !path_exists(&body, path) {
                set_path(&mut body, path, val.clone());
            }
        }
        for (path, val) in &self.overrides {
            set_path(&mut body, path, val.clone());
        }
        for path in &self.strip {
            delete_path(&mut body, path);
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- PayloadRules --------------------------------------------------------

    #[test]
    fn apply_defaults() {
        let rules = PayloadRules {
            defaults: [("temperature".to_string(), json!(0.7))]
                .into_iter()
                .collect(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        let body = json!({ "model": "gpt-4" });
        let result = rules.apply(body);
        assert_eq!(result["temperature"], json!(0.7));
    }

    #[test]
    fn apply_defaults_does_not_overwrite() {
        let rules = PayloadRules {
            defaults: [("temperature".to_string(), json!(0.7))]
                .into_iter()
                .collect(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        let body = json!({ "model": "gpt-4", "temperature": 0.1 });
        let result = rules.apply(body);
        assert_eq!(result["temperature"], json!(0.1));
    }

    #[test]
    fn overrides_replace_existing() {
        let rules = PayloadRules {
            defaults: HashMap::new(),
            overrides: [("max_tokens".to_string(), json!(512))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let body = json!({ "model": "gpt-4", "max_tokens": 4096 });
        let result = rules.apply(body);
        assert_eq!(result["max_tokens"], json!(512));
    }

    #[test]
    fn strip_removes_fields() {
        let rules = PayloadRules {
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec!["top_p".to_string(), "frequency_penalty".to_string()],
        };
        let body = json!({ "model": "gpt-4", "top_p": 1, "frequency_penalty": 0 });
        let result = rules.apply(body);
        assert!(result.get("top_p").is_none());
        assert!(result.get("frequency_penalty").is_none());
    }

    #[test]
    fn apply_order_defaults_overrides_strip() {
        let mut rules = PayloadRules {
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec!["temperature".to_string()],
        };
        rules
            .overrides
            .insert("temperature".to_string(), json!(0.5));
        let body = json!({ "model": "gpt-4" });
        let result = rules.apply(body);
        assert!(result.get("temperature").is_none());
    }

    #[test]
    fn payload_rules_with_nested_paths() {
        let rules = PayloadRules {
            defaults: HashMap::new(),
            overrides: [(
                "generationConfig.thinkingConfig.thinkingBudget".to_string(),
                json!(4096),
            )]
            .into_iter()
            .collect(),
            strip: vec!["metadata.trace_id".to_string()],
        };
        let body = json!({
            "model": "gemini-2.5-flash",
            "metadata": { "trace_id": "abc", "keep": true }
        });
        let result = rules.apply(body);
        assert_eq!(
            result["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(4096)
        );
        assert!(result["metadata"].get("trace_id").is_none());
        assert_eq!(result["metadata"]["keep"], json!(true));
    }

    #[test]
    fn payload_rules_defaults_nested_path_only_when_absent() {
        let rules = PayloadRules {
            defaults: [("generationConfig.temperature".to_string(), json!(0.5))]
                .into_iter()
                .collect(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        // Absent -> applied.
        let body = json!({ "model": "gpt-4" });
        let result = rules.apply(body.clone());
        assert_eq!(result["generationConfig"]["temperature"], json!(0.5));

        // Present -> not overwritten.
        let body = json!({ "generationConfig": { "temperature": 0.9 } });
        let result = rules.apply(body);
        assert_eq!(result["generationConfig"]["temperature"], json!(0.9));
    }

    // -- ModelPayloadRule matching -------------------------------------------

    #[test]
    fn model_rule_glob_matches() {
        let rule = ModelPayloadRule {
            models: vec!["gpt-*".to_string()],
            protocol: None,
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        assert!(rule.matches("gpt-4o", "openai"));
        assert!(rule.matches("gpt-3.5-turbo", "openai"));
        assert!(!rule.matches("claude-3", "anthropic"));
    }

    #[test]
    fn model_rule_star_matches_all() {
        let rule = ModelPayloadRule {
            models: vec!["*".to_string()],
            protocol: None,
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        assert!(rule.matches("anything", "openai"));
        assert!(rule.matches("whatever", "anthropic"));
    }

    #[test]
    fn model_rule_empty_models_matches_all() {
        let rule = ModelPayloadRule {
            models: vec![],
            protocol: None,
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        assert!(rule.matches("anything", "openai"));
    }

    #[test]
    fn model_rule_protocol_filter() {
        let rule = ModelPayloadRule {
            models: vec!["*".to_string()],
            protocol: Some("anthropic".to_string()),
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        assert!(rule.matches("claude-3", "anthropic"));
        assert!(!rule.matches("claude-3", "openai"));
    }

    #[test]
    fn model_rule_protocol_empty_matches_all() {
        let rule = ModelPayloadRule {
            models: vec!["*".to_string()],
            protocol: Some("".to_string()),
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        assert!(rule.matches("gpt-4", "openai"));
        assert!(rule.matches("claude-3", "anthropic"));
    }

    // -- Config serialization (round-trip) -----------------------------------

    #[test]
    fn model_payload_rule_round_trip() {
        let rule = ModelPayloadRule {
            models: vec!["gpt-*".to_string(), "claude-*".to_string()],
            protocol: Some("openai".to_string()),
            defaults: [("temperature".to_string(), json!(0.5))]
                .into_iter()
                .collect(),
            overrides: [("reasoning.effort".to_string(), json!("high"))]
                .into_iter()
                .collect(),
            strip: vec!["top_p".to_string()],
        };
        let serialized = serde_json::to_string(&rule).unwrap();
        let deserialized: ModelPayloadRule = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.models, rule.models);
        assert_eq!(deserialized.protocol, rule.protocol);
        assert_eq!(deserialized.strip, rule.strip);
    }

    #[test]
    fn model_payload_rule_defaults_default_models_when_missing() {
        // When `models` is omitted, serde should default to ["*"].
        let json_str = r#"{"overrides":{"a":"b"}}"#;
        let rule: ModelPayloadRule = serde_json::from_str(json_str).unwrap();
        assert_eq!(rule.models, vec!["*".to_string()]);
    }
}
