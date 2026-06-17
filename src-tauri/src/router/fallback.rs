use std::collections::HashMap;

/// Resolve the fallback chain for a given model.
///
/// Matching priority:
/// 1. Exact key match
/// 2. Date-suffix stripped exact match (e.g., "claude-3-opus-20240229" -> "claude-3-opus")
/// 3. Wildcard pattern match (e.g., "gpt-4*" matches "gpt-4-turbo"), longest prefix first
/// 4. Date-suffix stripped wildcard match
pub fn resolve_fallback_chain(
    model: &str,
    fallbacks: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut chain = vec![model.to_string()];

    // Collect fallback lists in priority order
    let stripped = strip_date_suffix(model);

    let mut lists: Vec<&Vec<String>> = Vec::new();

    // 1. Exact match
    if let Some(fb) = fallbacks.get(model) {
        lists.push(fb);
    }

    // 2. Stripped exact match
    if let Some(ref stripped_model) = stripped {
        if let Some(fb) = fallbacks.get(stripped_model) {
            lists.push(fb);
        }
    }

    // 3. Wildcard matches on original model
    let mut wildcard_matches = match_wildcards(model, fallbacks);

    // 4. Wildcard matches on stripped model
    if let Some(ref stripped_model) = stripped {
        wildcard_matches.extend(match_wildcards(stripped_model, fallbacks));
    }

    // Deduplicate wildcard matches while preserving order
    let mut seen = std::collections::HashSet::new();
    for (_, fb_list) in &wildcard_matches {
        if seen.insert(fb_list.as_ptr() as usize) {
            lists.push(fb_list);
        }
    }

    // Merge all lists, deduplicating
    for fb_list in &lists {
        for fb in fb_list.iter() {
            if !chain.contains(fb) {
                chain.push(fb.clone());
            }
        }
    }

    chain
}

/// Strip a trailing date-like suffix from a model name.
/// Matches patterns like `-20240229` (8 digits) or `-0613` (4-8 digits).
pub(crate) fn strip_date_suffix(model: &str) -> Option<String> {
    // Look for trailing `-\d{4,8}` at the end
    let dash_pos = model.rfind('-')?;
    let suffix = &model[dash_pos + 1..];
    if suffix.len() >= 4 && suffix.len() <= 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
        let stripped = &model[..dash_pos];
        if !stripped.is_empty() && stripped != model {
            return Some(stripped.to_string());
        }
    }
    None
}

/// Find all wildcard pattern matches for a model, sorted by prefix length (longest first).
/// Returns (pattern_key, fallback_list) pairs.
fn match_wildcards<'a>(
    model: &str,
    fallbacks: &'a HashMap<String, Vec<String>>,
) -> Vec<(&'a String, &'a Vec<String>)> {
    let mut matches: Vec<(&String, &Vec<String>)> = fallbacks
        .iter()
        .filter(|(key, _)| *key == "*" || (key.ends_with('*') && key.len() > 1))
        .filter(|(key, _)| {
            let prefix = &key[..key.len() - 1];
            *key == "*" || model.starts_with(prefix)
        })
        .collect();
    // Sort by prefix length descending (longer = more specific = higher priority)
    matches.sort_by(|a, b| {
        let prefix_a = a.0.len() - 1;
        let prefix_b = b.0.len() - 1;
        prefix_b.cmp(&prefix_a)
    });
    matches
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
        assert_eq!(chain, vec!["claude-4-opus", "claude-4-sonnet", "gpt-4o"]);
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

    #[test]
    fn wildcard_pattern_matches() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert("gpt-4*".to_string(), vec!["gpt-3.5-turbo".to_string()]);
        let chain = resolve_fallback_chain("gpt-4-turbo", &fallbacks);
        assert_eq!(chain, vec!["gpt-4-turbo", "gpt-3.5-turbo"]);
    }

    #[test]
    fn exact_match_takes_priority_over_wildcard() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert("gpt-4o".to_string(), vec!["gpt-4-turbo".to_string()]);
        fallbacks.insert("gpt-4*".to_string(), vec!["gpt-3.5-turbo".to_string()]);
        let chain = resolve_fallback_chain("gpt-4o", &fallbacks);
        assert_eq!(chain, vec!["gpt-4o", "gpt-4-turbo", "gpt-3.5-turbo"]);
    }

    #[test]
    fn date_suffix_stripping() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert(
            "claude-3-opus".to_string(),
            vec!["claude-3-sonnet".to_string()],
        );
        let chain = resolve_fallback_chain("claude-3-opus-20240229", &fallbacks);
        assert_eq!(chain, vec!["claude-3-opus-20240229", "claude-3-sonnet"]);
    }

    #[test]
    fn date_suffix_not_stripped_for_short_suffixes() {
        let fallbacks = HashMap::new();
        let chain = resolve_fallback_chain("gpt-4", &fallbacks);
        assert_eq!(chain, vec!["gpt-4"]);
    }

    #[test]
    fn wildcard_after_date_stripping() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert("claude*".to_string(), vec!["gpt-4o".to_string()]);
        let chain = resolve_fallback_chain("claude-3-opus-20240229", &fallbacks);
        assert_eq!(chain, vec!["claude-3-opus-20240229", "gpt-4o"]);
    }

    #[test]
    fn catch_all_wildcard() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert("*".to_string(), vec!["gpt-3.5-turbo".to_string()]);
        let chain = resolve_fallback_chain("any-model", &fallbacks);
        assert_eq!(chain, vec!["any-model", "gpt-3.5-turbo"]);
    }

    #[test]
    fn multiple_wildcards_by_specificity() {
        let mut fallbacks = HashMap::new();
        fallbacks.insert("gpt*".to_string(), vec!["fallback-gpt".to_string()]);
        fallbacks.insert(
            "gpt-4*".to_string(),
            vec!["fallback-gpt4".to_string()],
        );
        let chain = resolve_fallback_chain("gpt-4o", &fallbacks);
        // gpt-4* (5 chars) is more specific than gpt* (3 chars)
        assert_eq!(
            chain,
            vec!["gpt-4o", "fallback-gpt4", "fallback-gpt"]
        );
    }

    #[test]
    fn no_false_strips_on_non_date_suffix() {
        assert_eq!(strip_date_suffix("gpt-4"), None);
        assert_eq!(strip_date_suffix("claude-3"), None);
        assert_eq!(strip_date_suffix("model-123"), None); // 3 digits, too short
    }

    #[test]
    fn strip_date_suffix_correct() {
        assert_eq!(
            strip_date_suffix("claude-3-opus-20240229"),
            Some("claude-3-opus".to_string())
        );
        assert_eq!(
            strip_date_suffix("gpt-4-0613"),
            Some("gpt-4".to_string())
        );
        assert_eq!(
            strip_date_suffix("text-embedding-3-small-20240101"),
            Some("text-embedding-3-small".to_string())
        );
    }
}
