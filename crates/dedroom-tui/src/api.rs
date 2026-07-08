//! Typed REST API client for the DedrooM proxy.
//!
//! Fetches stats, attribution reports, health, and SSE events from the
//! proxy's admin endpoints. All endpoints return JSON; we deserialize
//! into matching Rust types here in the TUI crate rather than depending
//! on core crate types (which may not derive Deserialize).

use std::time::Duration;
use anyhow::{Context, Result};
use serde::Deserialize;

// ── Response types (mirror proxy JSON shapes) ──────────────────────────────

/// Response from `GET /admin/learning` — learning memory stats.
#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct LearningResponse {
    pub stats: Option<LearningStatsBlock>,
    pub failing_patterns: Vec<FailingPatternBlock>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct LearningStatsBlock {
    pub total_records: usize,
    pub total_successes: usize,
    pub total_failing_patterns: usize,
    pub total_projects: usize,
    pub overall_success_rate: f64,
    pub by_tool: Vec<ToolLearningBlock>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct ToolLearningBlock {
    pub tool_name: String,
    pub total_attempts: usize,
    pub successes: usize,
    pub success_rate: f64,
    pub best_strategy: Option<String>,
    pub best_strategy_rate: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct FailingPatternBlock {
    pub project_id: String,
    pub tool_name: String,
    pub args_hash: String,
    pub error_signature: String,
    pub frequency: u32,
    pub last_seen: i64,
}

/// Response from `GET /admin/instincts` — active instinct rules.
#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct InstinctsResponse {
    pub rules: Vec<InstinctRuleBlock>,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct InstinctRuleBlock {
    pub id: Option<i64>,
    pub project_id: String,
    pub tool_name: String,
    pub condition: serde_json::Value,
    pub action: serde_json::Value,
    pub confidence: f64,
    pub source: String,
    pub created_at: i64,
    pub hit_count: u32,
    pub success_count: u32,
}

/// Response from `GET /admin/stats`.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct StatsResponse {
    pub savings: SavingsBlock,
    pub loop_state: LoopStateBlock,
    pub config: ConfigBlock,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SavingsBlock {
    pub total_compression_savings_tokens: u64,
    pub total_loop_savings_tokens: u64,
    pub total_calls_blocked: u64,
    pub total_original_tokens: u64,
    pub total_compressed_tokens: u64,
    pub blocked_by_tool: Vec<ToolBreakdownItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct LoopStateBlock {
    pub total_calls: usize,
    pub tool_counts: std::collections::HashMap<String, usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ConfigBlock {
    pub max_repeats: u32,
    pub session_count: usize,
}

/// Per-tool breakdown from the proxy.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolBreakdownItem {
    pub tool: String,
    pub call_count: u64,
    pub tokens_saved: u64,
    pub tokens_processed: u64,
    pub blocked_count: u64,
    pub error_count: u64,
    pub compression_ratio: Option<f64>,
}

/// Response from `GET /admin/attribution` — full attribution report.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct AttributionResponse {
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
    pub waste: WasteBreakdownItem,
    pub per_tool: Vec<ToolBreakdownItem>,
    pub uptime_seconds: u64,
    pub estimated_cost_saved_usd: f64,
    pub estimated_cost_processed_usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasteBreakdownItem {
    pub error_waste_tokens: u64,
    pub blocked_saved_tokens: u64,
    pub uncompressible_waste_tokens: u64,
    pub error_call_count: u64,
    pub blocked_call_count: u64,
    pub uncompressible_call_count: u64,
}

/// Response from `GET /health`.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    #[serde(default)]
    pub pipeline: Option<PipelineBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PipelineBlock {
    pub total_calls_tracked: usize,
    #[serde(default)]
    pub current_loop_state: String,
    pub tool_count: usize,
    #[serde(default)]
    pub max_repeats: u32,
}

/// A single proxy event, matching `ProxyEvent` from the core crate.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ProxyEventItem {
    pub timestamp: u64,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub tool_name: String,
    #[serde(default)]
    pub args_hash: Option<String>,
    pub verdict: String,
    #[serde(default)]
    pub compression_ratio: Option<f64>,
    #[serde(default)]
    pub original_tokens: Option<u64>,
    #[serde(default)]
    pub compressed_tokens: Option<u64>,
    #[serde(default)]
    pub tilt_index: Option<f64>,
    #[serde(default)]
    pub latency_us: u64,
}

// ── API Client ─────────────────────────────────────────────────────────────

/// Async client for fetching data from the DedrooM proxy admin endpoints.
#[derive(Debug, Clone)]
pub struct DashboardApi {
    client: reqwest::Client,
    sse_client: reqwest::Client,
    base_url: String,
}

impl DashboardApi {
    /// Create a new client targeting the proxy on the given `port`.
    pub fn new(port: u16) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Failed to build reqwest client"),
            sse_client: reqwest::Client::builder()
                .build()
                .expect("Failed to build reqwest SSE client"),
            base_url: format!("http://127.0.0.1:{port}"),
        }
    }

    /// Helper for GET requests that return JSON.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET {url} returned {status}: {body}");
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("Failed to parse JSON from {url}"))
    }

    /// Fetch `GET /admin/stats` — savings, loop state, and config summary.
    pub async fn fetch_stats(&self) -> Result<StatsResponse> {
        self.get_json("/admin/stats").await
    }

    /// Fetch `GET /admin/attribution` — full attribution & waste report.
    pub async fn fetch_attribution(&self) -> Result<AttributionResponse> {
        self.get_json("/admin/attribution").await
    }

    /// Fetch `GET /health` — proxy liveness and pipeline summary.
    pub async fn fetch_health(&self) -> Result<HealthResponse> {
        self.get_json("/health").await
    }

    /// Fetch `GET /admin/learning` — learning memory stats and failing patterns.
    pub async fn fetch_learning(&self) -> Result<LearningResponse> {
        self.get_json("/admin/learning").await
    }

    /// Fetch `GET /admin/instincts` — active instinct rules.
    pub async fn fetch_instincts(&self) -> Result<InstinctsResponse> {
        self.get_json("/admin/instincts").await
    }

    /// Fetch all dashboard data in parallel (stats + attribution + health + learning + instincts).
    ///
    /// Returns `None` for any request that fails (connection errors, timeouts).
    /// The caller can then decide whether to show stale data or an error state.
    pub async fn fetch_all(
        &self,
    ) -> (
        Option<StatsResponse>,
        Option<AttributionResponse>,
        Option<HealthResponse>,
        Option<LearningResponse>,
        Option<InstinctsResponse>,
    ) {
        let stats = self.fetch_stats().await.ok();
        let attribution = self.fetch_attribution().await.ok();
        let health = self.fetch_health().await.ok();
        let learning = self.fetch_learning().await.ok();
        let instincts = self.fetch_instincts().await.ok();
        (stats, attribution, health, learning, instincts)
    }

    /// Open an SSE stream of live proxy events.
    ///
    /// Returns a stream of deserialized `ProxyEventItem`s. Non-data SSE
    /// lines (keep-alive, comments) are silently filtered out. The stream
    /// ends when the connection is closed or an error occurs.
    ///
    /// **Note:** This is a best-effort parser. SSE frames spanning chunk
    /// boundaries may occasionally be missed. A future enhancement could
    /// buffer partial lines across chunks.
    pub async fn event_stream(
        &self,
    ) -> Result<impl futures::Stream<Item = ProxyEventItem>> {
        use futures::StreamExt;

        let url = format!("{}/admin/events/stream", self.base_url);
        let resp = self
            .sse_client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        if !resp.status().is_success() {
            anyhow::bail!("GET {url} returned {}", resp.status());
        }

        let stream = resp.bytes_stream().filter_map(|chunk_result| {
            async move {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[dedroom-dash] SSE error: {e}");
                        return None;
                    }
                };
                let text = String::from_utf8_lossy(&chunk);
                // SSE format: "data: {json}\n\n"
                if let Some(data_line) = text.strip_prefix("data:") {
                    let json_str = data_line.trim();
                    match serde_json::from_str::<ProxyEventItem>(json_str) {
                        Ok(event) => Some(event),
                        Err(e) => {
                            eprintln!("[dedroom-dash] SSE parse error: {e}");
                            None
                        }
                    }
                } else {
                    // Skip non-data lines (keep-alive, comments)
                    None
                }
            }
        });

        Ok(stream)
    }
}
