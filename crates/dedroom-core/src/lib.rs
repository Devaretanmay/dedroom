//! DedrooM — unified agent runtime.
//!
//! Two capabilities, one pipeline:
//! - **Loop detection** — detect and block repeated tool calls before
//!   they reach the LLM API (sub-microsecond, multi-stage detection).
//! - **Context compression** — compress tool outputs, logs, code, JSON,
//!   and text by 60–95% before sending to the LLM.
//!
//! # Quick start
//!
//! ```rust
//! use dedroom_core::config::DedrooMConfig;
//! use dedroom_core::loop_detection::LoopDetector;
//!
//! let config = DedrooMConfig::from_yaml_str("loop_detection:\n  max_repeats: 3")
//!     .unwrap();
//! let mut detector = LoopDetector::new(&config.loop_detection);
//! let verdict = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
//! assert_eq!(verdict, dedroom_core::loop_detection::LoopVerdict::Allow);
//! ```

pub mod config;
pub mod loop_detection;
pub mod compression;
pub mod ccr;
pub mod embedding;
pub mod telemetry;
pub mod security;
pub mod attribution;
pub mod pipeline;
pub mod intelligence;

// Re-export the top-level API
pub use config::DedrooMConfig;
pub use pipeline::Pipeline;
