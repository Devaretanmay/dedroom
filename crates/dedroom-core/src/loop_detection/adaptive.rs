//! Adaptive thresholding for loop detection.
//!
//! Dynamically tightens the loop detection threshold based on error rates
//! in the tool-call trajectory. When the agent is in an error loop, we
//! force it to pivot faster by reducing `max_repeats`.

use std::collections::HashMap;

/// Tracks per-tool error rates and adjusts the effective `max_repeats`
/// threshold accordingly.
#[derive(Debug)]
pub struct AdaptiveThreshold {
    enabled: bool,
    base_max_repeats: u32,
    error_reduction: u32,
    min_repeats: u32,

    /// Per-tool: recent consecutive error count.
    error_counts: HashMap<String, u32>,
    /// Per-tool: recent consecutive success count.
    success_counts: HashMap<String, u32>,

    /// Current effective max_repeats (per-tool).
    effective: HashMap<String, u32>,
}

impl AdaptiveThreshold {
    pub fn new(
        enabled: bool,
        base_max_repeats: u32,
        error_reduction: u32,
        min_repeats: u32,
    ) -> Self {
        Self {
            enabled,
            base_max_repeats,
            error_reduction: error_reduction.max(1),
            min_repeats: min_repeats.max(1),
            error_counts: HashMap::new(),
            success_counts: HashMap::new(),
            effective: HashMap::new(),
        }
    }

    /// Record an error for a tool. Returns `true` if the effective threshold changed.
    pub fn record_error(&mut self, tool: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let err_count = self.error_counts.entry(tool.to_string()).or_insert(0);
        *err_count += 1;
        self.success_counts.remove(tool);

        let reduction = self.error_reduction * *err_count;
        let effective = self.base_max_repeats.saturating_sub(reduction).max(self.min_repeats);
        self.effective.insert(tool.to_string(), effective);

        tracing::debug!(
            "adaptive: error on tool={}, consecutive_errors={}, effective_max={}",
            tool, err_count, effective,
        );
        true
    }

    /// Record a success for a tool (resets error count).
    /// Returns `true` if the effective threshold changed (i.e., count reached 3 and was restored).
    pub fn record_success(&mut self, tool: &str) -> bool {
        if !self.enabled {
            return false;
        }
        self.error_counts.remove(tool);
        let success_count = self.success_counts.entry(tool.to_string()).or_insert(0);
        *success_count += 1;

        // After 3 consecutive non-error calls, restore base threshold
        if *success_count >= 3 {
            self.effective.remove(tool);
            self.success_counts.remove(tool);
            return true;
        }
        false
    }

    /// Get the effective max_repeats for a tool.
    /// Falls back to `base_max_repeats` if no adaptive adjustment is active.
    pub fn effective_max_repeats(&self) -> u32 {
        // When called without a specific tool, return the base
        self.base_max_repeats
    }

    /// Get the effective max_repeats for a specific tool.
    pub fn effective_for_tool(&self, tool: &str) -> u32 {
        self.effective.get(tool).copied().unwrap_or(self.base_max_repeats)
    }

    /// Get the error count for a tool (for persistence).
    pub fn error_count(&self, tool: &str) -> u32 {
        self.error_counts.get(tool).copied().unwrap_or(0)
    }

    /// Get the success count for a tool (for persistence).
    pub fn success_count(&self, tool: &str) -> u32 {
        self.success_counts.get(tool).copied().unwrap_or(0)
    }

    /// Reset all adaptive state.
    pub fn reset(&mut self) {
        self.error_counts.clear();
        self.success_counts.clear();
        self.effective.clear();
    }

    /// Export all per-tool counts for periodic batch persistence.
    pub fn export_state(&self) -> Vec<(String, u32, u32)> {
        let mut result = Vec::new();
        for (tool, &err) in &self.error_counts {
            let succ = self.success_counts.get(tool).copied().unwrap_or(0);
            result.push((tool.clone(), err, succ));
        }
        for (tool, &succ) in &self.success_counts {
            if !self.error_counts.contains_key(tool) {
                result.push((tool.clone(), 0, succ));
            }
        }
        result
    }

    /// Import persisted per-tool counts and recompute effectiveness.
    pub fn import_state(&mut self, entries: &[(String, u32, u32)]) {
        for (tool, err_count, succ_count) in entries {
            if *err_count > 0 {
                self.error_counts.insert(tool.clone(), *err_count);
                // Recompute effective
                let reduction = self.error_reduction * err_count;
                let effective = self.base_max_repeats.saturating_sub(reduction).max(self.min_repeats);
                self.effective.insert(tool.clone(), effective);
            }
            if *succ_count > 0 {
                self.success_counts.insert(tool.clone(), *succ_count);
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_errors_reduce_threshold() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);

        // 3 consecutive errors should reduce effective to 5 - 3 = 2, floor 2
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 4);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 2);
    }

    #[test]
    fn test_successes_restore_threshold() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);

        at.record_error("write_file");
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);

        // 3 consecutive successes should restore
        at.record_success("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);
        at.record_success("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);
        at.record_success("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 5);
    }

    #[test]
    fn test_min_repeats_floor() {
        let mut at = AdaptiveThreshold::new(true, 3, 2, 1);

        at.record_error("write_file");
        // 3 - 2 = 1
        assert_eq!(at.effective_for_tool("write_file"), 1);
    }

    #[test]
    fn test_disabled_no_effect() {
        let mut at = AdaptiveThreshold::new(false, 5, 1, 2);

        at.record_error("write_file");
        at.record_error("write_file");
        // Should still be base
        assert_eq!(at.effective_for_tool("write_file"), 5);
    }

    #[test]
    fn test_reset() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);

        at.record_error("write_file");
        at.reset();
        assert_eq!(at.effective_for_tool("write_file"), 5);
    }

    #[test]
    fn test_different_tools_independent() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);

        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 4);
        assert_eq!(at.effective_for_tool("read_file"), 5);
    }
}
