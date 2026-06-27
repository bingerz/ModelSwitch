//! Thread-safe per-channel payload rules registry.

use super::types::{ModelPayloadRule, PayloadRules};
use parking_lot::RwLock;
use std::collections::HashMap;

/// Per-channel payload rules registry. Stores both channel-level rules
/// (applied to every request on the channel) and optional per-model rules
/// (applied only when model + protocol match).
pub struct ChannelPayloadRules {
    rules: RwLock<HashMap<uuid::Uuid, ChannelRules>>,
}

/// Combined channel-level + per-model rules for a single channel.
#[derive(Debug, Clone, Default)]
pub struct ChannelRules {
    /// Channel-level rules applied to every request.
    pub channel: PayloadRules,
    /// Model-specific rules applied after channel-level rules.
    pub model_rules: Vec<ModelPayloadRule>,
}

impl Default for ChannelPayloadRules {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelPayloadRules {
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(HashMap::new()),
        }
    }

    /// Replace the full rule set for a channel (channel-level + per-model).
    pub fn set(&self, channel_id: uuid::Uuid, rules: ChannelRules) {
        self.rules.write().insert(channel_id, rules);
    }

    /// Add or replace only the channel-level rules. Preserves any existing
    /// per-model rules.
    pub fn add(&self, channel_id: uuid::Uuid, rules: PayloadRules) {
        let mut guard = self.rules.write();
        let entry = guard.entry(channel_id).or_default();
        entry.channel = rules;
    }

    /// Add or replace the per-model rules for a channel. Preserves any
    /// existing channel-level rules.
    pub fn set_model_rules(&self, channel_id: uuid::Uuid, rules: Vec<ModelPayloadRule>) {
        let mut guard = self.rules.write();
        let entry = guard.entry(channel_id).or_default();
        entry.model_rules = rules;
    }

    /// Get a clone of the channel-level rules (backward-compatible getter).
    pub fn get(&self, channel_id: uuid::Uuid) -> Option<PayloadRules> {
        self.rules
            .read()
            .get(&channel_id)
            .map(|r| r.channel.clone())
    }

    /// Get a clone of the full rule set (channel + per-model).
    pub fn get_full(&self, channel_id: uuid::Uuid) -> Option<ChannelRules> {
        self.rules.read().get(&channel_id).cloned()
    }

    /// Check if any rules exist for a given channel (without cloning).
    pub fn has_rules(&self, channel_id: uuid::Uuid) -> bool {
        self.rules.read().contains_key(&channel_id)
    }

    /// Remove rules for a channel that has been deleted.
    pub fn remove(&self, channel_id: uuid::Uuid) {
        self.rules.write().remove(&channel_id);
    }

    /// Apply rules with model and protocol context.
    ///
    /// First applies channel-level rules, then applies each matching
    /// model-specific rule in declaration order.
    pub fn apply_for_model(
        &self,
        channel_id: uuid::Uuid,
        mut body: serde_json::Value,
        model: &str,
        protocol: &str,
    ) -> serde_json::Value {
        let snapshot = self.rules.read().get(&channel_id).cloned();
        let Some(rules) = snapshot else {
            return body;
        };
        body = rules.channel.apply(body);
        for mr in &rules.model_rules {
            if mr.matches(model, protocol) {
                body = mr.apply(body);
            }
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn channel_with_rules(
        channel: PayloadRules,
        model_rules: Vec<ModelPayloadRule>,
    ) -> ChannelPayloadRules {
        let cpr = ChannelPayloadRules::new();
        let id = uuid::Uuid::nil();
        cpr.set(
            id,
            ChannelRules {
                channel,
                model_rules,
            },
        );
        cpr
    }

    // -- apply_for_model -----------------------------------------------------

    #[test]
    fn apply_for_model_channel_level_only() {
        let channel = PayloadRules {
            defaults: [("temperature".to_string(), json!(0.7))]
                .into_iter()
                .collect(),
            overrides: HashMap::new(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![]);
        let body = json!({ "model": "gpt-4" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4", "openai");
        assert_eq!(result["temperature"], json!(0.7));
    }

    #[test]
    fn apply_for_model_with_matching_model_rule() {
        let channel = PayloadRules::default();
        let model_rule = ModelPayloadRule {
            models: vec!["gpt-*".to_string()],
            protocol: Some("openai".to_string()),
            defaults: HashMap::new(),
            overrides: [("reasoning.effort".to_string(), json!("high"))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![model_rule]);
        let body = json!({ "model": "gpt-4o" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4o", "openai");
        assert_eq!(result["reasoning"]["effort"], json!("high"));
    }

    #[test]
    fn apply_for_model_skips_non_matching_model_rule() {
        let channel = PayloadRules::default();
        let model_rule = ModelPayloadRule {
            models: vec!["claude-*".to_string()],
            protocol: None,
            defaults: HashMap::new(),
            overrides: [("thinking.budget_tokens".to_string(), json!(8192))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![model_rule]);
        let body = json!({ "model": "gpt-4o" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4o", "openai");
        assert!(result.get("thinking").is_none());
    }

    #[test]
    fn apply_for_model_skips_non_matching_protocol() {
        let channel = PayloadRules::default();
        let model_rule = ModelPayloadRule {
            models: vec!["*".to_string()],
            protocol: Some("anthropic".to_string()),
            defaults: HashMap::new(),
            overrides: [("thinking.budget_tokens".to_string(), json!(8192))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![model_rule]);
        let body = json!({ "model": "gpt-4o" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4o", "openai");
        assert!(result.get("thinking").is_none());
    }

    #[test]
    fn apply_for_model_combined_channel_and_model_rules() {
        let channel = PayloadRules {
            defaults: [("temperature".to_string(), json!(0.7))]
                .into_iter()
                .collect(),
            overrides: [("max_tokens".to_string(), json!(4096))]
                .into_iter()
                .collect(),
            strip: vec!["top_p".to_string()],
        };
        let model_rule = ModelPayloadRule {
            models: vec!["gpt-*".to_string()],
            protocol: Some("openai".to_string()),
            defaults: HashMap::new(),
            overrides: [("reasoning.effort".to_string(), json!("high"))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![model_rule]);
        let body = json!({
            "model": "gpt-4o",
            "top_p": 0.9
        });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4o", "openai");
        // Channel-level applied.
        assert_eq!(result["temperature"], json!(0.7));
        assert_eq!(result["max_tokens"], json!(4096));
        assert!(result.get("top_p").is_none());
        // Model-level applied on top.
        assert_eq!(result["reasoning"]["effort"], json!("high"));
    }

    #[test]
    fn apply_for_model_multiple_model_rules_first_match_wins_priority() {
        // When two model rules both match, both are applied in declaration order.
        let channel = PayloadRules::default();
        let r1 = ModelPayloadRule {
            models: vec!["gpt-*".to_string()],
            protocol: None,
            defaults: HashMap::new(),
            overrides: [("temperature".to_string(), json!(0.3))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let r2 = ModelPayloadRule {
            models: vec!["*".to_string()],
            protocol: None,
            defaults: HashMap::new(),
            overrides: [("temperature".to_string(), json!(0.9))]
                .into_iter()
                .collect(),
            strip: vec![],
        };
        let cpr = channel_with_rules(channel, vec![r1, r2]);
        let body = json!({ "model": "gpt-4" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4", "openai");
        // Last applied wins for overrides.
        assert_eq!(result["temperature"], json!(0.9));
    }

    #[test]
    fn apply_for_model_no_rules_returns_body_unchanged() {
        let cpr = ChannelPayloadRules::new();
        let body = json!({ "model": "gpt-4" });
        let result = cpr.apply_for_model(uuid::Uuid::nil(), body, "gpt-4", "openai");
        assert_eq!(result, json!({ "model": "gpt-4" }));
    }
}
