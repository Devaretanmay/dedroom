//! Canonicalization of tool arguments for loop detection.
//!
//! Removes volatile fields (timestamps, request IDs, etc.) from JSON
//! arguments before comparison, and provides auto-inference of which
//! fields change on every call.

use std::collections::{HashMap, HashSet};
use serde_json::{Value, Map};

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

/// Compact JSON representation for comparison.
fn compact_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .filter_map(|k| {
                    map.get(*k).map(|v| format!("\"{}\":{}", k, compact_json(v)))
                })
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(compact_json).collect();
            format!("[{}]", items.join(","))
        }
        Value::String(s) => format!("\"{}\"", s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
    }
}

/// Tracks field-level differences across consecutive calls to auto-infer
/// volatile fields (fields that change every time, like `request_id`).
#[derive(Debug)]
pub struct VolatileInferenceEngine {
    enabled: bool,
    min_occurrences: usize,
    /// Per-tool: previous call's args as JSON map
    previous_args: HashMap<String, Map<String, Value>>,
    /// Per-tool, per-field: count of consecutive differences
    field_diff_counts: HashMap<String, HashMap<String, usize>>,
    /// Per-tool: inferred volatile fields
    inferred: HashMap<String, HashSet<String>>,
}

impl VolatileInferenceEngine {
    pub fn new(enabled: bool, min_occurrences: usize) -> Self {
        Self {
            enabled,
            min_occurrences: min_occurrences.max(1),
            previous_args: HashMap::new(),
            field_diff_counts: HashMap::new(),
            inferred: HashMap::new(),
        }
    }

    /// Process the next set of arguments for a tool.
    /// Returns the canonicalized arguments with auto-inferred fields stripped.
    pub fn process(&mut self, tool: &str, canonical_args: &str) -> String {
        if !self.enabled {
            return canonical_args.to_string();
        }

        let current: Map<String, Value> = match serde_json::from_str(canonical_args) {
            Ok(Value::Object(m)) => m,
            _ => return canonical_args.to_string(),
        };



        // Check for new volatile fields by comparing with previous call
        if let Some(prev) = self.previous_args.get(tool) {
            let diff_counts = self.field_diff_counts
                .entry(tool.to_string())
                .or_default();

            for key in current.keys() {
                if let Some(prev_val) = prev.get(key) {
                    let cur_val = &current[key];
                    if prev_val != cur_val {
                        let count = diff_counts.entry(key.clone()).or_insert(0);
                        *count += 1;
                        if *count >= self.min_occurrences {
                            self.inferred
                                .entry(tool.to_string())
                                .or_default()
                                .insert(key.clone());
                        }
                    } else {
                        diff_counts.remove(key);
                    }
                }
            }
        }

        self.previous_args.insert(tool.to_string(), current);

        // Compute inferred fields AFTER the comparison loop
        let inferred_set = self.inferred.get(tool).cloned().unwrap_or_default();

        // Strip inferred volatile fields
        if inferred_set.is_empty() {
            canonical_args.to_string()
        } else {
            let inferred_fields: Vec<&str> = inferred_set.iter().map(|s| s.as_str()).collect();
            strip_volatile_fields(canonical_args, &inferred_fields)
        }
    }

    /// Get the currently inferred volatile fields for a tool.
    pub fn inferred_fields(&self, tool: &str) -> HashSet<String> {
        self.inferred.get(tool).cloned().unwrap_or_default()
    }

    /// Reset inference state for a tool.
    pub fn reset(&mut self, tool: &str) {
        self.previous_args.remove(tool);
        self.field_diff_counts.remove(tool);
        self.inferred.remove(tool);
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

    #[test]
    fn test_volatile_inference() {
        let mut engine = VolatileInferenceEngine::new(true, 2);

        // First call — nothing to compare
        let r1 = engine.process("search", r#"{"query":"hello","req_id":"1"}"#);
        assert_eq!(r1, r#"{"query":"hello","req_id":"1"}"#);

        // Second call — req_id differs once
        let r2 = engine.process("search", r#"{"query":"hello","req_id":"2"}"#);
        assert_eq!(r2, r#"{"query":"hello","req_id":"2"}"#);

        // Third call — req_id differs twice, now inferred as volatile
        let r3 = engine.process("search", r#"{"query":"hello","req_id":"3"}"#);
        // req_id should be stripped
        assert!(!r3.contains("req_id"));
        assert!(r3.contains("hello"));

        // Check inference state
        let inferred = engine.inferred_fields("search");
        assert!(inferred.contains("req_id"));
    }

    #[test]
    fn test_no_false_inference_on_stable_fields() {
        let mut engine = VolatileInferenceEngine::new(true, 2);

        for i in 0..5u32 {
            let args = format!(r#"{{"query":"hello","req_id":"{i}"}}"#);
            engine.process("search", &args);
        }

        // After 5 calls with always the same query but different req_id...
        let inferred = engine.inferred_fields("search");
        // req_id should be inferred as volatile once we're past min_occurrences
        // query should NOT be inferred since it never changed
        assert!(!inferred.contains("query"));
    }

    #[test]
    fn test_inference_requires_min_occurrences() {
        let mut engine = VolatileInferenceEngine::new(true, 3);

        engine.process("search", r#"{"query":"hello","req_id":"1"}"#);
        engine.process("search", r#"{"query":"hello","req_id":"2"}"#);
        // Only 2 diffs, min_occurrences is 3 — not yet inferred
        let inferred = engine.inferred_fields("search");
        assert!(!inferred.contains("req_id"));
    }
}
