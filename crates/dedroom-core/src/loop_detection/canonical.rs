//! Canonicalization of tool arguments for loop detection.
//!
//! Removes volatile fields (timestamps, request IDs, etc.) from JSON
//! arguments before comparison, and provides auto-inference of which
//! fields change on every call.

use serde_json::Value;

/// Strip configured volatile fields from a JSON arguments string.
///
/// Returns a compact JSON string with the specified fields removed at
/// the top level.
pub fn strip_volatile_fields(args_json: &str, volatile_fields: &[&str]) -> String {
    let mut value: Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(_) => return args_json.to_string(),
    };

    if volatile_fields.is_empty() {
        return compact_json(&value);
    }

    if let Value::Object(ref mut map) = value {
        for field in volatile_fields {
            map.remove(*field);
        }
    }

    compact_json(&value)
}

/// Compact JSON representation for comparison (sorted keys).
fn compact_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let obj: serde_json::Map<String, Value> = keys.iter()
                .map(|k| ((*k).clone(), map[*k].clone()))
                .collect();
            serde_json::to_string(&Value::Object(obj)).unwrap_or_default()
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_single_field() {
        let result = strip_volatile_fields(
            r#"{"query":"hello","request_id":"abc123"}"#,
            &["request_id"],
        );
        assert!(!result.contains("request_id"));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_strip_multiple_fields() {
        let result = strip_volatile_fields(
            r#"{"query":"hello","request_id":"abc","timestamp":"123"}"#,
            &["request_id", "timestamp"],
        );
        assert!(!result.contains("request_id"));
        assert!(!result.contains("timestamp"));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_strip_no_fields() {
        let result = strip_volatile_fields(
            r#"{"query":"hello"}"#,
            &[],
        );
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_compact_json_stable_ordering() {
        let a = strip_volatile_fields(r#"{"b":2,"a":1}"#, &[]);
        let b = strip_volatile_fields(r#"{"a":1,"b":2}"#, &[]);
        assert_eq!(a, b);
    }

}
