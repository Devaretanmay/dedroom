//! SmartCrusher — JSON array compressor.
//!
//! Compresses arrays of JSON objects by selecting a subset of rows that
//! maximize content coverage. Uses a greedy coverage algorithm based on
//! bigram diversity.

use serde_json::Value;
use std::collections::HashSet;

/// Maximum rows to feed into the O(n²) greedy selection.
/// Beyond this, only the first `MAX_ROWS` are considered.
const MAX_ROWS: usize = 200;

/// Compress a JSON array string, returning the compressed result.
pub fn compress_json_array(
    input: &str,
    retention: f64,
) -> Result<CompressedJson, String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|e| format!("parse error: {e}"))?;

    let array = match value {
        Value::Array(arr) => arr,
        Value::Object(_) => return compress_single_object(input),
        _ => return Err("unsupported type for smart crusher".into()),
    };

    if array.is_empty() || array.len() <= 2 {
        return Ok(CompressedJson {
            content: input.to_string(),
            original_count: array.len(),
            compressed_count: array.len(),
            rows_dropped: 0,
        });
    }

    compress_slice(&array, retention, array.len())
}

/// Compress a pre-parsed slice of JSON values (avoids double-parse
/// when the caller already parsed via content routing).
///
/// Caps the working set at [`MAX_ROWS`] rows to keep the O(n²)
/// greedy selection bounded.
pub(crate) fn compress_slice(
    rows: &[Value],
    retention: f64,
    original_count: usize,
) -> Result<CompressedJson, String> {
    if rows.is_empty() || rows.len() <= 2 {
        let content = serde_json::to_string(rows).unwrap_or_default();
        return Ok(CompressedJson {
            content,
            original_count,
            compressed_count: rows.len(),
            rows_dropped: original_count - rows.len(),
        });
    }

    // Cap working set to bound greedy selection cost
    let working_set = if rows.len() > MAX_ROWS {
        &rows[..MAX_ROWS]
    } else {
        rows
    };

    let num_to_keep = (working_set.len() as f64 * retention).max(1.0).ceil() as usize;
    let num_to_keep = num_to_keep.min(working_set.len());

    let selected = greedy_select_rows(working_set, num_to_keep);

    let compressed_rows: Vec<Value> = selected.iter().map(|&idx| working_set[idx].clone()).collect();
    let compressed_content = serde_json::to_string(&compressed_rows)
        .map_err(|e| format!("serialize error: {e}"))?;

    Ok(CompressedJson {
        content: compressed_content,
        original_count,
        compressed_count: compressed_rows.len(),
        rows_dropped: original_count - compressed_rows.len(),
    })
}

/// Compress a single JSON object (wraps in array if needed).
fn compress_single_object(input: &str) -> Result<CompressedJson, String> {
    Ok(CompressedJson {
        content: input.to_string(),
        original_count: 1,
        compressed_count: 1,
        rows_dropped: 0,
    })
}

/// Greedy row selection: iteratively pick the row that adds the most
/// new bigrams to the coverage set.
fn greedy_select_rows(rows: &[Value], num_to_keep: usize) -> Vec<usize> {
    if rows.is_empty() || num_to_keep == 0 {
        return Vec::new();
    }

    let row_bigrams: Vec<HashSet<String>> = rows.iter().map(extract_bigrams).collect();
    let mut selected: Vec<usize> = Vec::with_capacity(num_to_keep);
    let mut covered: HashSet<String> = HashSet::new();
    let mut remaining: HashSet<usize> = (0..rows.len()).collect();

    while selected.len() < num_to_keep && !remaining.is_empty() {
        let best = remaining.iter()
            .max_by_key(|&&idx| {
                row_bigrams[idx].difference(&covered).count()
            })
            .copied();

        if let Some(best_idx) = best {
            selected.push(best_idx);
            covered.extend(row_bigrams[best_idx].iter().cloned());
            remaining.remove(&best_idx);
        } else {
            break;
        }
    }

    selected.sort();
    selected
}

/// Extract bigrams from a JSON value for coverage comparison.
fn extract_bigrams(value: &Value) -> HashSet<String> {
    let mut bigrams = HashSet::new();
    collect_bigrams(value, &mut bigrams);
    bigrams
}

fn collect_bigrams(value: &Value, output: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                // Object key as a bigram marker
                output.insert(format!("key:{}", k));
                collect_bigrams(v, output);
            }
        }
        Value::Array(arr) => {
            output.insert("[array]".to_string());
            for v in arr {
                collect_bigrams(v, output);
            }
        }
        Value::String(s) => {
            for w in s.split_whitespace() {
                output.insert(format!("word:{}", w.to_lowercase()));
            }
        }
        Value::Number(n) => {
            output.insert(format!("num:{}", n));
        }
        Value::Bool(b) => {
            output.insert(format!("bool:{}", b));
        }
        Value::Null => {
            output.insert("null".to_string());
        }
    }
}

/// Result of compression.
#[derive(Debug, Clone)]
pub struct CompressedJson {
    pub content: String,
    pub original_count: usize,
    pub compressed_count: usize,
    pub rows_dropped: usize,
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_array_of_objects() {
        let input = r#"[
            {"id": 1, "name": "Alice", "email": "alice@example.com"},
            {"id": 2, "name": "Bob", "email": "bob@example.com"},
            {"id": 3, "name": "Charlie", "email": "charlie@example.com"},
            {"id": 4, "name": "Diana", "email": "diana@example.com"},
            {"id": 5, "name": "Eve", "email": "eve@example.com"}
        ]"#;
        let result = compress_json_array(input, 0.4).unwrap();
        assert_eq!(result.original_count, 5);
        assert_eq!(result.compressed_count, 2);
        assert_eq!(result.rows_dropped, 3);
        // Result should be valid JSON
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn test_empty_array() {
        let result = compress_json_array("[]", 0.5).unwrap();
        assert_eq!(result.original_count, 0);
    }

    #[test]
    fn test_single_object() {
        let result = compress_json_array(r#"{"key": "value"}"#, 0.5).unwrap();
        assert_eq!(result.original_count, 1);
    }

    #[test]
    fn test_small_array_no_compression() {
        let input = r#"[{"a": 1}, {"b": 2}]"#;
        let result = compress_json_array(input, 0.5).unwrap();
        assert_eq!(result.original_count, 2);
        assert_eq!(result.compressed_count, 2); // retention=0.5 → ceil(2*0.5)=1 max 2
    }

    #[test]
    fn test_greedy_selection_basic() {
        let rows: Vec<Value> = vec![
            serde_json::json!({"a": 1}),
            serde_json::json!({"b": 2}),
            serde_json::json!({"c": 3}),
        ];
        let selected = greedy_select_rows(&rows, 2);
        assert_eq!(selected.len(), 2);
        // Should pick 2 out of 3 rows — any pair is valid since all have unique bigrams
        assert!(selected[0] < selected[1]);
        assert!(selected[1] < 3);
    }
}
