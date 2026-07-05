//! Security module: PII/secret redaction, trust scoring, and audit.
//!
//! Provides a [`RedactionEngine`] that detects and redacts sensitive
//! information (API keys, tokens, secrets) from tool call payloads
//! before they are sent to the LLM.

mod redaction;

pub use redaction::{
    RedactionEngine,
    RedactionConfig,
    RedactionReport,
    RedactedItem,
};
