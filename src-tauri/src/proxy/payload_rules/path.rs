//! Dotted JSON path manipulation utilities.
//!
//! Paths use dot-separated segments (e.g., `"generationConfig.thinkingConfig.thinkingBudget"`).
//! Array indices are supported: `"messages.0.content"` indexes into a JSON array.

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
pub(crate) fn path_exists(body: &serde_json::Value, path: &str) -> bool {
    get_path(body, path).is_some()
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
}
