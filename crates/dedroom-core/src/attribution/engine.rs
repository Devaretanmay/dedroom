//! Core attribution engine.
//!
//! Tags each tool call with token-level categories, tracks waste
//! (error results, blocked calls, uncompressible content), and
//! computes ROI metrics for the `/admin/attribution` endpoint.

use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A tag categorizing why tokens were consumed or saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionTag {
    /// Tokens saved by compression.
    CompressionSaved,
    /// Tokens saved by loop prevention.
    LoopBlocked,
    /// Tokens served from the CCR cache (cache hit, no compression needed).
    CacheHit,
    /// Tokens processed (original input before any transformation).
    Processed,
    /// Tokens after compression (what was actually sent/received).
    Compressed,
    /// Tokens wasted on error-producing tool calls.
    ErrorWaste,
    /// Tokens wasted because the content was uncompressible.
    UncompressibleWaste,
}

/// Per-tool-call attribution snapshot recorded by the engine.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallAttribution {
    /// Name of the tool that was called.
    pub tool_name: String,
    /// Estimated tokens in the original (uncompressed) result.
    pub original_tokens: u64,
    /// Estimated tokens after compression (0 if blocked).
    pub compressed_tokens: u64,
    /// Tokens saved = original - compressed (or original if blocked).
    pub tokens_saved: u64,
    /// Whether this call was blocked by loop detection.
    pub was_blocked: bool,
    /// Whether this call produced an error result.
    pub was_error: bool,
    /// Whether the result was served from the CCR cache.
    pub was_cached: bool,
    /// Content type detected by the router (or "blocked" / "error").
    pub content_type: String,
    /// Microseconds spent in the pipeline for this call.
    pub latency_us: u64,
}

/// Waste breakdown for the session.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WasteBreakdown {
    /// Tokens wasted on error-producing tool calls.
    pub error_waste_tokens: u64,
    /// Tokens saved (not wasted) by blocking looped calls.
    pub blocked_saved_tokens: u64,
    /// Tokens wasted because content was uncompressible.
    pub uncompressible_waste_tokens: u64,
    /// Number of error-producing tool calls.
    pub error_call_count: u64,
    /// Number of blocked tool calls.
    pub blocked_call_count: u64,
    /// Number of calls with uncompressible content.
    pub uncompressible_call_count: u64,
}

/// Per-tool summary statistics.
#[derive(Debug, Clone, Serialize)]
pub struct ToolBreakdown {
    /// Tool name.
    pub tool: String,
    /// How many times this tool was called.
    pub call_count: u64,
    /// Tokens saved for this tool.
    pub tokens_saved: u64,
    /// Tokens processed for this tool.
    pub tokens_processed: u64,
    /// How many times this tool was blocked.
    pub blocked_count: u64,
    /// How many times this tool errored.
    pub error_count: u64,
    /// Compression ratio achieved for this tool (0.0–1.0), None if no data.
    pub compression_ratio: Option<f64>,
}

/// Top-level attribution report served at `/admin/attribution`.
#[derive(Debug, Clone, Serialize)]
pub struct AttributionReport {
    /// Total estimated tokens that passed through the pipeline.
    pub total_tokens_processed: u64,
    /// Total estimated tokens saved (compression + loop blocking + cache).
    pub total_tokens_saved: u64,
    /// Tokens saved by compression alone.
    pub total_compression_savings: u64,
    /// Tokens saved by loop blocking alone.
    pub total_loop_savings: u64,
    /// Cache hits — calls served from CCR without recomputation.
    pub total_cache_hits: u64,
    /// Tokens saved by cache hits.
    pub total_cache_saved_tokens: u64,
    /// Overall savings ratio (saved / processed), clamped to [0.0, 1.0].
    pub savings_ratio: f64,
    /// Compression ratio (1.0 - compressed/original), clamped to [0.0, 1.0].
    pub compression_ratio: f64,
    /// Total tool calls processed.
    pub total_calls: u64,
    /// Number of tool calls blocked by loop detection.
    pub blocked_calls: u64,
    /// Number of tool calls that produced errors.
    pub error_calls: u64,
    /// Number of cache hits.
    pub cache_hits: u64,
    /// Waste breakdown.
    pub waste: WasteBreakdown,
    /// Per-tool breakdown.
    pub per_tool: Vec<ToolBreakdown>,
    /// Uptime of the engine in seconds.
    pub uptime_seconds: u64,
    /// Cost estimate (USD), assuming a default per-token rate.
    pub estimated_cost_saved_usd: f64,
    /// Cost estimate for tokens processed (USD).
    pub estimated_cost_processed_usd: f64,
}

impl AttributionReport {
    /// Estimated cost per 1M tokens (input, Claude 3 Haiku pricing).
    const COST_PER_1M_TOKENS: f64 = 0.25;

    fn cost_for_tokens(tokens: u64) -> f64 {
        tokens as f64 / 1_000_000.0 * Self::COST_PER_1M_TOKENS
    }
}

/// Thread-safe attribution engine that tracks per-call token usage.
///
/// Designed to be used alongside `SavingsLedger` — the attribution engine
/// adds a per-call granularity layer on top of the aggregate savings counters.
#[derive(Debug)]
pub struct AttributionEngine {
    /// Per-tool attribution history (ordered by call time).
    history: Vec<ToolCallAttribution>,
    /// Per-tool aggregate counters.
    tool_aggregates: HashMap<String, ToolAggregate>,
    /// Start time for uptime computation.
    start_time: Instant,
    /// Cache hit counter.
    cache_hits: u64,
    /// Cache saved tokens.
    cache_saved_tokens: u64,
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

impl Default for AttributionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AttributionEngine {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            tool_aggregates: HashMap::new(),
            start_time: Instant::now(),
            cache_hits: 0,
            cache_saved_tokens: 0,
        }
    }

    /// Record a tool call attribution after processing through the pipeline.
    pub fn record(&mut self, attribution: ToolCallAttribution) {
        let tokens_saved = attribution.tokens_saved;

        // Update per-tool aggregates
        let agg = self.tool_aggregates.entry(attribution.tool_name.clone()).or_default();
        agg.call_count += 1;
        agg.tokens_saved += tokens_saved;
        agg.tokens_processed += attribution.original_tokens;
        if attribution.was_blocked {
            agg.blocked_count += 1;
        }
        if attribution.was_error {
            agg.error_count += 1;
        }
        if !attribution.was_blocked {
            agg.original_tokens_total += attribution.original_tokens;
            agg.compressed_tokens_total += attribution.compressed_tokens;
        }

        // Keep history bounded (last 10,000 calls)
        self.history.push(attribution);
        if self.history.len() > 10_000 {
            self.history.remove(0);
        }
    }

    /// Record a cache hit (result served from CCR without re-compression).
    pub fn record_cache_hit(&mut self, tool_name: &str, tokens_saved: u64) {
        self.cache_hits += 1;
        self.cache_saved_tokens += tokens_saved;

        let agg = self.tool_aggregates.entry(tool_name.to_string()).or_default();
        agg.call_count += 1;
        agg.tokens_saved += tokens_saved;
    }

    /// Generate the full attribution report.
    pub fn report(&self) -> AttributionReport {
        let total_calls = self.history.len() as u64;
        let total_processed: u64 = self.history.iter().map(|a| a.original_tokens).sum();
        let total_saved: u64 = self.history.iter().map(|a| a.tokens_saved).sum();
        let compression_savings: u64 = self
            .history
            .iter()
            .filter(|a| !a.was_blocked)
            .map(|a| a.original_tokens.saturating_sub(a.compressed_tokens))
            .sum();
        let loop_savings: u64 = self
            .history
            .iter()
            .filter(|a| a.was_blocked)
            .map(|a| a.tokens_saved)
            .sum();

        let blocked_calls = self.history.iter().filter(|a| a.was_blocked).count() as u64;
        let error_calls = self.history.iter().filter(|a| a.was_error).count() as u64;

        // Waste breakdown
        let error_waste_tokens: u64 = self
            .history
            .iter()
            .filter(|a| a.was_error)
            .map(|a| a.original_tokens)
            .sum();
        let blocked_saved_tokens: u64 = loop_savings;
        let blocked_call_count = blocked_calls;
        let uncompressible_waste_tokens: u64 = self
            .history
            .iter()
            .filter(|a| !a.was_blocked && !a.was_error && a.compressed_tokens == a.original_tokens && a.original_tokens > 0)
            .map(|a| a.original_tokens)
            .sum();
        let uncompressible_call_count = self
            .history
            .iter()
            .filter(|a| !a.was_blocked && !a.was_error && a.compressed_tokens == a.original_tokens && a.original_tokens > 0)
            .count() as u64;

        // Average compression ratio (across non-blocked calls with data)
        let (compressed_total, original_total) = self
            .history
            .iter()
            .filter(|a| !a.was_blocked && a.original_tokens > 0)
            .fold((0u64, 0u64), |(c, o), a| (c + a.compressed_tokens, o + a.original_tokens));
        let compression_ratio = if original_total > 0 {
            1.0 - compressed_total as f64 / original_total as f64
        } else {
            0.0
        };

        // Overall savings ratio
        let savings_ratio = if total_processed > 0 {
            (total_saved + self.cache_saved_tokens) as f64 / total_processed as f64
        } else {
            0.0
        };

        // Per-tool breakdown
        let mut per_tool: Vec<ToolBreakdown> = self
            .tool_aggregates
            .iter()
            .map(|(tool, agg)| {
                let compression_ratio = if agg.original_tokens_total > 0 {
                    Some(1.0 - agg.compressed_tokens_total as f64 / agg.original_tokens_total as f64)
                } else {
                    None
                };
                ToolBreakdown {
                    tool: tool.clone(),
                    call_count: agg.call_count,
                    tokens_saved: agg.tokens_saved,
                    tokens_processed: agg.tokens_processed,
                    blocked_count: agg.blocked_count,
                    error_count: agg.error_count,
                    compression_ratio,
                }
            })
            .collect();
        per_tool.sort_unstable_by_key(|t| std::cmp::Reverse(t.tokens_saved));

        let total_saved_with_cache = total_saved + self.cache_saved_tokens;
        let uptime = self.start_time.elapsed();

        AttributionReport {
            total_tokens_processed: total_processed,
            total_tokens_saved: total_saved_with_cache,
            total_compression_savings: compression_savings,
            total_loop_savings: loop_savings,
            total_cache_hits: self.cache_hits,
            total_cache_saved_tokens: self.cache_saved_tokens,
            savings_ratio,
            compression_ratio: compression_ratio.max(0.0),
            total_calls,
            blocked_calls,
            error_calls,
            cache_hits: self.cache_hits,
            waste: WasteBreakdown {
                error_waste_tokens,
                blocked_saved_tokens,
                uncompressible_waste_tokens,
                error_call_count: error_calls,
                blocked_call_count,
                uncompressible_call_count,
            },
            per_tool,
            uptime_seconds: uptime.as_secs(),
            estimated_cost_saved_usd: AttributionReport::cost_for_tokens(total_saved_with_cache),
            estimated_cost_processed_usd: AttributionReport::cost_for_tokens(total_processed),
        }
    }

    /// Uptime since engine creation.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Number of calls recorded.
    pub fn call_count(&self) -> usize {
        self.history.len()
    }

    /// Returns a copy of the recent attribution history.
    pub fn recent_history(&self, n: usize) -> Vec<ToolCallAttribution> {
        let len = self.history.len();
        let take = n.min(len);
        self.history.iter().rev().take(take).cloned().rev().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_attribution(
        tool: &str,
        original: u64,
        compressed: u64,
        blocked: bool,
        error: bool,
        cached: bool,
    ) -> ToolCallAttribution {
        let tokens_saved = if blocked {
            original
        } else {
            original.saturating_sub(compressed)
        };
        ToolCallAttribution {
            tool_name: tool.to_string(),
            original_tokens: original,
            compressed_tokens: compressed,
            tokens_saved,
            was_blocked: blocked,
            was_error: error,
            was_cached: cached,
            content_type: if blocked { "blocked".into() } else { "text".into() },
            latency_us: 100,
        }
    }

    #[test]
    fn test_empty_engine() {
        let engine = AttributionEngine::new();
        let report = engine.report();
        assert_eq!(report.total_calls, 0);
        assert_eq!(report.total_tokens_processed, 0);
        assert_eq!(report.savings_ratio, 0.0);
        assert_eq!(report.estimated_cost_saved_usd, 0.0);
    }

    #[test]
    fn test_tracks_compression_savings() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("read_file", 1000, 300, false, false, false));
        let report = engine.report();
        assert_eq!(report.total_calls, 1);
        assert_eq!(report.total_tokens_saved, 700);
        assert_eq!(report.total_compression_savings, 700);
        assert!((report.compression_ratio - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_tracks_loop_savings() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("write_file", 500, 0, true, false, false));
        let report = engine.report();
        assert_eq!(report.total_calls, 1);
        assert_eq!(report.total_loop_savings, 500);
        assert_eq!(report.blocked_calls, 1);
        assert_eq!(report.waste.blocked_saved_tokens, 500);
    }

    #[test]
    fn test_tracks_error_waste() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("search", 200, 200, false, true, false));
        let report = engine.report();
        assert_eq!(report.error_calls, 1);
        assert_eq!(report.waste.error_waste_tokens, 200);
    }

    #[test]
    fn test_tracks_cache_hits() {
        let mut engine = AttributionEngine::new();
        engine.record_cache_hit("read_file", 1000);
        let report = engine.report();
        assert_eq!(report.cache_hits, 1);
        assert_eq!(report.total_cache_saved_tokens, 1000);
        assert_eq!(report.total_tokens_saved, 1000);
    }

    #[test]
    fn test_per_tool_breakdown() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("read_file", 1000, 300, false, false, false));
        engine.record(make_attribution("read_file", 800, 200, false, false, false));
        engine.record(make_attribution("write_file", 500, 0, true, false, false));
        let report = engine.report();
        assert_eq!(report.total_calls, 3);
        assert_eq!(report.per_tool.len(), 2);

        let read = report.per_tool.iter().find(|t| t.tool == "read_file").unwrap();
        assert_eq!(read.call_count, 2);
        assert_eq!(read.tokens_saved, 1300); // 700 + 600

        let write = report.per_tool.iter().find(|t| t.tool == "write_file").unwrap();
        assert_eq!(write.call_count, 1);
        assert_eq!(write.blocked_count, 1);
    }

    #[test]
    fn test_history_bounded() {
        let mut engine = AttributionEngine::new();
        // Record more than the 10,000 limit
        for _i in 0..10_050 {
            engine.record(make_attribution("tool", 10, 5, false, false, false));
        }
        assert_eq!(engine.call_count(), 10_000);
    }

    #[test]
    fn test_cost_estimates() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("read_file", 1_000_000, 300_000, false, false, false));
        let report = engine.report();
        // 700,000 tokens saved at $0.25/1M = $0.175
        assert!((report.estimated_cost_saved_usd - 0.175).abs() < 0.001);
        // 1,000,000 tokens processed at $0.25/1M = $0.25
        assert!((report.estimated_cost_processed_usd - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_recent_history() {
        let mut engine = AttributionEngine::new();
        for i in 0..10 {
            engine.record(make_attribution(&format!("tool_{i}"), 10, 5, false, false, false));
        }
        let recent = engine.recent_history(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].tool_name, "tool_7");
        assert_eq!(recent[2].tool_name, "tool_9");
    }

    #[test]
    fn test_savings_ratio() {
        let mut engine = AttributionEngine::new();
        engine.record(make_attribution("read_file", 1000, 300, false, false, false));
        engine.record(make_attribution("write_file", 500, 0, true, false, false));
        let report = engine.report();
        // total_processed = 1500, total_saved = 700 + 500 = 1200
        // savings_ratio = 1200 / 1500 = 0.8
        assert!((report.savings_ratio - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_uncompressible_waste() {
        let mut engine = AttributionEngine::new();
        // Same tokens for original and compressed = uncompressible
        engine.record(make_attribution("read_file", 100, 100, false, false, false));
        let report = engine.report();
        assert_eq!(report.waste.uncompressible_waste_tokens, 100);
        assert_eq!(report.waste.uncompressible_call_count, 1);
        assert_eq!(report.total_tokens_saved, 0);
    }

    #[test]
    fn test_handle_zero_processed() {
        let engine = AttributionEngine::new();
        let report = engine.report();
        assert_eq!(report.savings_ratio, 0.0);
        assert_eq!(report.compression_ratio, 0.0);
        assert!(report.estimated_cost_saved_usd == 0.0);
    }

    #[test]
    fn test_uptime() {
        let engine = AttributionEngine::new();
        let uptime = engine.uptime();
        assert!(uptime.as_secs() >= 0);
    }
}
