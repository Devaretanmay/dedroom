//! Parity test harness for DedrooM.
//!
//! Compares Rust pipeline output against recorded JSON fixtures to
//! ensure correctness across versions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A parity test fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityFixture {
    pub name: String,
    pub module: String,
    pub input: serde_json::Value,
    pub expected_output: serde_json::Value,
    pub tolerances: Option<HashMap<String, f64>>,
}

/// Result of a single parity check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityResult {
    pub name: String,
    pub passed: bool,
    pub actual: serde_json::Value,
    pub expected: serde_json::Value,
    pub diff: Option<String>,
}

/// Run parity checks for a set of fixtures.
pub fn check_parity(fixtures: &[ParityFixture]) -> Vec<ParityResult> {
    let mut results = Vec::new();
    for fixture in fixtures {
        let result = match fixture.module.as_str() {
            "smart_crusher" => check_smart_crusher(fixture),
            "loop_detection" => check_loop_detection(fixture),
            "content_router" => check_content_router(fixture),
            _ => ParityResult {
                name: fixture.name.clone(),
                passed: false,
                actual: serde_json::json!({"error": "unknown module"}),
                expected: fixture.expected_output.clone(),
                diff: Some("no comparator for module".into()),
            },
        };
        results.push(result);
    }
    results
}

fn check_smart_crusher(fixture: &ParityFixture) -> ParityResult {
    let input_str = fixture.input.to_string();
    let result = dedroom_core::compression::smart_crusher::compress_json_array(
        &input_str, 0.3,
    );

    match result {
        Ok(compressed) => {
            let actual = serde_json::json!({
                "content": compressed.content,
                "original_count": compressed.original_count,
                "compressed_count": compressed.compressed_count,
            });
            let passed = actual == fixture.expected_output;
            ParityResult {
                name: fixture.name.clone(),
                passed,
                actual,
                expected: fixture.expected_output.clone(),
                diff: if passed { None } else { Some("output mismatch".into()) },
            }
        }
        Err(e) => ParityResult {
            name: fixture.name.clone(),
            passed: false,
            actual: serde_json::json!({"error": e.to_string()}),
            expected: fixture.expected_output.clone(),
            diff: Some(e.to_string()),
        },
    }
}

fn check_loop_detection(fixture: &ParityFixture) -> ParityResult {
    use dedroom_core::config::LoopDetectionConfig;
    use dedroom_core::loop_detection::LoopDetector;

    let config = LoopDetectionConfig::default();
    let mut detector = LoopDetector::new(&config);

    let tool = fixture.input.get("tool").and_then(|v| v.as_str()).unwrap_or("unknown");
    let args = fixture.input.get("args").and_then(|v| v.as_str()).unwrap_or("{}");

    let verdict = detector.verify(tool, args);
    let actual = serde_json::json!({"verdict": verdict.to_code()});

    let passed = actual == fixture.expected_output;
    ParityResult {
        name: fixture.name.clone(),
        passed,
        actual,
        expected: fixture.expected_output.clone(),
        diff: if passed { None } else { Some("verdict mismatch".into()) },
    }
}

fn check_content_router(fixture: &ParityFixture) -> ParityResult {
    use dedroom_core::config::ContentRouterConfig;
    use dedroom_core::compression::ContentRouter;

    let router = ContentRouter::new(&ContentRouterConfig::default());
    let content = fixture.input.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let detected = router.detect_type(content);
    let actual = serde_json::json!({"content_type": detected.name()});

    let passed = actual == fixture.expected_output;
    ParityResult {
        name: fixture.name.clone(),
        passed,
        actual,
        expected: fixture.expected_output.clone(),
        diff: if passed { None } else { Some("content type mismatch".into()) },
    }
}

/// Print parity results in a human-readable format.
pub fn print_results(results: &[ParityResult]) {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    println!("─── Parity Results ───");
    for r in results {
        let status = if r.passed { "✅" } else { "❌" };
        println!("  {} {}: {}", status, r.name, r.module());
    }
    println!("─── {}/{} passed ───", passed, total);
}

trait ModuleName {
    fn module(&self) -> &str;
}

impl ModuleName for ParityResult {
    fn module(&self) -> &str {
        if self.diff.as_deref() == Some("no comparator for module") {
            "no comparator"
        } else if self.passed {
            "passed"
        } else {
            "failed"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_router_parity() {
        let fixtures = vec![ParityFixture {
            name: "detect_json".into(),
            module: "content_router".into(),
            input: serde_json::json!({"content": r#"{"key":"value"}"#}),
            expected_output: serde_json::json!({"content_type": "json_object"}),
            tolerances: None,
        }];
        let results = check_parity(&fixtures);
        assert!(results[0].passed);
    }

    #[test]
    fn test_loop_detection_parity() {
        let fixtures = vec![ParityFixture {
            name: "allow_first_call".into(),
            module: "loop_detection".into(),
            input: serde_json::json!({
                "tool": "write_file",
                "args": r#"{"path":"/tmp/x.txt"}"#
            }),
            expected_output: serde_json::json!({"verdict": 0}),
            tolerances: None,
        }];
        let results = check_parity(&fixtures);
        assert!(results[0].passed);
    }
}
