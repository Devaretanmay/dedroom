//! Instincts Engine — simple config-loaded rules that fire before other strategies.
//!
//! Instincts are in-memory only, loaded from `dedroom.yaml` at startup.
//! They are the highest-authority tier in the healing pipeline:
//!
//! ```text
//! generate_hint() priority:
//!   1. InstinctsEngine   (config rules, highest authority)
//!   2. HealingMemory     (per-tool strategy tracking)
//!   3. Fresh mutations   (generated on-demand)
//! ```

use serde::{Deserialize, Serialize};

// ── Data Models ─────────────────────────────────────────────────────────────

/// When this instinct applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstinctCondition {
    /// Always apply when this tool is called.
    Always,
    /// Only when the tool produced an error.
    OnError,
    /// When a specific parameter exceeds a threshold.
    ParamExceeds { param: String, threshold: f64 },
    /// When the call has repeated N+ times.
    OnRepeat { min_repeats: u32 },
}

/// A user-defined instinct rule from config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstinctRuleDef {
    pub tool: String,
    pub condition: InstinctCondition,
    pub action: String,
    #[serde(default = "default_rule_confidence")]
    pub confidence: f64,
}

fn default_rule_confidence() -> f64 { 0.8 }

/// Configuration for the instincts subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstinctsConfig {
    /// Enable the instincts engine.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// User-defined instinct rules.
    #[serde(default)]
    pub rules: Vec<InstinctRuleDef>,
}

fn default_true() -> bool { true }

impl Default for InstinctsConfig {
    fn default() -> Self {
        Self { enabled: true, rules: Vec::new() }
    }
}

// ── Reduced in-memory rule ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct InstinctRule {
    pub tool_name: String,
    pub condition: InstinctCondition,
    pub action: String,
    pub confidence: f64,
}

/// Simple in-memory instinct engine. Rules are loaded from config at startup.
#[derive(Debug, Clone)]
pub struct InstinctsEngine {
    pub enabled: bool,
    rules: Vec<InstinctRule>,
}

impl InstinctsEngine {
    /// Create from config, loading user-defined rules.
    pub fn from_config(config: &InstinctsConfig) -> Self {
        let rules: Vec<InstinctRule> = config.rules.iter().map(|def| InstinctRule {
            tool_name: def.tool.clone(),
            condition: def.condition.clone(),
            action: def.action.clone(),
            confidence: def.confidence,
        }).collect();
        Self { enabled: config.enabled, rules }
    }

    /// Check if a tool call matches any instinct rules.
    /// Returns the hint text of the best matching rule, or `None`.
    pub fn check_instincts(
        &self,
        tool_name: &str,
        tool_args: &str,
        is_error: bool,
        repeat_count: u32,
    ) -> Option<String> {
        if !self.enabled || self.rules.is_empty() {
            return None;
        }

        let mut best: Option<(f64, String)> = None;

        for rule in &self.rules {
            if rule.tool_name != "*" && rule.tool_name != tool_name {
                continue;
            }

            if !evaluate_condition(&rule.condition, tool_args, is_error, repeat_count) {
                continue;
            }

            let hint = build_hint(&rule.action, rule.confidence);

            if best.as_ref().map_or(true, |(c, _)| rule.confidence > *c) {
                best = Some((rule.confidence, hint));
            }
        }

        best.map(|(_, hint)| hint)
    }

    /// The number of loaded instinct rules (for admin API).
    pub fn instinct_count(&self) -> usize {
        self.rules.len()
    }

    /// List loaded instinct rules as simple serde values (for admin API).
    pub fn list_instincts(&self) -> Vec<serde_json::Value> {
        self.rules.iter().map(|rule| {
            serde_json::json!({
                "tool_name": rule.tool_name,
                "confidence": rule.confidence,
            })
        }).collect()
    }
}

impl Default for InstinctsEngine {
    fn default() -> Self {
        Self { enabled: true, rules: Vec::new() }
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn evaluate_condition(
    condition: &InstinctCondition,
    tool_args: &str,
    is_error: bool,
    repeat_count: u32,
) -> bool {
    match condition {
        InstinctCondition::Always => true,
        InstinctCondition::OnError => is_error,
        InstinctCondition::OnRepeat { min_repeats } => repeat_count >= *min_repeats,
        InstinctCondition::ParamExceeds { param, threshold } => {
            check_param_exceeds(tool_args, param, *threshold)
        }
    }
}

fn check_param_exceeds(args: &str, param: &str, threshold: f64) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(args) {
        if let Some(obj) = value.as_object() {
            if let Some(serde_json::Value::Number(n)) = obj.get(param) {
                if let Some(val) = n.as_f64() {
                    return val > threshold;
                }
                if let Some(val) = n.as_u64() {
                    return (val as f64) > threshold;
                }
            }
        }
    }
    args.contains(&format!("\"{}\":", param))
        || args.contains(&format!("{}:", param))
        || args.contains(&format!("{}=", param))
}

fn build_hint(action: &str, confidence: f64) -> String {
    format!(
        "[Instinct] {} (confidence: {:.0}%)",
        action,
        confidence * 100.0,
    )
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with_rules(rules: Vec<InstinctRuleDef>) -> InstinctsEngine {
        let config = InstinctsConfig { enabled: true, rules };
        InstinctsEngine::from_config(&config)
    }

    #[test]
    fn test_always_condition_matches() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "write_file".into(),
            condition: InstinctCondition::Always,
            action: "Try edit_file instead.".into(),
            confidence: 0.9,
        }]);
        let hint = engine.check_instincts("write_file", "{}", false, 0);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("edit_file"));
    }

    #[test]
    fn test_on_error_condition() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "*".into(),
            condition: InstinctCondition::OnError,
            action: "Check the error message.".into(),
            confidence: 0.8,
        }]);
        assert!(engine.check_instincts("read_file", "{}", true, 2).is_some());
        assert!(engine.check_instincts("read_file", "{}", false, 2).is_none());
    }

    #[test]
    fn test_on_repeat_condition() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "search".into(),
            condition: InstinctCondition::OnRepeat { min_repeats: 3 },
            action: "Try a different query.".into(),
            confidence: 0.7,
        }]);
        assert!(engine.check_instincts("search", "{}", false, 3).is_some());
        assert!(engine.check_instincts("search", "{}", false, 2).is_none());
    }

    #[test]
    fn test_param_exceeds_condition() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "list_files".into(),
            condition: InstinctCondition::ParamExceeds { param: "depth".into(), threshold: 3.0 },
            action: "Reduce depth to 1.".into(),
            confidence: 0.9,
        }]);
        assert!(engine.check_instincts("list_files", r#"{"depth":5}"#, false, 0).is_some());
        assert!(engine.check_instincts("list_files", r#"{"depth":2}"#, false, 0).is_none());
    }

    #[test]
    fn test_tool_wildcard() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "*".into(),
            condition: InstinctCondition::OnError,
            action: "Generic error advice.".into(),
            confidence: 0.5,
        }]);
        assert!(engine.check_instincts("any_tool", "{}", true, 0).is_some());
    }

    #[test]
    fn test_disabled_engine() {
        let config = InstinctsConfig { enabled: false, rules: vec![InstinctRuleDef {
            tool: "*".into(),
            condition: InstinctCondition::Always,
            action: "test".into(),
            confidence: 0.9,
        }]};
        let engine = InstinctsEngine::from_config(&config);
        assert!(engine.check_instincts("test", "{}", false, 0).is_none());
    }

    #[test]
    fn test_no_match_for_different_tool() {
        let engine = engine_with_rules(vec![InstinctRuleDef {
            tool: "specific_tool".into(),
            condition: InstinctCondition::Always,
            action: "only for specific tool".into(),
            confidence: 0.9,
        }]);
        assert!(engine.check_instincts("other_tool", "{}", false, 0).is_none());
        assert!(engine.check_instincts("specific_tool", "{}", false, 0).is_some());
    }

    #[test]
    fn test_best_confidence_wins() {
        let engine = engine_with_rules(vec![
            InstinctRuleDef {
                tool: "*".into(),
                condition: InstinctCondition::Always,
                action: "low confidence".into(),
                confidence: 0.3,
            },
            InstinctRuleDef {
                tool: "*".into(),
                condition: InstinctCondition::Always,
                action: "high confidence".into(),
                confidence: 0.9,
            },
        ]);
        let hint = engine.check_instincts("any", "{}", false, 0).unwrap();
        assert!(hint.contains("high confidence"));
    }

    #[test]
    fn test_empty_rules_yields_none() {
        let engine = engine_with_rules(Vec::new());
        assert!(engine.check_instincts("any", "{}", false, 0).is_none());
    }

    #[test]
    fn test_list_instincts() {
        let engine = engine_with_rules(vec![
            InstinctRuleDef {
                tool: "a".into(),
                condition: InstinctCondition::Always,
                action: "rule a".into(),
                confidence: 0.5,
            },
            InstinctRuleDef {
                tool: "b".into(),
                condition: InstinctCondition::OnError,
                action: "rule b".into(),
                confidence: 0.6,
            },
        ]);
        assert_eq!(engine.instinct_count(), 2);
        let list = engine.list_instincts();
        assert_eq!(list.len(), 2);
    }
}
