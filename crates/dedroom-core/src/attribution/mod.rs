//! Attribution engine — token tagging, waste categorization, and ROI tracking.
//!
//! Tracks every tool call through the pipeline, categorizing tokens saved,
//! wasted, or cached, and produces per-session and per-tool attribution
//! reports for the `/admin/attribution` endpoint.

mod engine;

pub use engine::{
    AttributionEngine, AttributionReport, ToolCallAttribution,
    WasteBreakdown, ToolBreakdown, AttributionTag,
};
