//! Unified telemetry and savings tracking.
//!
//! Records both compression savings and loop prevention events in a
//! single ledger, with Prometheus metric exports.

mod event_log;
mod savings_ledger;

pub use event_log::{EventLog, ProxyEvent};
pub use savings_ledger::{
    SavingsLedger, SavingsReport,
    CompressionSaving, LoopBlockSaving,
};
