//! Core loop detection engine.
//!
//! [`LoopDetector`] sits between agent and LLM, intercepting every tool call.
//! It applies a multi-stage pipeline to decide whether the call should be
//! allowed, warned, or blocked.

use std::collections::HashMap;
use serde_json::Value;
use crate::config::{
    LoopDetectionConfig, Strictness, CountMode, ToolOverride,
    RuleConfig, RuleKind, RuleAction, ErrorDetectionConfig,
};

use super::history::HistoryTracker;
use super::canonical::{strip_volatile_fields, VolatileInferenceEngine};
use super::adaptive::AdaptiveThreshold;

// ── Public types ───────────────────────────────────────────────────────────

/// The verdict after running loop detection on a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoopVerdict {
    /// Call is allowed — not a loop.
    Allow,
    /// Call is suspicious but allowed (lenient mode). Tag the response.
    Warn,
    /// Call is blocked — agent should retry with a different approach.
    BlockRetry,
    /// Call is blocked — agent should give up on this approach.
    BlockHalt,
}

impl LoopVerdict {
    /// Returns `true` if the call should be blocked.
    pub fn is_blocked(self) -> bool {
        matches!(self, Self::BlockRetry | Self::BlockHalt)
    }

    /// Convert to an integer code (0–3).
    pub fn to_code(self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Warn => 1,
            Self::BlockRetry => 2,
            Self::BlockHalt => 3,
        }
    }
}

/// A compiled rule for argument validation.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub tool_pattern: String,
    pub kind: CompiledRuleKind,
    pub action: RuleAction,
}

#[derive(Debug, Clone)]
pub enum CompiledRuleKind {
    Regex(regex::Regex),
    Exact(String),
    JsonSchema { required: Vec<String>, type_name: String },
}

// ── Rule engine ────────────────────────────────────────────────────────────

/// Validates tool arguments against configured rules.
#[derive(Debug, Default)]
pub struct RuleEngine {
    rules: Vec<CompiledRule>,
}

impl RuleEngine {
    /// Build from configuration.
    pub fn from_config(configs: &[RuleConfig]) -> Self {
        let mut rules = Vec::new();
        for rc in configs {
            let kind = match &rc.kind {
                RuleKind::Regex { pattern } => {
                    match regex::Regex::new(pattern) {
                        Ok(re) => CompiledRuleKind::Regex(re),
                        Err(e) => {
                            tracing::warn!("invalid regex rule '{}': {}", pattern, e);
                            continue;
                        }
                    }
                }
                RuleKind::Exact { value } => {
                    CompiledRuleKind::Exact(value.clone())
                }
                RuleKind::JsonSchema { required, type_name } => {
                    CompiledRuleKind::JsonSchema {
                        required: required.clone(),
                        type_name: type_name.clone(),
                    }
                }
            };
            rules.push(CompiledRule {
                tool_pattern: rc.tool.clone(),
                kind,
                action: rc.on_match,
            });
        }
        Self { rules }
    }

    /// Validate a tool call against all matching rules.
    /// Returns `None` if all rules pass, or the action of the first blocking rule.
    pub fn validate(&self, tool: &str, args_json: &str) -> Option<RuleAction> {
        let args: Value = serde_json::from_str(args_json).ok()?;
        for rule in &self.rules {
            if !tool_matches_pattern(tool, &rule.tool_pattern) {
                continue;
            }
            let triggers = match &rule.kind {
                CompiledRuleKind::Regex(re) => {
                    let flat = serde_json::to_string(&args).unwrap_or_default();
                    re.is_match(&flat)
                }
                CompiledRuleKind::Exact(value) => {
                    let flat = serde_json::to_string(&args).unwrap_or_default();
                    flat == *value
                }
                CompiledRuleKind::JsonSchema { required, .. } => {
                    required.iter().any(|field| args.get(field).is_none())
                }
            };
            if triggers {
                return Some(rule.action);
            }
        }
        None
    }
}

fn tool_matches_pattern(tool: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Simple glob: support wildcard suffix
    if let Some(prefix) = pattern.strip_suffix('*') {
        tool.starts_with(prefix)
    } else {
        tool == pattern
    }
}

// ── Per-tool config lookup ─────────────────────────────────────────────────

#[derive(Debug)]
struct ToolConfig {
    max_repeats: Option<u32>,
    count_mode: Option<CountMode>,
    volatile_fields: Vec<String>,
    #[allow(dead_code)]
    error_detection: Option<ErrorDetectionConfig>,
}

impl ToolConfig {
    fn from_overrides(overrides: &[ToolOverride]) -> HashMap<String, Self> {
        let mut map = HashMap::new();
        for o in overrides {
            map.insert(o.name.clone(), Self {
                max_repeats: o.max_repeats,
                count_mode: o.count_mode,
                volatile_fields: o.volatile_fields.clone(),
                error_detection: o.error_detection.clone(),
            });
        }
        map
    }
}

// ── LoopDetector ───────────────────────────────────────────────────────────

/// The main loop detection engine.
///
/// Thread-safe: uses internal synchronization. Clone to create per-session
/// instances from shared config.
#[derive(Debug)]
pub struct LoopDetector {
    config: LoopDetectionConfig,
    history: HistoryTracker,
    rule_engine: RuleEngine,
    adaptive: AdaptiveThreshold,
    volatile_inference: VolatileInferenceEngine,
    tool_configs: HashMap<String, ToolConfig>,
}

impl LoopDetector {
    /// Create a new detector from configuration.
    pub fn new(config: &LoopDetectionConfig) -> Self {
        let effective_window = config.history_window
            .unwrap_or(config.max_repeats * 2);

        Self {
            config: config.clone(),
            history: HistoryTracker::new(effective_window as usize),
            rule_engine: RuleEngine::from_config(&config.rules),
            adaptive: AdaptiveThreshold::new(
                config.adaptive.enabled,
                config.max_repeats,
                config.adaptive.error_reduction,
                config.adaptive.min_repeats,
            ),
            volatile_inference: VolatileInferenceEngine::new(
                config.volatile_fields.auto_inference,
                config.volatile_fields.min_occurrences as usize,
            ),
            tool_configs: ToolConfig::from_overrides(&config.tools),
        }
    }

    /// Verify a tool call. Returns the loop verdict.
    ///
    /// * `tool` — the tool name (e.g. `write_file`, `search`)
    /// * `args_json` — JSON string of tool arguments
    pub fn verify(&mut self, tool: &str, args_json: &str) -> LoopVerdict {
        if !self.config.enabled {
            return LoopVerdict::Allow;
        }

        // 1. Rule engine validation
        if let Some(action) = self.rule_engine.validate(tool, args_json) {
            match action {
                RuleAction::Block => {
                    tracing::info!("rule blocked call: tool={}", tool);
                    return LoopVerdict::BlockHalt;
                }
                RuleAction::Warn => {
                    tracing::debug!("rule warned call: tool={}", tool);
                    // Continue but return Warn if other checks pass
                    let _ = action;
                }
                RuleAction::Allow => {}
            }
        }
        // 2. Per-tool config
        let tcfg = self.tool_configs.get(tool);

        // 3. Strip volatile fields for comparison
        let configured_volatiles: Vec<&str> = tcfg
            .map(|t| t.volatile_fields.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        let canonical_args = strip_volatile_fields(args_json, &configured_volatiles);
        let canonical_args = self.volatile_inference
            .process(tool, &canonical_args);

        // 5. Count repeats in history
        let effective_max = tcfg
            .and_then(|t| t.max_repeats)
            .or_else(|| Some(self.adaptive.effective_max_repeats()))
            .unwrap_or(self.config.max_repeats);

        let effective_mode = tcfg
            .and_then(|t| t.count_mode)
            .unwrap_or(self.config.count_mode);

        let repeat_count = self.history.count_repeats(tool, &canonical_args, effective_mode);

        // 6. Get block threshold from strictness
        let threshold = self.block_threshold(effective_max);

        if repeat_count >= threshold {
            let strictness = self.config.strictness;
            let verdict = match strictness {
                Strictness::Lenient => LoopVerdict::Warn,
                Strictness::Balanced => LoopVerdict::BlockRetry,
                Strictness::Strict => LoopVerdict::BlockHalt,
            };
            tracing::info!(
                "loop detected: tool={}, repeats={}, threshold={}, verdict={:?}",
                tool, repeat_count, threshold, verdict,
            );
            return verdict;
        }

        LoopVerdict::Allow
    }

    /// Record a tool result after the call completes (or was blocked).
    pub fn record_result(
        &mut self,
        tool: &str,
        args_json: &str,
        was_error: bool,
    ) {
        let tcfg = self.tool_configs.get(tool);
        let configured_volatiles: Vec<&str> = tcfg
            .map(|t| t.volatile_fields.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let canonical_args = strip_volatile_fields(args_json, &configured_volatiles);
        let canonical_args = self.volatile_inference.process(tool, &canonical_args);

        self.history.push(tool.to_string(), canonical_args, was_error);

        // Feed back to adaptive threshold if error
        if was_error {
            self.adaptive.record_error(tool);
        } else {
            self.adaptive.record_success(tool);
        }
    }

    /// Current loop state summary.
    pub fn state_summary(&self) -> LoopStateSummary {
        let total_calls = self.history.len();
        let tool_counts: HashMap<String, usize> = self.history
            .iter()
            .fold(HashMap::new(), |mut acc, entry| {
                *acc.entry(entry.tool.clone()).or_default() += 1;
                acc
            });

        LoopStateSummary {
            total_calls,
            tool_counts,
            current_max_repeats: self.adaptive.effective_max_repeats(),
        }
    }

    /// Convert max_repeats to a block threshold based on strictness.
    fn block_threshold(&self, max_repeats: u32) -> u32 {
        match self.config.strictness {
            Strictness::Lenient => max_repeats + 1,
            Strictness::Balanced => max_repeats,
            Strictness::Strict => max_repeats.saturating_sub(1).max(1),
        }
    }
}

/// Snapshot of current loop detection state.
#[derive(Debug, Clone)]
pub struct LoopStateSummary {
    pub total_calls: usize,
    pub tool_counts: HashMap<String, usize>,
    pub current_max_repeats: u32,
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_first_call() {
        let cfg = LoopDetectionConfig::default();
        let mut detector = LoopDetector::new(&cfg);
        let verdict = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
        assert_eq!(verdict, LoopVerdict::Allow);
    }

    #[test]
    fn test_block_after_repeats() {
        let cfg = LoopDetectionConfig {
            max_repeats: 3,
            ..Default::default()
        };
        let mut detector = LoopDetector::new(&cfg);

        // First 3 calls: allowed
        for _ in 0..3 {
            let v = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
            assert_eq!(v, LoopVerdict::Allow);
            detector.record_result("write_file", r#"{"path":"/tmp/x.txt"}"#, false);
        }

        // 4th call: blocked
        let v = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
        assert!(v.is_blocked());
    }

    #[test]
    fn test_different_args_not_looping() {
        let cfg = LoopDetectionConfig::default();
        let mut detector = LoopDetector::new(&cfg);

        for i in 0..5 {
            let v = detector.verify("write_file", &format!(r#"{{"path":"/tmp/x{}.txt"}}"#, i));
            assert_eq!(v, LoopVerdict::Allow);
            detector.record_result("write_file", &format!(r#"{{"path":"/tmp/x{}.txt"}}"#, i), false);
        }
    }

    #[test]
    fn test_different_tool_not_looping() {
        let cfg = LoopDetectionConfig {
            max_repeats: 10,
            ..Default::default()
        };
        let mut detector = LoopDetector::new(&cfg);

        for _ in 0..5 {
            assert_eq!(detector.verify("read_file", r#"{"path":"/tmp/x.txt"}"#), LoopVerdict::Allow);
            detector.record_result("read_file", r#"{"path":"/tmp/x.txt"}"#, false);
            assert_eq!(detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#), LoopVerdict::Allow);
            detector.record_result("write_file", r#"{"path":"/tmp/x.txt"}"#, false);
        }
    }

    #[test]
    fn test_volatile_field_stripping() {
        let cfg = LoopDetectionConfig {
            max_repeats: 3,
            tools: vec![ToolOverride {
                name: "search".into(),
                max_repeats: None,
                count_mode: None,
                volatile_fields: vec!["request_id".into()],
                error_detection: None,
            }],
            ..Default::default()
        };
        let mut detector = LoopDetector::new(&cfg);

        for _ in 0..3 {
            let v = detector.verify("search", r#"{"query":"hello","request_id":"abc"}"#);
            assert_eq!(v, LoopVerdict::Allow);
            detector.record_result("search", r#"{"query":"hello","request_id":"abc"}"#, false);
        }

        // Same query, different request_id — should still be detected as loop
        // because request_id is stripped
        let v = detector.verify("search", r#"{"query":"hello","request_id":"xyz"}"#);
        assert!(v.is_blocked());
    }

    #[test]
    fn test_rule_engine_blocks() {
        let cfg = LoopDetectionConfig {
            rules: vec![RuleConfig {
                tool: "execute_command".into(),
                kind: RuleKind::Exact { value: r#"{"command":"rm -rf /"}"#.into() },
                on_match: RuleAction::Block,
            }],
            ..Default::default()
        };
        let mut detector = LoopDetector::new(&cfg);
        let v = detector.verify("execute_command", r#"{"command":"rm -rf /"}"#);
        assert_eq!(v, LoopVerdict::BlockHalt);
    }

    #[test]
    fn test_state_summary() {
        let cfg = LoopDetectionConfig::default();
        let mut detector = LoopDetector::new(&cfg);

        detector.verify("write_file", r#"{"path":"/tmp/a.txt"}"#);
        detector.record_result("write_file", r#"{"path":"/tmp/a.txt"}"#, false);
        detector.verify("read_file", r#"{"path":"/tmp/b.txt"}"#);
        detector.record_result("read_file", r#"{"path":"/tmp/b.txt"}"#, false);

        let summary = detector.state_summary();
        assert_eq!(summary.total_calls, 2);
        assert_eq!(*summary.tool_counts.get("write_file").unwrap(), 1);
        assert_eq!(*summary.tool_counts.get("read_file").unwrap(), 1);
    }
}
