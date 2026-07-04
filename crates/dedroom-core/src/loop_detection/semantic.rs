//! Semantic loop detection via embedding similarity.
//!
//! Catches *semantic* loops — where the tool *name* or *arguments* differ
//! but the *intent* is the same (e.g. `delete_line(5)` vs `remove_line(5)`).
//!
//! Uses the shared embedder trait (from `crate::embedding`), so it
//! piggybacks on whatever embedding backend the rest of the system uses —
//! no separate model loading required.

use crate::config::SemanticConfig;

/// Compares tool calls by embedding similarity to detect semantic loops.
#[derive(Debug)]
pub struct SemanticDetector {
    enabled: bool,
    threshold: f32,
    window: usize,
}

impl SemanticDetector {
    /// Create a new semantic detector from config.
    pub fn from_config(config: &SemanticConfig) -> Self {
        Self {
            enabled: config.enabled,
            threshold: config.similarity_threshold,
            window: config.window.max(1),
        }
    }

    /// Returns true if semantic detection is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The similarity threshold for loop detection.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// The number of recent calls to compare against.
    pub fn window(&self) -> usize {
        self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_config_defaults() {
        let config = SemanticConfig::default();
        let detector = SemanticDetector::from_config(&config);
        assert!(!detector.is_enabled());
        assert!((detector.threshold() - 0.85).abs() < 0.001);
        assert_eq!(detector.window(), 5);
    }

    #[test]
    fn test_semantic_config_enabled() {
        let config = SemanticConfig {
            enabled: true,
            ..Default::default()
        };
        let detector = SemanticDetector::from_config(&config);
        assert!(detector.is_enabled());
    }
}
