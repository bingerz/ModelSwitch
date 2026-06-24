use crate::channel::matches_glob;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Get a value at a dotted JSON path (e.g., "generationConfig.thinkingConfig.thinkingBudget").
/// Returns `None` if any intermediate key doesn't exist or the path is invalid.
///
/// Array indices are supported: `"messages.0.content"` indexes into a JSON array.
pub(crate) fn get_path<'a>(
    body: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = body;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Set a value at a dotted JSON path, creating intermediate objects as needed.
///
/// - Simple paths like `"model"` set a top-level key.
/// - Nested paths like `"a.b.c"` create `a`, `b`, then set `c`.
/// - Array indices (e.g., `"messages.0.role"`) navigate into existing arrays.
///   When the current node is an array and the segment parses as a valid index
///   within the array bounds, we descend into it. We do not auto-extend arrays
///   (to avoid ambiguous placeholder semantics); missing/out-of-bounds indices
///   cause the set to be skipped.
/// - If an intermediate node exists but is a scalar (not object/array), the
///   path cannot be created and the set is skipped.
pub(crate) fn set_path(body: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return;
    }
    set_path_recursive(body, &segments, value);
}

fn set_path_recursive(body: &mut serde_json::Value, segments: &[&str], value: serde_json::Value) {
    if segments.is_empty() {
        return;
    }
    let first = segments[0];
    if first.is_empty() {
        return;
    }
    if segments.len() == 1 {
        // Terminal segment: set the value.
        match body {
            serde_json::Value::Object(map) => {
                map.insert(first.to_string(), value);
            }
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = first.parse::<usize>() {
                    if idx < arr.len() {
                        arr[idx] = value;
                    }
                    // Out-of-bounds: skip (we don't auto-extend arrays).
                }
                // Non-numeric index into array: skip.
            }
            _ => {
                // Scalar node — cannot descend. Caller's intent is to replace
                // at this path, but we can't because the parent isn't a container.
            }
        }
        return;
    }

    // Intermediate segment: ensure a container exists, then recurse.
    match body {
        serde_json::Value::Object(map) => {
            let entry = map
                .entry(first.to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            // If existing entry is a scalar/null, replace with an object so we
            // can descend. `null` is treated as missing.
            if !entry.is_object() && !entry.is_array() {
                *entry = serde_json::Value::Object(serde_json::Map::new());
            }
            set_path_recursive(entry, &segments[1..], value);
        }
        serde_json::Value::Array(arr) => {
            let Ok(idx) = first.parse::<usize>() else {
                return;
            };
            if idx >= arr.len() {
                return;
            }
            // If existing entry is a scalar/null, replace with object so we can descend.
            if !arr[idx].is_object() && !arr[idx].is_array() {
                arr[idx] = serde_json::Value::Object(serde_json::Map::new());
            }
            set_path_recursive(&mut arr[idx], &segments[1..], value);
        }
        _ => {}
    }
}

/// Delete a value at a dotted JSON path. No-op if the path doesn't exist.
pub(crate) fn delete_path(body: &mut serde_json::Value, path: &str) {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return;
    }
    delete_path_recursive(body, &segments);
}

fn delete_path_recursive(body: &mut serde_json::Value, segments: &[&str]) {
    if segments.is_empty() {
        return;
    }
    let first = segments[0];
    if first.is_empty() {
        return;
    }
    if segments.len() == 1 {
        match body {
            serde_json::Value::Object(map) => {
                map.remove(first);
            }
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = first.parse::<usize>() {
                    if idx < arr.len() {
                        arr.remove(idx);
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // Intermediate: descend.
    match body {
        serde_json::Value::Object(map) => {
            if let Some(child) = map.get_mut(first) {
                delete_path_recursive(child, &segments[1..]);
            }
        }
        serde_json::Value::Array(arr) => {
            if let Ok(idx) = first.parse::<usize>() {
                if let Some(child) = arr.get_mut(idx) {
                    delete_path_recursive(child, &segments[1..]);
                }
            }
        }
        _ => {}
    }
}

/// Check whether a path exists in the body (used for defaults logic).
fn path_exists(body: &serde_json::Value, path: &str) -> bool {
    get_path(body, path).is_some()
}

/// Rules that modify outgoing request payloads per channel.
///
/// All keys in `defaults`, `overrides`, and `strip` use dotted JSON path
/// notation (e.g., `"generationConfig.thinkingConfig.thinkingBudget"`).
/// Simple top-level keys like `"temperature"` are still supported — they are
/// just single-segment paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for PayloadRules {
    fn default() -> Self {
        Self {
            defaults: HashMap::new(),
            overrides: HashMap::new(),
            strip: Vec::new(),
        }
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
    fn matches(&self, model: &str, protocol: &str) -> bool {
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
    fn apply(&self, mut body: serde_json::Value) -> serde_json::Value {
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

    // -- get_path -------------------------------------------------------------

    #[test]
    fn get_path_top_level() {
        let body = json!({ "model": "gpt-4", "temperature": 0.7 });
        assert_eq!(get_path(&body, "model"), Some(&json!("gpt-4")));
        assert_eq!(get_path(&body, "temperature"), Some(&json!(0.7)));
    }

    #[test]
    fn get_path_nested() {
        let body = json!({
            "generationConfig": {
                "thinkingConfig": { "thinkingBudget": 8192 }
            }
        });
        assert_eq!(
            get_path(&body, "generationConfig.thinkingConfig.thinkingBudget"),
            Some(&json!(8192))
        );
    }

    #[test]
    fn get_path_missing_intermediate() {
        let body = json!({ "model": "gpt-4" });
        assert_eq!(
            get_path(&body, "generationConfig.thinkingConfig.thinkingBudget"),
            None
        );
    }

    #[test]
    fn get_path_array_index() {
        let body = json!({
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hi" }
            ]
        });
        assert_eq!(get_path(&body, "messages.0.role"), Some(&json!("system")));
        assert_eq!(get_path(&body, "messages.1.content"), Some(&json!("Hi")));
        assert_eq!(get_path(&body, "messages.5.role"), None);
    }

    #[test]
    fn get_path_empty_segment() {
        let body = json!({ "model": "gpt-4" });
        assert_eq!(get_path(&body, "model."), None);
        assert_eq!(get_path(&body, ".model"), None);
    }

    #[test]
    fn get_path_on_scalar() {
        let body = json!("just a string");
        assert_eq!(get_path(&body, "anything"), None);
    }

    // -- set_path -------------------------------------------------------------

    #[test]
    fn set_path_top_level() {
        let mut body = json!({ "model": "gpt-4" });
        set_path(&mut body, "temperature", json!(0.7));
        assert_eq!(body["temperature"], json!(0.7));
    }

    #[test]
    fn set_path_creates_intermediate_objects() {
        let mut body = json!({ "model": "gpt-4" });
        set_path(
            &mut body,
            "generationConfig.thinkingConfig.thinkingBudget",
            json!(8192),
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(8192)
        );
    }

    #[test]
    fn set_path_overwrites_existing_nested() {
        let mut body = json!({
            "generationConfig": { "thinkingConfig": { "thinkingBudget": 1024 } }
        });
        set_path(
            &mut body,
            "generationConfig.thinkingConfig.thinkingBudget",
            json!(4096),
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(4096)
        );
    }

    #[test]
    fn set_path_replaces_scalar_intermediate() {
        let mut body = json!({ "generationConfig": "oops" });
        set_path(
            &mut body,
            "generationConfig.thinkingConfig.thinkingBudget",
            json!(8192),
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(8192)
        );
    }

    #[test]
    fn set_path_array_index() {
        let mut body = json!({
            "messages": [
                { "role": "system" },
                { "role": "user" }
            ]
        });
        set_path(&mut body, "messages.0.role", json!("developer"));
        assert_eq!(body["messages"][0]["role"], json!("developer"));
    }

    #[test]
    fn set_path_empty_segment_no_op() {
        let mut body = json!({ "model": "gpt-4" });
        set_path(&mut body, "", json!(42));
        assert_eq!(body, json!({ "model": "gpt-4" }));
    }

    // -- delete_path ----------------------------------------------------------

    #[test]
    fn delete_path_top_level() {
        let mut body = json!({ "model": "gpt-4", "top_p": 1 });
        delete_path(&mut body, "top_p");
        assert!(body.get("top_p").is_none());
    }

    #[test]
    fn delete_path_nested() {
        let mut body = json!({
            "generationConfig": { "thinkingConfig": { "thinkingBudget": 8192, "enabled": true } }
        });
        delete_path(&mut body, "generationConfig.thinkingConfig.thinkingBudget");
        assert!(body["generationConfig"]["thinkingConfig"]
            .get("thinkingBudget")
            .is_none());
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["enabled"],
            json!(true)
        );
    }

    #[test]
    fn delete_path_missing_no_error() {
        let mut body = json!({ "model": "gpt-4" });
        delete_path(&mut body, "generationConfig.thinkingConfig.thinkingBudget");
        assert_eq!(body, json!({ "model": "gpt-4" }));
    }

    #[test]
    fn delete_path_array_index() {
        let mut body = json!({ "items": [1, 2, 3] });
        delete_path(&mut body, "items.1");
        assert_eq!(body["items"], json!([1, 3]));
    }

    // -- PayloadRules backward compatibility ----------------------------------

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

    // -- ModelPayloadRule matching --------------------------------------------

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

    // -- ChannelPayloadRules.apply_for_model ----------------------------------

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

    // -- Config serialization (round-trip) ------------------------------------

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
