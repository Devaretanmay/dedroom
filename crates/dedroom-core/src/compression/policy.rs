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
    Recovering,
}

/// How aggressively to compress content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    Normal,
    Moderate,
    Aggressive,
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

// ── Pure functions ─────────────────────────────────────────────────────────

pub fn determine_level(state: LoopState, coupling: &LoopCompressionCoupling) -> CompressionLevel {
    if !coupling.enabled {
        return CompressionLevel::Normal;
    }
    match state {
        LoopState::None => CompressionLevel::Normal,
        LoopState::Detected => coupling.on_detected.compression_budget.into(),
        LoopState::ErrorLoop => coupling.on_error_loop.compression_budget.into(),
        LoopState::Recovering => coupling.on_recovery.compression_budget.into(),
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

pub fn fresh_context_window(coupling: &LoopCompressionCoupling) -> usize {
    coupling.on_recovery.fresh_context_window
}

pub fn retention_for_level(level: CompressionLevel) -> f64 {
    match level {
        CompressionLevel::Normal => 0.3,
        CompressionLevel::Moderate => 0.2,
        CompressionLevel::Aggressive => 0.1,
        CompressionLevel::Maximum => 0.05,
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
                compression_budget: crate::config::CompressionBudget::Aggressive,
                inject_hint: false,
                hint_template: None,
            },
            on_error_loop: crate::config::LoopCouplingAction {
                compression_budget: crate::config::CompressionBudget::Maximum,
                inject_hint: true,
                hint_template: Some("You are looping on '{tool}'. Try a completely different approach.".into()),
            },
            on_recovery: crate::config::RecoveryCouplingAction {
                compression_budget: crate::config::CompressionBudget::Moderate,
                fresh_context_window: 3,
            },
        }
    }

    #[test]
    fn test_normal_state_returns_normal_level() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::None, &coupling), CompressionLevel::Normal);
        assert!(!should_inject_hint(LoopState::None, &coupling));
        assert!(!skip_identical_content(LoopState::None));
    }

    #[test]
    fn test_detected_triggers_aggressive() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::Detected, &coupling), CompressionLevel::Aggressive);
    }

    #[test]
    fn test_error_loop_triggers_maximum() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::ErrorLoop, &coupling), CompressionLevel::Maximum);
    }

    #[test]
    fn test_recovery_is_moderate() {
        let coupling = default_coupling();
        assert_eq!(determine_level(LoopState::Recovering, &coupling), CompressionLevel::Moderate);
    }

    #[test]
    fn test_disabled_coupling_returns_normal() {
        let mut coupling = default_coupling();
        coupling.enabled = false;
        assert_eq!(determine_level(LoopState::ErrorLoop, &coupling), CompressionLevel::Normal);
    }

    #[test]
    fn test_error_loop_injects_hint() {
        let coupling = default_coupling();
        assert!(should_inject_hint(LoopState::ErrorLoop, &coupling));
        assert!(hint_template(LoopState::ErrorLoop, &coupling).is_some());
    }

    #[test]
    fn test_compression_level_from_budget() {
        let lv: CompressionLevel = CompressionBudget::Normal.into();
        assert_eq!(lv, CompressionLevel::Normal);
        let lv: CompressionLevel = CompressionBudget::Maximum.into();
        assert_eq!(lv, CompressionLevel::Maximum);
    }

    #[test]
    fn test_retention_for_level() {
        assert!(retention_for_level(CompressionLevel::Normal) > retention_for_level(CompressionLevel::Aggressive));
    }
}
