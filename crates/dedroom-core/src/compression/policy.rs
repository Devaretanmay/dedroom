//! Compression policy — pure functions keyed on loop state.
//!
//! All logic is stateless: pass `LoopState` and the coupling config
//! to get a decision. No struct, no mutable state to carry around.

use crate::config::{CompressionBudget, LoopCompressionCoupling};

/// Loop state as seen by the compression policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopState {
    None,
    Detected,
    ErrorLoop,
}

// ── Pure functions ─────────────────────────────────────────────────────────

pub fn determine_level(state: LoopState, coupling: &LoopCompressionCoupling) -> CompressionBudget {
    if !coupling.enabled {
        return CompressionBudget::Normal;
    }
    match state {
        LoopState::None => CompressionBudget::Normal,
        LoopState::Detected => coupling.on_detected.compression_budget,
        LoopState::ErrorLoop => coupling.on_error_loop.compression_budget,
    }
}

pub fn should_inject_hint(state: LoopState, coupling: &LoopCompressionCoupling) -> bool {
    match state {
        LoopState::ErrorLoop => coupling.on_error_loop.inject_hint,
        LoopState::Detected => coupling.on_detected.inject_hint,
        _ => false,
    }
}

pub fn hint_template(state: LoopState, coupling: &LoopCompressionCoupling) -> Option<&str> {
    match state {
        LoopState::ErrorLoop => coupling.on_error_loop.hint_template.as_deref(),
        LoopState::Detected => coupling.on_detected.hint_template.as_deref(),
        _ => None,
    }
}

pub fn retention_for_level(budget: CompressionBudget) -> f64 {
    match budget {
        CompressionBudget::Normal => 0.3,
        CompressionBudget::Moderate => 0.2,
        CompressionBudget::Aggressive => 0.1,
        CompressionBudget::Maximum => 0.05,
    }
}

pub fn skip_identical_content(state: LoopState) -> bool {
    matches!(state, LoopState::Detected | LoopState::ErrorLoop)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_coupling() -> LoopCompressionCoupling {
        LoopCompressionCoupling {
            enabled: true,
            on_detected: crate::config::LoopCouplingAction {
                compression_budget: CompressionBudget::Aggressive,
                inject_hint: false,
                hint_template: None,
            },
            on_error_loop: crate::config::LoopCouplingAction {
                compression_budget: CompressionBudget::Maximum,
                inject_hint: true,
                hint_template: Some("You are looping on '{tool}'. Try a completely different approach.".into()),
            },
        }
    }

    #[test]
    fn test_normal_state_returns_normal_level() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::None, &coupling), CompressionBudget::Normal);
        assert!(!should_inject_hint(LoopState::None, &coupling));
        assert!(!skip_identical_content(LoopState::None));
    }

    #[test]
    fn test_detected_triggers_aggressive() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::Detected, &coupling), CompressionBudget::Aggressive);
    }

    #[test]
    fn test_error_loop_triggers_maximum() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::ErrorLoop, &coupling), CompressionBudget::Maximum);
    }

    #[test]
    fn test_disabled_coupling_returns_normal() {
        let mut coupling = default_coupling();
        coupling.enabled = false;
        assert_eq!(determine_level(LoopState::ErrorLoop, &coupling), CompressionBudget::Normal);
    }

    #[test]
    fn test_error_loop_injects_hint() {
        let coupling = default_coupling();
        assert!(should_inject_hint(LoopState::ErrorLoop, &coupling));
        assert!(hint_template(LoopState::ErrorLoop, &coupling).is_some());
    }

    #[test]
    fn test_retention_for_level() {
        assert!(retention_for_level(CompressionBudget::Normal) > retention_for_level(CompressionBudget::Aggressive));
    }
}
