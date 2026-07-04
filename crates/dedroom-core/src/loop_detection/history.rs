//! Tracks tool call history for loop detection.
//!
//! Stores a bounded sliding window of `(tool, canonical_args, was_error)` tuples.
//! History is automatically pruned when it exceeds the configured window size
//! (fixes the unbounded-growth problem).

use crate::config::CountMode;

/// A single entry in the call history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub tool: String,
    /// Canonicalized arguments (volatile fields stripped).
    pub canonical_args: String,
    /// Whether the tool call resulted in an error.
    pub was_error: bool,
}

/// Bounded, sliding-window history of tool calls.
#[derive(Debug)]
pub struct HistoryTracker {
    /// Maximum number of entries to keep.
    window: usize,
    /// Circular buffer of entries (newest at end).
    entries: Vec<HistoryEntry>,
}

impl HistoryTracker {
    /// Create a new tracker with the given window size.
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            entries: Vec::with_capacity(window),
        }
    }

    /// Push a new tool call into history.
    pub fn push(&mut self, tool: String, canonical_args: String, was_error: bool) {
        if self.entries.len() >= self.window {
            self.entries.remove(0);
        }
        self.entries.push(HistoryEntry {
            tool,
            canonical_args,
            was_error,
        });
    }

    /// Count how many times the given tool+args pair appears in history
    /// (up to and including the window size).
    pub fn count_repeats(
        &self,
        tool: &str,
        canonical_args: &str,
        mode: CountMode,
    ) -> u32 {
        self.entries.iter().rev().fold(0u32, |count, entry| {
            if entry.tool == tool && entry.canonical_args == canonical_args {
                match mode {
                    CountMode::All => count + 1,
                    CountMode::ErrorsOnly => {
                        if entry.was_error {
                            count + 1
                        } else {
                            count
                        }
                    }
                }
            } else {
                count
            }
        })
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries (oldest first).
    pub fn iter(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Resize the window. Truncates oldest entries if shrinking.
    pub fn set_window(&mut self, new_window: usize) {
        self.window = new_window.max(1);
        while self.entries.len() > self.window {
            self.entries.remove(0);
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_count() {
        let mut h = HistoryTracker::new(10);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 2);
    }

    #[test]
    fn test_window_eviction() {
        let mut h = HistoryTracker::new(3);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false);
        h.push("a".into(), "1".into(), false); // evicts the first
        assert_eq!(h.count_repeats("a", "1", CountMode::All), 3);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn test_count_errors_only() {
        let mut h = HistoryTracker::new(10);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), true);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::ErrorsOnly), 2);
    }

    #[test]
    fn test_different_tools_independent() {
        let mut h = HistoryTracker::new(10);
        h.push("read_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        h.push("write_file".into(), r#"{"path":"/tmp/x.txt"}"#.into(), false);
        assert_eq!(h.count_repeats("write_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
        assert_eq!(h.count_repeats("read_file", r#"{"path":"/tmp/x.txt"}"#, CountMode::All), 1);
    }

    #[test]
    fn test_clear() {
        let mut h = HistoryTracker::new(10);
        h.push("a".into(), "1".into(), false);
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn test_set_window_shrinks() {
        let mut h = HistoryTracker::new(10);
        for _ in 0..5 {
            h.push("a".into(), "1".into(), false);
        }
        h.set_window(2);
        assert_eq!(h.len(), 2);
    }
}
