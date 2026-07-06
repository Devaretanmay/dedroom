//! Unified ledger tracking compression savings and loop prevention.
//!
//! Thread-safe aggregate counters via `AtomicU64`. Also tracks per-tool
//! attribution data for ROI reporting.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use serde::Serialize;

/// A snapshot savings report.
#[derive(Debug, Clone, Default)]
pub struct SavingsReport {
    pub total_compression_savings: u64,
    pub total_loop_savings: u64,
    pub total_calls_blocked: u64,
    pub total_original_tokens: u64,
    pub total_compressed_tokens: u64,
}

/// Per-tool aggregate counters for attribution.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ToolBreakdown {
    pub tool: String,
    pub call_count: u64,
    pub tokens_saved: u64,
    pub tokens_processed: u64,
    pub blocked_count: u64,
    pub error_count: u64,
    pub compression_ratio: Option<f64>,
}

/// Waste breakdown for the session.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WasteBreakdown {
    pub error_waste_tokens: u64,
    pub blocked_saved_tokens: u64,
    pub uncompressible_waste_tokens: u64,
    pub error_call_count: u64,
    pub blocked_call_count: u64,
    pub uncompressible_call_count: u64,
}

/// Top-level attribution report.
#[derive(Debug, Clone, Serialize)]
pub struct AttributionReport {
    pub total_tokens_processed: u64,
    pub total_tokens_saved: u64,
    pub total_compression_savings: u64,
    pub total_loop_savings: u64,
    pub total_cache_hits: u64,
    pub total_cache_saved_tokens: u64,
    pub savings_ratio: f64,
    pub compression_ratio: f64,
    pub total_calls: u64,
    pub blocked_calls: u64,
    pub error_calls: u64,
    pub cache_hits: u64,
    pub waste: WasteBreakdown,
    pub per_tool: Vec<ToolBreakdown>,
    pub uptime_seconds: u64,
    pub estimated_cost_saved_usd: f64,
    pub estimated_cost_processed_usd: f64,
}

impl AttributionReport {
    const COST_PER_1M_TOKENS: f64 = 0.25;
    fn cost_for_tokens(tokens: u64) -> f64 {
        tokens as f64 / 1_000_000.0 * Self::COST_PER_1M_TOKENS
    }
}

/// Per-tool call record for history tracking.
#[derive(Debug, Clone)]
struct ToolCallRecord {
    original_tokens: u64,
    compressed_tokens: u64,
    tokens_saved: u64,
    was_blocked: bool,
    was_error: bool,
}

/// Thread-safe savings ledger with per-tool attribution tracking.
#[derive(Debug)]
pub struct SavingsLedger {
    compressed_original: AtomicU64,
    compressed_result: AtomicU64,
    loop_calls_blocked: AtomicU64,
    loop_tokens_saved: AtomicU64,
    start_time: Instant,
    /// Per-tool aggregate counters (behind Mutex, used infrequently).
    tool_aggregates: Mutex<HashMap<String, ToolAggregate>>,
    /// History for attribution reports (last 10,000 calls).
    history: Mutex<Vec<ToolCallRecord>>,
    cache_hits: AtomicU64,
    cache_saved_tokens: AtomicU64,
}

#[derive(Debug, Default, Clone)]
struct ToolAggregate {
    call_count: u64,
    tokens_saved: u64,
    tokens_processed: u64,
    blocked_count: u64,
    error_count: u64,
    original_tokens_total: u64,
    compressed_tokens_total: u64,
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
            tool_aggregates: Mutex::new(HashMap::new()),
            history: Mutex::new(Vec::new()),
            cache_hits: AtomicU64::new(0),
            cache_saved_tokens: AtomicU64::new(0),
        }
    }

    pub fn record_compression(&self, original_tokens: u64, compressed_tokens: u64) {
        self.compressed_original.fetch_add(original_tokens, Ordering::Relaxed);
        self.compressed_result.fetch_add(compressed_tokens, Ordering::Relaxed);
    }

    pub fn record_loop_block(&self, calls_prevented: u64, tokens_saved: u64) {
        self.loop_calls_blocked.fetch_add(calls_prevented, Ordering::Relaxed);
        self.loop_tokens_saved.fetch_add(tokens_saved, Ordering::Relaxed);
    }

    pub fn record_tool_call(
        &self,
        tool_name: &str,
        original_tokens: u64,
        compressed_tokens: u64,
        was_blocked: bool,
        was_error: bool,
        was_cached: bool,
    ) {
        let tokens_saved = if was_blocked || was_cached {
            original_tokens
        } else {
            original_tokens.saturating_sub(compressed_tokens)
        };

        let mut aggs = self.tool_aggregates.lock().unwrap();
        let agg = aggs.entry(tool_name.to_string()).or_default();
        agg.call_count += 1;
        agg.tokens_saved += tokens_saved;
        agg.tokens_processed += original_tokens;
        if was_blocked { agg.blocked_count += 1; }
        if was_error { agg.error_count += 1; }
        if !was_blocked {
            agg.original_tokens_total += original_tokens;
            agg.compressed_tokens_total += compressed_tokens;
        }
        drop(aggs);

        let mut hist = self.history.lock().unwrap();
        hist.push(ToolCallRecord {
            original_tokens,
            compressed_tokens,
            tokens_saved,
            was_blocked,
            was_error,
        });
        if hist.len() > 10_000 {
            hist.remove(0);
        }
    }

    pub fn record_cache_hit(&self, tool_name: &str, tokens_saved: u64) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        self.cache_saved_tokens.fetch_add(tokens_saved, Ordering::Relaxed);

        let mut aggs = self.tool_aggregates.lock().unwrap();
        let agg = aggs.entry(tool_name.to_string()).or_default();
        agg.call_count += 1;
        agg.tokens_saved += tokens_saved;
    }

    pub fn report(&self) -> SavingsReport {
        let original = self.compressed_original.load(Ordering::Relaxed);
        let compressed = self.compressed_result.load(Ordering::Relaxed);
        SavingsReport {
            total_compression_savings: original.saturating_sub(compressed),
            total_loop_savings: self.loop_tokens_saved.load(Ordering::Relaxed),
            total_calls_blocked: self.loop_calls_blocked.load(Ordering::Relaxed),
            total_original_tokens: original,
            total_compressed_tokens: compressed,
        }
    }

    pub fn attribution_report(&self) -> AttributionReport {
        let hist = self.history.lock().unwrap();
        let aggs = self.tool_aggregates.lock().unwrap();

        let total_calls = hist.len() as u64;
        let total_processed: u64 = hist.iter().map(|r| r.original_tokens).sum();
        let total_saved: u64 = hist.iter().map(|r| r.tokens_saved).sum();
        let compression_savings: u64 = hist.iter()
            .filter(|r| !r.was_blocked)
            .map(|r| r.original_tokens.saturating_sub(r.compressed_tokens))
            .sum();
        let loop_savings: u64 = hist.iter()
            .filter(|r| r.was_blocked)
            .map(|r| r.tokens_saved)
            .sum();
        let blocked_calls = hist.iter().filter(|r| r.was_blocked).count() as u64;
        let error_calls = hist.iter().filter(|r| r.was_error).count() as u64;
        let cache_hits = self.cache_hits.load(Ordering::Relaxed);
        let cache_saved = self.cache_saved_tokens.load(Ordering::Relaxed);

        let error_waste_tokens: u64 = hist.iter().filter(|r| r.was_error).map(|r| r.original_tokens).sum();
        let uncompressible_waste_tokens: u64 = hist.iter()
            .filter(|r| !r.was_blocked && !r.was_error && r.compressed_tokens == r.original_tokens && r.original_tokens > 0)
            .map(|r| r.original_tokens)
            .sum();
        let uncompressible_call_count = hist.iter()
            .filter(|r| !r.was_blocked && !r.was_error && r.compressed_tokens == r.original_tokens && r.original_tokens > 0)
            .count() as u64;

        let (compressed_total, original_total) = hist.iter()
            .filter(|r| !r.was_blocked && r.original_tokens > 0)
            .fold((0u64, 0u64), |(c, o), r| (c + r.compressed_tokens, o + r.original_tokens));
        let compression_ratio = if original_total > 0 { 1.0 - compressed_total as f64 / original_total as f64 } else { 0.0 };
        let savings_ratio = if total_processed > 0 { (total_saved + cache_saved) as f64 / total_processed as f64 } else { 0.0 };

        let mut per_tool: Vec<ToolBreakdown> = aggs.iter().map(|(tool, agg)| {
            let cr = if agg.original_tokens_total > 0 {
                Some(1.0 - agg.compressed_tokens_total as f64 / agg.original_tokens_total as f64)
            } else { None };
            ToolBreakdown {
                tool: tool.clone(),
                call_count: agg.call_count,
                tokens_saved: agg.tokens_saved,
                tokens_processed: agg.tokens_processed,
                blocked_count: agg.blocked_count,
                error_count: agg.error_count,
                compression_ratio: cr,
            }
        }).collect();
        per_tool.sort_unstable_by_key(|t| std::cmp::Reverse(t.tokens_saved));

        let total_saved_with_cache = total_saved + cache_saved;
        let uptime = self.start_time.elapsed();

        AttributionReport {
            total_tokens_processed: total_processed,
            total_tokens_saved: total_saved_with_cache,
            total_compression_savings: compression_savings,
            total_loop_savings: loop_savings,
            total_cache_hits: cache_hits,
            total_cache_saved_tokens: cache_saved,
            savings_ratio,
            compression_ratio: compression_ratio.max(0.0),
            total_calls,
            blocked_calls,
            error_calls,
            cache_hits,
            waste: WasteBreakdown {
                error_waste_tokens,
                blocked_saved_tokens: loop_savings,
                uncompressible_waste_tokens,
                error_call_count: error_calls,
                blocked_call_count: blocked_calls,
                uncompressible_call_count,
            },
            per_tool,
            uptime_seconds: uptime.as_secs(),
            estimated_cost_saved_usd: AttributionReport::cost_for_tokens(total_saved_with_cache),
            estimated_cost_processed_usd: AttributionReport::cost_for_tokens(total_processed),
        }
    }

    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn compression_ratio(&self) -> f64 {
        let original = self.compressed_original.load(Ordering::Relaxed);
        if original == 0 { return 0.0; }
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
        ledger.record_compression(1000, 300);
        let report = ledger.report();
        assert_eq!(report.total_compression_savings, 700);
        assert!((ledger.compression_ratio() - 70.0).abs() < 0.01);
    }

    #[test]
    fn test_loop_savings() {
        let ledger = SavingsLedger::new();
        ledger.record_loop_block(3, 600);
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
    fn test_attribution_report_empty() {
        let ledger = SavingsLedger::new();
        let report = ledger.attribution_report();
        assert_eq!(report.total_calls, 0);
        assert_eq!(report.savings_ratio, 0.0);
    }

    #[test]
    fn test_tracks_tool_calls() {
        let ledger = SavingsLedger::new();
        ledger.record_tool_call("read_file", 1000, 300, false, false, false);
        let r = ledger.attribution_report();
        assert_eq!(r.total_calls, 1);
        assert_eq!(r.total_tokens_saved, 700);
        assert_eq!(r.total_compression_savings, 700);
    }

    #[test]
    fn test_tracks_blocked_calls() {
        let ledger = SavingsLedger::new();
        ledger.record_tool_call("write_file", 500, 0, true, false, false);
        let r = ledger.attribution_report();
        assert_eq!(r.blocked_calls, 1);
        assert_eq!(r.total_loop_savings, 500);
    }

    #[test]
    fn test_tracks_cache_hits() {
        let ledger = SavingsLedger::new();
        ledger.record_cache_hit("read_file", 1000);
        let r = ledger.attribution_report();
        assert_eq!(r.cache_hits, 1);
        assert_eq!(r.total_cache_saved_tokens, 1000);
    }

    #[test]
    fn test_per_tool_breakdown() {
        let ledger = SavingsLedger::new();
        ledger.record_tool_call("read_file", 1000, 300, false, false, false);
        ledger.record_tool_call("read_file", 800, 200, false, false, false);
        ledger.record_tool_call("write_file", 500, 0, true, false, false);
        let r = ledger.attribution_report();
        assert_eq!(r.total_calls, 3);
        assert_eq!(r.per_tool.len(), 2);
        let read = r.per_tool.iter().find(|t| t.tool == "read_file").unwrap();
        assert_eq!(read.call_count, 2);
        assert_eq!(read.tokens_saved, 1300);
    }

    #[test]
    fn test_multiple_events_accumulate() {
        let ledger = SavingsLedger::new();
        ledger.record_compression(2000, 500);
        ledger.record_compression(3000, 1000);
        ledger.record_loop_block(2, 400);
        let report = ledger.report();
        assert_eq!(report.total_compression_savings, 3500);
        assert_eq!(report.total_loop_savings, 400);
        assert_eq!(report.total_calls_blocked, 2);
    }
}
