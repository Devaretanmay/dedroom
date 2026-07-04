//! Loop detection engine.
//!
//! Detects and blocks repeated tool calls using a multi-stage pipeline:
//! 1. Exact match against history (sub-microsecond)
//! 2. Volatile field stripping (ignore timestamps, request IDs)
//! 3. Auto-inference of volatile fields from call patterns
//! 4. Semantic similarity via embeddings (optional, configurable)
//!
//! Each stage catches cases the previous misses. The engine can also
//! adapt its threshold dynamically based on error rates.

mod engine;
mod history;
mod canonical;
mod adaptive;
mod semantic;

pub use engine::{LoopDetector, LoopVerdict, RuleEngine, CompiledRule, LoopStateSummary};
pub use history::HistoryTracker;
pub use canonical::{strip_volatile_fields, VolatileInferenceEngine};
pub use adaptive::AdaptiveThreshold;
pub use semantic::SemanticDetector;
