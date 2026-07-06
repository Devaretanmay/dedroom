//! Adaptive thresholding for loop detection.
//!
//! Tightens `max_repeats` based on error rate: `max_repeats - errors * reduction`,
//! floored at `min_repeats`. Per-tool state.

use std::collections::HashMap;

#[derive(Debug)]
pub struct AdaptiveThreshold {
    enabled: bool,
    base_max_repeats: u32,
    error_reduction: u32,
    min_repeats: u32,
    error_counts: HashMap<String, u32>,
    effective: HashMap<String, u32>,
}

impl AdaptiveThreshold {
    pub fn new(enabled: bool, base_max_repeats: u32, error_reduction: u32, min_repeats: u32) -> Self {
        Self {
            enabled,
            base_max_repeats,
            error_reduction: error_reduction.max(1),
            min_repeats: min_repeats.max(1),
            error_counts: HashMap::new(),
            effective: HashMap::new(),
        }
    }

    pub fn record_error(&mut self, tool: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let err_count = self.error_counts.entry(tool.to_string()).or_insert(0);
        *err_count += 1;
        let reduction = self.error_reduction * *err_count;
        let effective = self.base_max_repeats.saturating_sub(reduction).max(self.min_repeats);
        self.effective.insert(tool.to_string(), effective);
        true
    }

    pub fn record_success(&mut self, tool: &str) -> bool {
        if !self.enabled {
            return false;
        }
        self.error_counts.remove(tool);
        self.effective.remove(tool);
        true
    }

    pub fn effective_max_repeats(&self) -> u32 {
        self.base_max_repeats
    }

    pub fn effective_for_tool(&self, tool: &str) -> u32 {
        self.effective.get(tool).copied().unwrap_or(self.base_max_repeats)
    }

    pub fn reset(&mut self) {
        self.error_counts.clear();
        self.effective.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_errors_reduce_threshold() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 4);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 2);
    }

    #[test]
    fn test_success_restores() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);
        at.record_error("write_file");
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 3);
        at.record_success("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 5);
    }

    #[test]
    fn test_min_repeats_floor() {
        let mut at = AdaptiveThreshold::new(true, 3, 2, 1);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 1);
    }

    #[test]
    fn test_disabled() {
        let mut at = AdaptiveThreshold::new(false, 5, 1, 2);
        at.record_error("write_file");
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
    fn test_independent_tools() {
        let mut at = AdaptiveThreshold::new(true, 5, 1, 2);
        at.record_error("write_file");
        assert_eq!(at.effective_for_tool("write_file"), 4);
        assert_eq!(at.effective_for_tool("read_file"), 5);
    }
}
