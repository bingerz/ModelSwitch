use std::collections::HashMap;

/// Resolve the fallback chain for a given model.
/// Returns a list starting with the original model, followed by fallback models.
/// If no fallback is configured, returns a single-element vec with the original model.
pub fn resolve_fallback_chain(
    model: &str,
    fallbacks: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut chain = vec![model.to_string()];
    if let Some(fallbacks) = fallbacks.get(model) {
        for fb in fallbacks {
            if !chain.contains(fb) {
                chain.push(fb.clone());
            }
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fallback_configured() {
        let fallbacks = HashMap::new();
        let chain = resolve_fallback_chain("gpt-4o", &fallbacks);
        assert_eq!(chain, vec!["gpt-4o"]);
    }

    #[test]
    fn with_fallback_chain() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert(
            "claude-4-opus".to_string(),
            vec!["claude-4-sonnet".to_string(), "gpt-4o".to_string()],
        );
        let chain = resolve_fallback_chain("claude-4-opus", &fallbacks);
        assert_eq!(
            chain,
            vec!["claude-4-opus", "claude-4-sonnet", "gpt-4o"]
        );
    }

    #[test]
    fn deduplicates_fallbacks() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert(
            "gpt-4o".to_string(),
            vec!["gpt-4o".to_string(), "deepseek-chat".to_string()],
        );
        let chain = resolve_fallback_chain("gpt-4o", &fallbacks);
        assert_eq!(chain, vec!["gpt-4o", "deepseek-chat"]);
    }
}
