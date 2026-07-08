//! Healing memory — records which mutations worked for future reference.
//!
//! Per-tool strategy tracking with optional args_hash + error_signature
//! matching (folded from the removed `learning` module). All data is
//! in-memory; no SQLite or backend trait.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// A recorded mutation outcome.
#[derive(Debug, Clone)]
pub struct MutationRecord {
    /// Tool that was looping.
    pub tool: String,
    /// Brief description of the mutation applied.
    pub strategy_label: String,
    /// Whether the mutation led to a successful outcome.
    pub success: bool,
    /// BLAKE3 hash of (tool_name + canonical_args), for similarity matching.
    pub args_hash: Option<String>,
    /// First line of the error result, for error-signature matching.
    pub error_signature: Option<String>,
    /// When this was recorded.
    pub timestamp: Instant,
}

/// In-memory store of mutation outcomes, keyed by tool name.
#[derive(Debug, Default)]
pub struct HealingMemory {
    inner: Mutex<HashMap<String, Vec<MutationRecord>>>,
}

impl HealingMemory {
    /// Create a new empty healing memory.
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Record the outcome of a mutation attempt.
    pub fn record(
        &self,
        tool: &str,
        strategy: &str,
        success: bool,
        args_hash: Option<String>,
        error_signature: Option<String>,
    ) {
        let mut store = self.inner.lock().unwrap();
        let records = store.entry(tool.to_string()).or_default();
        records.push(MutationRecord {
            tool: tool.to_string(),
            strategy_label: strategy.to_string(),
            success,
            args_hash,
            error_signature,
            timestamp: Instant::now(),
        });
        // Keep only the last 50 per tool (memory bounded)
        if records.len() > 50 {
            records.remove(0);
        }
    }

    /// Record the outcome of a mutation attempt (simple variant without args/error context).
    pub fn record_simple(&self, tool: &str, strategy: &str, success: bool) {
        self.record(tool, strategy, success, None, None);
    }

    /// Get the success rate for a given strategy on a given tool.
    pub fn success_rate(&self, tool: &str, strategy: &str) -> Option<f64> {
        let store = self.inner.lock().unwrap();
        let records = store.get(tool)?;
        let relevant: Vec<_> = records
            .iter()
            .filter(|r| r.strategy_label == strategy)
            .collect();
        if relevant.is_empty() {
            return None;
        }
        let successes = relevant.iter().filter(|r| r.success).count();
        Some(successes as f64 / relevant.len() as f64)
    }

    /// Get the best strategy for a given tool based on past success rate.
    pub fn best_strategy(&self, tool: &str) -> Option<(String, f64)> {
        let store = self.inner.lock().unwrap();
        let records = store.get(tool)?;
        if records.is_empty() {
            return None;
        }
        let mut rates: HashMap<&str, (usize, usize)> = HashMap::new();
        for r in records {
            let entry = rates.entry(&r.strategy_label).or_insert((0, 0));
            entry.0 += 1;
            if r.success {
                entry.1 += 1;
            }
        }
        rates
            .into_iter()
            .map(|(s, (total, success))| (s.to_string(), success as f64 / total as f64))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Total number of recorded outcomes.
    pub fn total_records(&self) -> usize {
        let store = self.inner.lock().unwrap();
        store.values().map(|v| v.len()).sum()
    }

    /// Number of successful recoveries.
    pub fn successful_recoveries(&self) -> usize {
        let store = self.inner.lock().unwrap();
        store.values().flatten().filter(|r| r.success).count()
    }

    /// Suggest the best strategy for a given tool + context.
    ///
    /// Considers args hash similarity (exact match scores highest) and
    /// error-signature similarity, falling back to overall tool success rate.
    pub fn suggest_strategy(
        &self,
        tool: &str,
        args_hash: Option<&str>,
        error_signature: Option<&str>,
    ) -> Option<(String, f64)> {
        let store = self.inner.lock().unwrap();
        let records = store.get(tool)?;
        if records.is_empty() {
            return None;
        }

        // Score each unique strategy based on matching + success rate
        let mut strategies: HashMap<&str, (f64, usize, usize)> = HashMap::new();

        for r in records {
            let score = if let Some(hash) = args_hash {
                if r.args_hash.as_deref() == Some(hash) {
                    1.0 // Exact args match
                } else if let (Some(rec_err), Some(ctx_err)) = (&r.error_signature, error_signature) {
                    if rec_err.as_str() == ctx_err { 0.8 } else { 0.3 }
                } else {
                    0.3
                }
            } else {
                0.3
            };

            let entry = strategies.entry(&r.strategy_label).or_insert((0.0, 0, 0));
            // Boost score for this strategy if this record contributed positively
            if score > entry.0 {
                entry.0 = score;
            }
            entry.1 += 1;          // total attempts
            if r.success {
                entry.2 += 1;      // successes
            }
        }

        // Score = match_score * success_rate
        strategies
            .into_iter()
            .map(|(label, (match_score, total, successes))| {
                let rate = if total > 0 { successes as f64 / total as f64 } else { 0.0 };
                (label.to_string(), match_score * rate)
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .filter(|(_, score)| *score > 0.0)
    }

    /// Return per-tool stats summary (for admin API).
    pub fn stats(&self) -> Vec<serde_json::Value> {
        let store = self.inner.lock().unwrap();
        store.iter().map(|(tool, records)| {
            let total = records.len();
            let successes = records.iter().filter(|r| r.success).count();
            let rate = if total > 0 { successes as f64 / total as f64 } else { 0.0 };
            let best = self.best_strategy(tool);
            serde_json::json!({
                "tool_name": tool,
                "total_attempts": total,
                "successes": successes,
                "success_rate": rate,
                "best_strategy": best.map(|(s, _)| s),
            })
        }).collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_success_rate() {
        let mem = HealingMemory::new();
        assert_eq!(mem.total_records(), 0);

        mem.record_simple("search", "parameter_tweak", true);
        mem.record_simple("search", "parameter_tweak", true);
        mem.record_simple("search", "parameter_tweak", false);

        let rate = mem.success_rate("search", "parameter_tweak");
        assert!((rate.unwrap() - 2.0 / 3.0).abs() < 0.01);

        assert_eq!(mem.total_records(), 3);
        assert_eq!(mem.successful_recoveries(), 2);
    }

    #[test]
    fn test_best_strategy() {
        let mem = HealingMemory::new();
        mem.record_simple("list_files", "tool_substitution", true);
        mem.record_simple("list_files", "tool_substitution", true);
        mem.record_simple("list_files", "parameter_tweak", false);

        let best = mem.best_strategy("list_files");
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "tool_substitution");
    }

    #[test]
    fn test_empty_memory() {
        let mem = HealingMemory::new();
        assert!(mem.best_strategy("unknown").is_none());
        assert!(mem.success_rate("unknown", "anything").is_none());
    }

    #[test]
    fn test_suggest_strategy_with_args_hash() {
        let mem = HealingMemory::new();
        let hash = "abc123".to_string();

        // Record a successful parameter_tweak with specific hash
        mem.record("search", "parameter_tweak", true, Some(hash.clone()), Some("timeout".into()));
        mem.record("search", "parameter_tweak", true, Some(hash.clone()), Some("timeout".into()));
        mem.record("search", "tool_substitution", false, Some(hash.clone()), Some("timeout".into()));

        // Same context should suggest the successful strategy
        let suggestion = mem.suggest_strategy("search", Some(&hash), Some("timeout"));
        assert!(suggestion.is_some());
        let (strategy, _score) = suggestion.unwrap();
        assert_eq!(strategy, "parameter_tweak");
    }

    #[test]
    fn test_suggest_returns_none_for_unknown() {
        let mem = HealingMemory::new();
        let suggestion = mem.suggest_strategy("unknown_tool", None, None);
        assert!(suggestion.is_none());
    }

    #[test]
    fn test_stats_empty() {
        let mem = HealingMemory::new();
        let stats = mem.stats();
        assert!(stats.is_empty());
    }

    #[test]
    fn test_stats_with_data() {
        let mem = HealingMemory::new();
        mem.record_simple("search", "parameter_tweak", true);
        mem.record_simple("list_files", "tool_substitution", true);

        let stats = mem.stats();
        assert_eq!(stats.len(), 2);
    }

    #[test]
    fn test_memory_bounded() {
        let mem = HealingMemory::new();
        for _ in 0..100 {
            mem.record_simple("test_tool", "strategy", true);
        }
        assert!(mem.total_records() <= 50, "should be bounded to 50 records per tool");
    }
}
