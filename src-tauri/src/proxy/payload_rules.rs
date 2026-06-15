use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Rules that modify outgoing request payloads per channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadRules {
    /// Default parameters to merge into the request if not present.
    #[serde(default)]
    pub defaults: HashMap<String, serde_json::Value>,
    /// Override parameters that always replace existing values.
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
    /// Parameter names to strip from the outgoing request.
    #[serde(default)]
    pub strip: Vec<String>,
}

impl PayloadRules {
    /// Apply rules to a request body, returning a modified copy.
    pub fn apply(&self, mut body: serde_json::Value) -> serde_json::Value {
        if let Some(map) = body.as_object_mut() {
            // 1. Apply defaults for missing fields
            for (key, val) in &self.defaults {
                if !map.contains_key(key) {
                    map.insert(key.clone(), val.clone());
                }
            }
            // 2. Apply overrides (always replace)
            for (key, val) in &self.overrides {
                map.insert(key.clone(), val.clone());
            }
            // 3. Strip specified fields
            for key in &self.strip {
                map.remove(key);
            }
        }
        body
    }
}

/// Per-channel payload rules registry.
pub struct ChannelPayloadRules {
    rules: std::sync::RwLock<HashMap<uuid::Uuid, PayloadRules>>,
}

impl Default for ChannelPayloadRules {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelPayloadRules {
    pub fn new() -> Self {
        Self {
            rules: std::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn add(&self, channel_id: uuid::Uuid, rules: PayloadRules) {
        self.rules.write().unwrap_or_else(|e| e.into_inner()).insert(channel_id, rules);
    }

    pub fn get(&self, channel_id: uuid::Uuid) -> Option<PayloadRules> {
        self.rules.read().unwrap_or_else(|e| e.into_inner()).get(&channel_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        // Override sets temperature to 0.5, strip should remove it
        // But strip is applied last, so it wins
        rules
            .overrides
            .insert("temperature".to_string(), json!(0.5));
        let body = json!({ "model": "gpt-4" });
        let result = rules.apply(body);
        assert!(result.get("temperature").is_none());
    }
}
