//! Self-Healing Engine — when loop detection blocks a tool call, this engine
//! generates alternative approaches ("mutations") to break the loop.
//!
//! The engine lives in the pipeline between loop detection and compression:
//!
//! ```text
//! Loop Detection → Self-Healing Decision → Compression → Forward
//!                      ├── No loop → passthrough
//!                      └── Loop → Mutation Engine → enhanced hint
//! ```

pub mod mutations;
pub mod memory;

use std::collections::HashMap;
use std::sync::Mutex;
use crate::config::SelfHealingConfig;

/// A healing context built from the current pipeline state.
#[derive(Debug, Clone)]
pub struct HealingContext {
    pub tool_name: String,
    pub tool_args: String,
    pub is_error: bool,
    pub repeat_count: u32,
    pub tilt_index: f64,
    pub session_tool_count: usize,
}

impl HealingContext {
    pub fn new(
        tool_name: &str,
        tool_args: &str,
        is_error: bool,
        repeat_count: u32,
        tilt_index: f64,
        session_tool_count: usize,
    ) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_args: tool_args.to_string(),
            is_error,
            repeat_count,
            tilt_index,
            session_tool_count,
        }
    }
}

/// The self-healing engine orchestrates mutation generation and scoring.
#[derive(Debug)]
pub struct SelfHealingEngine {
    config: SelfHealingConfig,
    pub memory: memory::HealingMemory,
    /// Pending outcomes from `generate_hint` that need to be evaluated
    /// on the next request to determine if the mutation broke the loop.
    /// Maps tool_name → strategy_label.
    pending_outcome: Mutex<HashMap<String, String>>,
}

impl SelfHealingEngine {
    /// Create a new self-healing engine with the given config and memory backend.
    pub fn new(config: SelfHealingConfig, memory: memory::HealingMemory) -> Self {
        Self {
            config,
            memory,
            pending_outcome: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new self-healing engine with a default in-memory backend.
    pub fn new_in_memory(config: SelfHealingConfig) -> Self {
        Self::new(config, memory::HealingMemory::new_in_memory())
    }

    /// Generate a healing hint for a loop that was just detected.
    ///
    /// Returns `None` if self-healing is disabled or no good mutation found.
    /// Returns `Some(hint_text)` with a context-aware suggestion.
    pub fn generate_hint(
        &self,
        context: &HealingContext,
    ) -> Option<String> {
        if !self.config.enabled {
            return None;
        }

        let mutations_ctx = mutations::MutationContext {
            tool_name: &context.tool_name,
            tool_args: &context.tool_args,
            is_error: context.is_error,
            repeat_count: context.repeat_count,
            tilt_index: context.tilt_index,
            session_tool_count: context.session_tool_count,
        };

        // Check memory for past successful strategies first
        let memory_strategy = self.memory.best_strategy(&context.tool_name);

        // Generate fresh mutation candidates
        let best = mutations::pick_best(&mutations_ctx);

        // Prefer remembered strategy if confidence is high enough,
        // otherwise use the freshly generated best
        let mutation = match (memory_strategy, best) {
            (Some((ref label, rate)), Some(ref m)) if rate > 0.7 && m.strategy != label => {
                // Remembered strategy has better track record — use it
                mutations::generate_all(&mutations_ctx)
                    .into_iter()
                    .find(|m| m.strategy == label.as_str())
                    .or_else(|| Some(m.clone()))
            }
            (_, Some(m)) => Some(m),
            (Some((ref label, _)), None) => {
                // No fresh candidates but have memory — build generic hint
                Some(mutations::Mutation {
                    strategy: "remembered",
                    suggestion: format!(
                        "A {} strategy worked before for `{}`. Try a different approach.",
                        label, context.tool_name,
                    ),
                    confidence: 0.5,
                    risk: 0.2,
                })
            }
            (None, None) => return None,
        };

        let result = mutation.map(|m| {
            // Store the strategy so the proxy handler can evaluate
            // the outcome on the next request
            if let Ok(mut store) = self.pending_outcome.lock() {
                store.insert(context.tool_name.clone(), m.strategy.to_string());
            }
            let prefix = match self.config.mode {
                crate::config::HealingMode::Conservative => "Consider an alternative approach: ",
                crate::config::HealingMode::Balanced => "Adapting strategy — ",
                crate::config::HealingMode::Aggressive => "",
            };
            format!("{}{}", prefix, m.suggestion)
        });
        result
    }

    /// Report a mutation outcome back to the memory store.
    pub fn report_outcome(&self, tool: &str, strategy: &str, success: bool) {
        self.memory.record(tool, strategy, success);
    }

    /// The number of successful recoveries.
    pub fn successful_recoveries(&self) -> usize {
        self.memory.successful_recoveries()
    }

    /// Total mutations attempted.
    pub fn total_attempts(&self) -> usize {
        self.memory.total_records()
    }

    /// Drain all pending outcomes that need evaluation on the next request.
    ///
    /// Called by the proxy handler at the start of each request to collect
    /// strategies that were generated during the *previous* request's blocked
    /// tool calls. The caller then evaluates whether those tools are now in
    /// `allowed` vs `blocked` and calls `report_outcome` with the result.
    pub fn drain_pending_outcomes(&self) -> Vec<(String, String)> {
        let mut store = self.pending_outcome.lock().unwrap();
        store.drain().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HealingMode;

    fn engine() -> SelfHealingEngine {
        SelfHealingEngine::new_in_memory(SelfHealingConfig {
            enabled: true,
            mode: HealingMode::Balanced,
            memory_backend: "memory".into(),
            memory_path: None,
        })
    }

    fn disabled_engine() -> SelfHealingEngine {
        SelfHealingEngine::new_in_memory(SelfHealingConfig {
            enabled: false,
            mode: HealingMode::Conservative,
            memory_backend: "memory".into(),
            memory_path: None,
        })
    }

    #[test]
    fn test_disabled_engine_returns_none() {
        let engine = disabled_engine();
        let ctx = HealingContext::new("search", r#"{"limit":100}"#, true, 4, 0.9, 10);
        assert!(engine.generate_hint(&ctx).is_none());
    }

    #[test]
    fn test_generates_hint_for_looping_search() {
        let engine = engine();
        let ctx = HealingContext::new("query_db", r#"{"query":"hello","limit":100}"#, true, 4, 0.8, 10);
        let hint = engine.generate_hint(&ctx);
        assert!(hint.is_some());
        let text = hint.unwrap();
        assert!(text.contains("limit"));
        assert!(text.contains("50")); // 100 / 2
        assert!(text.contains("Adapting strategy"));
    }

    #[test]
    fn test_generates_hint_for_known_tool_substitution() {
        let engine = engine();
        let ctx = HealingContext::new("web_search", r#"{"query":"rust async"}"#, false, 3, 0.5, 8);
        let hint = engine.generate_hint(&ctx);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("browse_page"));
    }

    #[test]
    fn test_report_outcome_and_memory_works() {
        let engine = engine();
        let ctx = HealingContext::new("search", r#"{"limit":50}"#, true, 3, 0.6, 5);

        // First call: generate hint
        assert!(engine.generate_hint(&ctx).is_some());

        // Report success
        engine.report_outcome("search", "parameter_tweak", true);
        assert_eq!(engine.successful_recoveries(), 1);
        assert_eq!(engine.total_attempts(), 1);
    }

    #[test]
    fn test_aggressive_mode_has_no_prefix() {
        let engine = SelfHealingEngine::new_in_memory(SelfHealingConfig {
            enabled: true,
            mode: HealingMode::Aggressive,
            memory_backend: "memory".into(),
            memory_path: None,
        });
        let ctx = HealingContext::new("search", r#"{"limit":50}"#, true, 4, 0.7, 8);
        let hint = engine.generate_hint(&ctx);
        assert!(hint.is_some());
        // Aggressive mode has no prefix like "Adapting strategy"
        assert!(!hint.unwrap().contains("Adapting strategy"));
    }

    #[test]
    fn test_no_hint_for_normal_call() {
        let engine = engine();
        let ctx = HealingContext::new("unknown_tool", r#"{}"#, false, 0, 0.0, 2);
        // No repeating → no error → no tilt → no mutations
        assert!(engine.generate_hint(&ctx).is_none());
    }
}
