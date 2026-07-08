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
pub mod instincts;

use std::collections::HashMap;
use std::sync::Mutex;
use crate::ccr::hash_tool_call;
use crate::config::SelfHealingConfig;

/// A healing context built from the current pipeline state.
#[derive(Debug, Clone)]
pub struct HealingContext {
    pub tool_name: String,
    pub tool_args: String,
    pub is_error: bool,
    /// The actual error result text (e.g. "permission denied"), if available.
    pub error_result: Option<String>,
    pub repeat_count: u32,
    pub tilt_index: f64,
    pub session_tool_count: usize,
}

impl HealingContext {
    pub fn new(
        tool_name: &str,
        tool_args: &str,
        is_error: bool,
        error_result: Option<String>,
        repeat_count: u32,
        tilt_index: f64,
        session_tool_count: usize,
    ) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_args: tool_args.to_string(),
            is_error,
            error_result,
            repeat_count,
            tilt_index,
            session_tool_count,
        }
    }
}

/// The self-healing engine orchestrates mutation generation and scoring.
///
/// Three-tier priority:
/// 1. [`instincts::InstinctsEngine`] — config-loaded rules (highest authority)
/// 2. [`memory::HealingMemory`] — per-tool strategy tracking with args_hash matching
/// 3. Fresh mutation candidates
#[derive(Debug)]
pub struct SelfHealingEngine {
    config: SelfHealingConfig,
    pub memory: memory::HealingMemory,
    /// Instincts engine — config-loaded rules checked first.
    pub instincts: instincts::InstinctsEngine,
    /// Pending outcomes from `generate_hint` that need to be evaluated
    /// on the next request to determine if the mutation broke the loop.
    /// Maps tool_name → strategy_label.
    pending_outcome: Mutex<HashMap<String, String>>,
}

impl SelfHealingEngine {
    /// Create a new self-healing engine.
    pub fn new(
        config: SelfHealingConfig,
        memory: memory::HealingMemory,
        instincts: instincts::InstinctsEngine,
    ) -> Self {
        Self {
            config,
            memory,
            instincts,
            pending_outcome: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new self-healing engine with default in-memory backend and instincts.
    pub fn new_in_memory(config: SelfHealingConfig) -> Self {
        Self::new(
            config,
            memory::HealingMemory::new(),
            instincts::InstinctsEngine::default(),
        )
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

        // Step 1: Check instincts FIRST (config-loaded rules, highest authority)
        if let Some(hint) = self.instincts.check_instincts(
            &context.tool_name,
            &context.tool_args,
            context.is_error,
            context.repeat_count,
        ) {
            if let Ok(mut store) = self.pending_outcome.lock() {
                store.insert(context.tool_name.clone(), "instinct".to_string());
            }
            return Some(hint);
        }

        // Step 2: Check healing memory for past successful strategies (with args_hash matching)
        let args_hash = hash_tool_call(&context.tool_name, &context.tool_args).to_hex().to_string();
        let error_sig = if context.is_error { context.error_result.as_deref() } else { None };
        let memory_suggestion = self.memory.suggest_strategy(
            &context.tool_name,
            Some(&args_hash),
            error_sig,
        );

        // Step 3: Generate fresh mutation candidates
        let best = mutations::pick_best(&mutations_ctx);

        // Use memory suggestion if confidence is high enough
        if let Some((ref strategy, confidence)) = memory_suggestion {
            if confidence >= 0.6 {
                let hint = format!(
                    "{} [learned from past sessions] {}",
                    match self.config.mode {
                        crate::config::HealingMode::Conservative => "Consider: ",
                        crate::config::HealingMode::Balanced => "Adapting strategy — ",
                        crate::config::HealingMode::Aggressive => "",
                    },
                    strategy,
                );
                if let Ok(mut store) = self.pending_outcome.lock() {
                    store.insert(context.tool_name.clone(), strategy.clone());
                }
                return Some(hint);
            }
        }

        // Prefer remembered strategy if high confidence, otherwise fresh mutation
        let memory_strategy = self.memory.best_strategy(&context.tool_name);
        let mutation = match (memory_strategy, best) {
            (Some((ref label, rate)), Some(ref m)) if rate > 0.7 && m.strategy != label => {
                mutations::generate_all(&mutations_ctx)
                    .into_iter()
                    .find(|m| m.strategy == label.as_str())
                    .or_else(|| Some(m.clone()))
            }
            (_, Some(m)) => Some(m),
            (Some((ref label, _)), None) => {
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

    /// Get a reference to the instincts engine.
    pub fn instincts_engine(&self) -> &instincts::InstinctsEngine {
        &self.instincts
    }

    /// Report a mutation outcome back to the memory store.
    pub fn report_outcome(&self, tool: &str, strategy: &str, success: bool) {
        self.memory.record_simple(tool, strategy, success);
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

    fn default_config() -> SelfHealingConfig {
        SelfHealingConfig {
            enabled: true,
            mode: HealingMode::Balanced,
            ..Default::default()
        }
    }

    fn engine() -> SelfHealingEngine {
        SelfHealingEngine::new_in_memory(default_config())
    }

    fn disabled_engine() -> SelfHealingEngine {
        SelfHealingEngine::new_in_memory(SelfHealingConfig {
            enabled: false,
            ..Default::default()
        })
    }

    #[test]
    fn test_disabled_engine_returns_none() {
        let engine = disabled_engine();
        let ctx = HealingContext::new("search", r#"{"limit":100}"#, true, None, 4, 0.9, 10);
        assert!(engine.generate_hint(&ctx).is_none());
    }

    #[test]
    fn test_generates_hint_for_looping_search() {
        let engine = engine();
        let ctx = HealingContext::new(
            "query_db", r#"{"query":"hello","limit":100}"#, true,
            Some("timeout error".to_string()), 4, 0.8, 10,
        );
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
        let ctx = HealingContext::new("web_search", r#"{"query":"rust async"}"#, false, None, 3, 0.5, 8);
        let hint = engine.generate_hint(&ctx);
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("browse_page"));
    }

    #[test]
    fn test_report_outcome_and_memory_works() {
        let engine = engine();
        let ctx = HealingContext::new("search", r#"{"limit":50}"#, true, Some("error".to_string()), 3, 0.6, 5);

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
            ..Default::default()
        });
        let ctx = HealingContext::new("search", r#"{"limit":50}"#, true, None, 4, 0.7, 8);
        let hint = engine.generate_hint(&ctx);
        assert!(hint.is_some());
        assert!(!hint.unwrap().contains("Adapting strategy"));
    }

    #[test]
    fn test_no_hint_for_normal_call() {
        let engine = engine();
        let ctx = HealingContext::new("unknown_tool", r#"{}"#, false, None, 0, 0.0, 2);
        assert!(engine.generate_hint(&ctx).is_none());
    }
}
