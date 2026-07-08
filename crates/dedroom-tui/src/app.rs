//! Application state — manages tabs, data aggregation, event history.
//!
//! The [`App`] struct holds all mutable state for the TUI dashboard.
//! Data is refreshed from the proxy on a tick interval and via SSE
//! events. A rolling history of stats samples is kept for sparklines.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::api;

/// Maximum number of events kept in the timeline.
const MAX_EVENTS: usize = 2000;

/// Maximum number of stats snapshots in the rolling history for sparklines.
const MAX_HISTORY_SAMPLES: usize = 180;

/// Available dashboard tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Tools,
    Healing,
    Waste,
    Export,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Tools,
        Tab::Healing,
        Tab::Waste,
        Tab::Export,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Tools => "Tools",
            Tab::Healing => "Healing",
            Tab::Waste => "Waste",
            Tab::Export => "Export",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tab::Overview => "\u{1f4ca}",   // 📊
            Tab::Tools => "\u{1f527}",       // 🔧
            Tab::Healing => "\u{2764}\u{fe0f}",  // ❤️
            Tab::Waste => "\u{1f4a8}",       // 💨
            Tab::Export => "\u{1f4e4}",      // 📤
        }
    }
}

/// A single data snapshot at a point in time (used for sparkline history).
#[derive(Debug, Clone)]
pub struct DataSample {
    pub tokens_saved: u64,
    pub compression_ratio: f64,
}

/// Top-level application state for the TUI dashboard.
#[derive(Debug)]
pub struct App {
    // ── Connection ──────────────────────────────────────────────────────
    pub api: api::DashboardApi,
    pub is_connected: bool,
    pub error: Option<String>,

    // ── Tab management ──────────────────────────────────────────────────
    pub current_tab: Tab,
    pub tabs: Vec<Tab>,

    // ── Latest data from proxy ──────────────────────────────────────────
    pub stats: Option<api::StatsResponse>,
    pub attribution: Option<api::AttributionResponse>,
    pub health: Option<api::HealthResponse>,
    pub learning: Option<api::LearningResponse>,
    pub instincts: Option<api::InstinctsResponse>,

    // ── Event timeline ──────────────────────────────────────────────────
    pub events: VecDeque<api::ProxyEventItem>,

    // ── Rolling history for sparklines ──────────────────────────────────
    pub history: VecDeque<DataSample>,

    // ── Refresh / timing ────────────────────────────────────────────────
    pub last_refresh: Instant,
    pub refresh_interval: Duration,

    // ── Derived values (computed once per refresh) ──────────────────────
    pub total_tokens_saved: u64,
    pub total_dollars_saved: f64,
    pub total_calls_processed: u64,
    pub blocked_calls: u64,
    pub self_healed_count: usize,
    pub compression_avg: f64,
    pub savings_avg: f64,

    /// Whether an SSE event notification was received since last render.
    /// Used to flash a "live" indicator.
    pub sse_activity: bool,
}

impl App {
    /// Create a new app state targeting the proxy on `port`.
    pub fn new(port: u16) -> Self {
        Self {
            api: api::DashboardApi::new(port),
            is_connected: false,
            error: None,
            current_tab: Tab::Overview,
            tabs: Tab::ALL.to_vec(),
            stats: None,
            attribution: None,
            health: None,
            learning: None,
            instincts: None,
            events: VecDeque::with_capacity(MAX_EVENTS),
            history: VecDeque::with_capacity(MAX_HISTORY_SAMPLES),
            last_refresh: Instant::now(),
            refresh_interval: Duration::from_secs(2),
            total_tokens_saved: 0,
            total_dollars_saved: 0.0,
            total_calls_processed: 0,
            blocked_calls: 0,
            self_healed_count: 0,
            compression_avg: 0.0,
            savings_avg: 0.0,
            sse_activity: false,
        }
    }

    /// Refresh all data from the proxy.
    ///
    /// Fetches stats, attribution, health, learning, and instincts in parallel.
    /// Updates derived values and pushes a sample to the rolling history.
    pub async fn refresh(&mut self) {
        let (stats, attribution, health, learning, instincts) = self.api.fetch_all().await;

        self.stats = stats;
        let had_attribution = self.attribution.is_some();
        self.attribution = attribution;
        self.health = health;
        self.learning = learning;
        self.instincts = instincts;

        // Update self-healed count from learning memory stats
        if let Some(ref lr) = self.learning {
            if let Some(ref stats) = lr.stats {
                self.self_healed_count = stats.total_successes;
            }
        }

        self.is_connected = self.stats.is_some() || self.attribution.is_some();
        self.error = None;

        self.update_derived();
        self.push_history_sample();

        // If we just got our first attribution data, mark connection
        if !had_attribution && self.attribution.is_some() {
            self.is_connected = true;
        }
    }

    /// Update derived values from current stats/attribution.
    fn update_derived(&mut self) {
        if let Some(ref att) = self.attribution {
            self.total_tokens_saved = att.total_tokens_saved;
            self.total_dollars_saved = att.estimated_cost_saved_usd;
            self.total_calls_processed = att.total_calls;
            self.blocked_calls = att.blocked_calls;
            self.compression_avg = att.compression_ratio;
            self.savings_avg = att.savings_ratio;
            // Self-healed count isn't directly in AttributionReport yet,
            // but we can use successful_recoveries from healing engine
        } else if let Some(ref stats) = self.stats {
            self.total_tokens_saved =
                stats.savings.total_compression_savings_tokens + stats.savings.total_loop_savings_tokens;
            self.total_calls_processed = stats.loop_state.total_calls as u64;
            self.blocked_calls = stats.savings.total_calls_blocked;
        }
    }

    /// Push a data sample to the rolling history for sparklines.
    fn push_history_sample(&mut self) {
        let sample = DataSample {
            tokens_saved: self.total_tokens_saved,
            compression_ratio: self.compression_avg.max(0.0),
        };
        self.history.push_back(sample);
        while self.history.len() > MAX_HISTORY_SAMPLES {
            self.history.pop_front();
        }
    }

    /// Called on each tick (every 500ms by default).
    /// Refreshes data if the refresh interval has elapsed.
    pub async fn tick(&mut self) {
        if self.last_refresh.elapsed() >= self.refresh_interval {
            self.refresh().await;
            self.last_refresh = Instant::now();
            self.sse_activity = false;
        }
    }

    /// Push an event from the SSE stream into the timeline.
    pub fn push_event(&mut self, event: api::ProxyEventItem) {
        self.events.push_back(event);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        self.sse_activity = true;
    }

    /// Set an error message to display.
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.is_connected = false;
    }

    // ── Tab navigation ──────────────────────────────────────────────────

    pub fn next_tab(&mut self) {
        let idx = self.tabs.iter().position(|t| *t == self.current_tab);
        if let Some(i) = idx {
            self.current_tab = self.tabs[(i + 1) % self.tabs.len()];
        }
    }

    pub fn prev_tab(&mut self) {
        let idx = self.tabs.iter().position(|t| *t == self.current_tab);
        if let Some(i) = idx {
            self.current_tab = if i == 0 {
                self.tabs[self.tabs.len() - 1]
            } else {
                self.tabs[i - 1]
            };
        }
    }

    pub fn go_to_tab(&mut self, tab: Tab) {
        self.current_tab = tab;
    }

    // ── Queries for components ──────────────────────────────────────────

    /// Get the top N tools by savings (sorted descending).
    pub fn top_tools(&self, n: usize) -> Vec<&api::ToolBreakdownItem> {
        match self.attribution {
            Some(ref att) => {
                let mut tools: Vec<&api::ToolBreakdownItem> = att.per_tool.iter().collect();
                tools.sort_by(|a, b| b.tokens_saved.cmp(&a.tokens_saved));
                tools.truncate(n);
                tools
            }
            None => Vec::new(),
        }
    }

    /// Get recent events for the timeline (most recent first).
    pub fn recent_events(&self, n: usize) -> Vec<&api::ProxyEventItem> {
        self.events.iter().rev().take(n).collect()
    }

    /// Get events within a time window (for zoom).
    pub fn events_in_window(&self, window_secs: u64) -> Vec<&api::ProxyEventItem> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(window_secs);
        self.events
            .iter()
            .filter(|e| e.timestamp >= cutoff * 1000) // timestamps are in millis
            .collect()
    }

    /// Get bytes count for the SSE live indicator.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Uptime string from health data.
    pub fn uptime_str(&self) -> String {
        if let Some(ref att) = self.attribution {
            let secs = att.uptime_seconds;
            if secs < 60 {
                format!("{}s", secs)
            } else if secs < 3600 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
            }
        } else {
            String::from("--")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_navigation() {
        let mut app = App::new(8080);
        assert_eq!(app.current_tab, Tab::Overview);
        app.next_tab();
        assert_eq!(app.current_tab, Tab::Tools);
        app.next_tab();
        assert_eq!(app.current_tab, Tab::Healing);
        app.prev_tab();
        assert_eq!(app.current_tab, Tab::Tools);
    }

    #[test]
    fn test_tab_wraparound() {
        let mut app = App::new(8080);
        app.current_tab = Tab::Export;
        app.next_tab();
        assert_eq!(app.current_tab, Tab::Overview);
        app.prev_tab();
        assert_eq!(app.current_tab, Tab::Export);
    }

    #[test]
    fn test_go_to_tab() {
        let mut app = App::new(8080);
        app.go_to_tab(Tab::Waste);
        assert_eq!(app.current_tab, Tab::Waste);
    }

    #[test]
    fn test_event_limit() {
        let mut app = App::new(8080);
        for i in 0..2100 {
            app.push_event(api::ProxyEventItem {
                timestamp: i as u64,
                session_id: None,
                agent_id: None,
                tool_name: format!("tool_{}", i),
                args_hash: None,
                verdict: "allow".into(),
                compression_ratio: None,
                original_tokens: None,
                compressed_tokens: None,
                tilt_index: None,
                latency_us: 0,
            });
        }
        assert!(app.events.len() <= MAX_EVENTS);
        // Should have removed oldest events
        let first = app.events.front().unwrap();
        assert!(first.tool_name != "tool_0");
    }

    #[test]
    fn test_history_limit() {
        let mut app = App::new(8080);
        app.attribution = Some(api::AttributionResponse {
            total_tokens_saved: 0,
            compression_ratio: 0.0,
            savings_ratio: 0.0,
            blocked_calls: 0,
            total_calls: 0,
            total_tokens_processed: 0,
            total_compression_savings: 0,
            total_loop_savings: 0,
            total_cache_hits: 0,
            total_cache_saved_tokens: 0,
            error_calls: 0,
            cache_hits: 0,
            waste: api::WasteBreakdownItem {
                error_waste_tokens: 0,
                blocked_saved_tokens: 0,
                uncompressible_waste_tokens: 0,
                error_call_count: 0,
                blocked_call_count: 0,
                uncompressible_call_count: 0,
            },
            per_tool: vec![],
            uptime_seconds: 0,
            estimated_cost_saved_usd: 0.0,
            estimated_cost_processed_usd: 0.0,
        });
        for i in 0..200u64 {
            app.total_tokens_saved = i * 100;
            app.push_history_sample();
        }
        assert!(app.history.len() <= MAX_HISTORY_SAMPLES);
    }

    #[test]
    fn test_top_tools_empty() {
        let app = App::new(8080);
        assert!(app.top_tools(5).is_empty());
    }

    #[test]
    fn test_uptime_str() {
        let mut app = App::new(8080);
        // No data → "--"
        assert_eq!(app.uptime_str(), "--");
        // With attribution data (3661s = 1h 1m)
        app.attribution = Some(api::AttributionResponse {
            total_tokens_processed: 0,
            total_tokens_saved: 0,
            total_compression_savings: 0,
            total_loop_savings: 0,
            total_cache_hits: 0,
            total_cache_saved_tokens: 0,
            savings_ratio: 0.0,
            compression_ratio: 0.0,
            total_calls: 0,
            blocked_calls: 0,
            error_calls: 0,
            cache_hits: 0,
            waste: api::WasteBreakdownItem {
                error_waste_tokens: 0,
                blocked_saved_tokens: 0,
                uncompressible_waste_tokens: 0,
                error_call_count: 0,
                blocked_call_count: 0,
                uncompressible_call_count: 0,
            },
            per_tool: vec![],
            uptime_seconds: 3661,
            estimated_cost_saved_usd: 0.0,
            estimated_cost_processed_usd: 0.0,
        });
        let uptime = app.uptime_str();
        assert!(uptime.contains("h"), "Expected uptime string to contain 'h', got: {uptime}");
    }
}
