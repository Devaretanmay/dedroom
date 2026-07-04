//! Unified ledger tracking compression savings and loop prevention.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A compression event.
#[derive(Debug, Clone)]
pub struct CompressionSaving {
    pub original_tokens: u64,
    pub compressed_tokens: u64,
    pub content_type: String,
}

/// A loop-blocked event.
#[derive(Debug, Clone)]
pub struct LoopBlockSaving {
    pub tool_name: String,
    pub calls_prevented: u64,
    pub estimated_tokens_saved: u64,
}

/// A loop early-intervention event.
#[derive(Debug, Clone)]
pub struct LoopEarlyIntervention {
    pub tool_name: String,
    pub calls_before_block: u64,
    pub strategy: String,
}

/// A recorded saving event.
#[derive(Debug, Clone)]
pub enum SavingEvent {
    Compression(CompressionSaving),
    LoopBlocked(LoopBlockSaving),
    LoopEarlyIntervention(LoopEarlyIntervention),
}

/// A snapshot savings report.
#[derive(Debug, Clone, Default)]
pub struct SavingsReport {
    pub total_compression_savings: u64,
    pub total_loop_savings: u64,
    pub total_calls_blocked: u64,
    pub total_original_tokens: u64,
    pub total_compressed_tokens: u64,
    pub loop_block_by_tool: Vec<(String, u64)>,
}

/// Thread-safe savings ledger.
#[derive(Debug)]
pub struct SavingsLedger {
    compressed_original: AtomicU64,
    compressed_result: AtomicU64,
    loop_calls_blocked: AtomicU64,
    loop_tokens_saved: AtomicU64,
    start_time: Instant,
}

impl Default for SavingsLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl SavingsLedger {
    pub fn new() -> Self {
        Self {
            compressed_original: AtomicU64::new(0),
            compressed_result: AtomicU64::new(0),
            loop_calls_blocked: AtomicU64::new(0),
            loop_tokens_saved: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    /// Record a compression event.
    pub fn record_compression(&self, saving: &CompressionSaving) {
        self.compressed_original.fetch_add(saving.original_tokens, Ordering::Relaxed);
        self.compressed_result.fetch_add(saving.compressed_tokens, Ordering::Relaxed);
    }

    /// Record a loop block event.
    pub fn record_loop_block(&self, saving: &LoopBlockSaving) {
        self.loop_calls_blocked.fetch_add(saving.calls_prevented, Ordering::Relaxed);
        self.loop_tokens_saved.fetch_add(saving.estimated_tokens_saved, Ordering::Relaxed);
    }

    /// Record an early intervention event.
    pub fn record_early_intervention(&self, _saving: &LoopEarlyIntervention) {
        // For now, counts toward loop blocks
        self.loop_calls_blocked.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot report.
    pub fn report(&self) -> SavingsReport {
        let original = self.compressed_original.load(Ordering::Relaxed);
        let compressed = self.compressed_result.load(Ordering::Relaxed);
        SavingsReport {
            total_compression_savings: original.saturating_sub(compressed),
            total_loop_savings: self.loop_tokens_saved.load(Ordering::Relaxed),
            total_calls_blocked: self.loop_calls_blocked.load(Ordering::Relaxed),
            total_original_tokens: original,
            total_compressed_tokens: compressed,
            loop_block_by_tool: Vec::new(),
        }
    }

    /// Uptime since ledger creation.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Compression ratio as a percentage (0.0 – 100.0).
    pub fn compression_ratio(&self) -> f64 {
        let original = self.compressed_original.load(Ordering::Relaxed);
        if original == 0 {
            return 0.0;
        }
        let compressed = self.compressed_result.load(Ordering::Relaxed);
        (1.0 - compressed as f64 / original as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_savings() {
        let ledger = SavingsLedger::new();
        ledger.record_compression(&CompressionSaving {
            original_tokens: 1000,
            compressed_tokens: 300,
            content_type: "json".into(),
        });
        let report = ledger.report();
        assert_eq!(report.total_compression_savings, 700);
        assert!((ledger.compression_ratio() - 70.0).abs() < 0.01);
    }

    #[test]
    fn test_loop_savings() {
        let ledger = SavingsLedger::new();
        ledger.record_loop_block(&LoopBlockSaving {
            tool_name: "write_file".into(),
            calls_prevented: 3,
            estimated_tokens_saved: 600,
        });
        let report = ledger.report();
        assert_eq!(report.total_loop_savings, 600);
        assert_eq!(report.total_calls_blocked, 3);
    }

    #[test]
    fn test_empty_report() {
        let ledger = SavingsLedger::new();
        let report = ledger.report();
        assert_eq!(report.total_compression_savings, 0);
        assert_eq!(report.total_loop_savings, 0);
    }

    #[test]
    fn test_multiple_events_accumulate() {
        let ledger = SavingsLedger::new();
        ledger.record_compression(&CompressionSaving {
            original_tokens: 2000, compressed_tokens: 500, content_type: "code".into(),
        });
        ledger.record_compression(&CompressionSaving {
            original_tokens: 3000, compressed_tokens: 1000, content_type: "text".into(),
        });
        ledger.record_loop_block(&LoopBlockSaving {
            tool_name: "search".into(), calls_prevented: 2, estimated_tokens_saved: 400,
        });
        let report = ledger.report();
        assert_eq!(report.total_compression_savings, 3500);
        assert_eq!(report.total_loop_savings, 400);
        assert_eq!(report.total_calls_blocked, 2);
    }

    #[test]
    fn test_compression_ratio_no_data() {
        let ledger = SavingsLedger::new();
        assert!((ledger.compression_ratio() - 0.0).abs() < 0.01);
    }
}
