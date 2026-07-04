//! Compression policy — adjusts behaviour based on loop state.

use crate::config::{CompressionBudget, LoopCompressionCoupling};

/// Loop state as seen by the compression policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopState {
    /// Agent is making progress — normal compression.
    None,
    /// Same tool called repeatedly — may be looping.
    Detected,
    /// Same tool producing errors — definitely looping with errors.
    ErrorLoop,
    /// Agent just pivoted to a new approach — give it fresh context.
    Recovering,
}

/// How aggressively to compress content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// Standard compression (default).
    Normal,
    /// Slightly more aggressive than normal.
    Moderate,
    /// Compress heavily — agent may be looping, tokens are at risk.
    Aggressive,
    /// Compress everything possible — error loop, save every token.
    Maximum,
}

impl From<CompressionBudget> for CompressionLevel {
    fn from(budget: CompressionBudget) -> Self {
        match budget {
            CompressionBudget::Normal => Self::Normal,
            CompressionBudget::Moderate => Self::Moderate,
            CompressionBudget::Aggressive => Self::Aggressive,
            CompressionBudget::Maximum => Self::Maximum,
        }
    }
}

/// Determines compression behaviour based on loop state and coupling config.
#[derive(Debug, Clone)]
pub struct CompressionPolicy {
    loop_state: LoopState,
    coupling: LoopCompressionCoupling,
    /// Number of recent messages to leave uncompressed in recovery mode.
    fresh_context_window: usize,
}

impl CompressionPolicy {
    /// Create a new policy with default state (no loop detected).
    pub fn new(coupling: &LoopCompressionCoupling) -> Self {
        Self {
            loop_state: LoopState::None,
            coupling: coupling.clone(),
            fresh_context_window: coupling.on_recovery.fresh_context_window,
        }
    }

    /// Update the current loop state.
    pub fn set_loop_state(&mut self, state: LoopState) {
        self.loop_state = state;
    }

    /// Current loop state.
    pub fn loop_state(&self) -> LoopState {
        self.loop_state
    }

    /// Get the compression level for the current state.
    pub fn compression_level(&self) -> CompressionLevel {
        if !self.coupling.enabled {
            return CompressionLevel::Normal;
        }
        match self.loop_state {
            LoopState::None => CompressionLevel::Normal,
            LoopState::Detected => {
                self.coupling.on_detected.compression_budget.into()
            }
            LoopState::ErrorLoop => {
                self.coupling.on_error_loop.compression_budget.into()
            }
            LoopState::Recovering => {
                self.coupling.on_recovery.compression_budget.into()
            }
        }
    }

    /// Whether to inject a recovery hint into the system prompt.
    pub fn should_inject_hint(&self) -> bool {
        match self.loop_state {
            LoopState::ErrorLoop => self.coupling.on_error_loop.inject_hint,
            LoopState::Detected => self.coupling.on_detected.inject_hint,
            _ => false,
        }
    }

    /// Get the recovery hint template, if any.
    pub fn hint_template(&self) -> Option<&str> {
        match self.loop_state {
            LoopState::ErrorLoop => {
                self.coupling.on_error_loop.hint_template.as_deref()
            }
            LoopState::Detected => {
                self.coupling.on_detected.hint_template.as_deref()
            }
            _ => None,
        }
    }

    /// How many recent messages to leave uncompressed (recovery mode).
    pub fn fresh_context_window(&self) -> usize {
        self.fresh_context_window
    }

    /// SmartCrusher retention parameter based on compression level.
    pub fn smart_crusher_retention(&self) -> f64 {
        match self.compression_level() {
            CompressionLevel::Normal => 0.3,    // keep ~30% rows
            CompressionLevel::Moderate => 0.2,
            CompressionLevel::Aggressive => 0.1,
            CompressionLevel::Maximum => 0.05,
        }
    }

    /// Whether to skip compression of already-seen content (loop optimization).
    pub fn skip_identical_content(&self) -> bool {
        matches!(self.loop_state, LoopState::Detected | LoopState::ErrorLoop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_coupling() -> LoopCompressionCoupling {
        LoopCompressionCoupling::default()
    }

    #[test]
    fn test_normal_state_no_special_compression() {
        let coupling = default_coupling();
        let mut policy = CompressionPolicy::new(&coupling);
        assert_eq!(policy.compression_level(), CompressionLevel::Normal);
        assert!(!policy.should_inject_hint());
        assert!(!policy.skip_identical_content());
    }

    #[test]
    fn test_detected_triggers_aggressive() {
        let coupling = default_coupling();
        let mut policy = CompressionPolicy::new(&coupling);
        policy.set_loop_state(LoopState::Detected);
        assert_eq!(policy.compression_level(), CompressionLevel::Aggressive);
    }

    #[test]
    fn test_error_loop_triggers_maximum() {
        let coupling = default_coupling();
        let mut policy = CompressionPolicy::new(&coupling);
        policy.set_loop_state(LoopState::ErrorLoop);
        assert_eq!(policy.compression_level(), CompressionLevel::Maximum);
    }

    #[test]
    fn test_recovery_is_moderate() {
        let coupling = default_coupling();
        let mut policy = CompressionPolicy::new(&coupling);
        policy.set_loop_state(LoopState::Recovering);
        assert_eq!(policy.compression_level(), CompressionLevel::Moderate);
    }

    #[test]
    fn test_disabled_coupling() {
        let mut coupling = default_coupling();
        coupling.enabled = false;
        let mut policy = CompressionPolicy::new(&coupling);
        policy.set_loop_state(LoopState::ErrorLoop);
        assert_eq!(policy.compression_level(), CompressionLevel::Normal);
    }

    #[test]
    fn test_error_loop_injects_hint() {
        let coupling = default_coupling();
        let mut policy = CompressionPolicy::new(&coupling);
        policy.set_loop_state(LoopState::ErrorLoop);
        assert!(policy.should_inject_hint());
        assert!(policy.hint_template().is_some());
    }

    #[test]
    fn test_compression_level_from_budget() {
        let lv: CompressionLevel = CompressionBudget::Normal.into();
        assert_eq!(lv, CompressionLevel::Normal);
        let lv: CompressionLevel = CompressionBudget::Maximum.into();
        assert_eq!(lv, CompressionLevel::Maximum);
    }
}
