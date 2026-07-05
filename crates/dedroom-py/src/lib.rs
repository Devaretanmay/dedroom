#![allow(unsafe_op_in_unsafe_fn)]
// PyO3's ? on map_err producing PyErr triggers a false positive.
#![allow(clippy::useless_conversion)]

//! Python bindings for DedrooM via PyO3.
//!
//! Note: The `clippy::useless_conversion` allow is intentionally kept because PyO3's
//! `?` operator on `map_err(|e| PyErr::from(e))` produces a false positive.
//!
//! Exposes a `DedrooM` class wrapping the Rust Pipeline, plus standalone
//! convenience functions for one-shot loop detection and compression.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use dedroom_core::compression::{CompressionPolicy, ContentRouter, ContentType};
use dedroom_core::compression::{
    compress_code as core_compress_code,
    compress_logs as core_compress_logs,
    compress_json_array as core_compress_json_array,
    compress_text as core_compress_text,
};
use dedroom_core::config::DedrooMConfig;
use dedroom_core::loop_detection::{LoopDetector, LoopVerdict};
use dedroom_core::pipeline::{Pipeline, ToolCall};
use dedroom_core::intelligence::store::{IntelligenceStore, InMemoryIntelligenceStore};
use std::sync::Arc;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Convert LoopVerdict to a Python-friendly code.
fn verdict_to_code(v: LoopVerdict) -> u8 {
    v.to_code()
}

/// Match a content-type name to the right compressor.
fn compress_by_type(content: &str, content_type: ContentType, retention: f64) -> String {
    match content_type {
        ContentType::JsonArray | ContentType::JsonObject => {
            core_compress_json_array(content, retention)
                .ok()
                .map(|r| r.content)
                .unwrap_or_else(|| content.to_string())
        }
        ContentType::Code => core_compress_code(content, "auto"),
        ContentType::Log => core_compress_logs(content),
        _ => core_compress_text(content),
    }
}

/// Parse a content-type string into a ContentType enum.
fn parse_content_type(s: &str) -> ContentType {
    match s.to_lowercase().trim() {
        "json_array" | "json" => ContentType::JsonArray,
        "json_object" => ContentType::JsonObject,
        "code" => ContentType::Code,
        "log" | "logs" => ContentType::Log,
        "text" | "txt" => ContentType::Text,
        "diff" => ContentType::Diff,
        "search_results" => ContentType::SearchResults,
        "tabular" => ContentType::Tabular,
        "html" => ContentType::Html,
        _ => ContentType::Text,
    }
}

// ── DedrooM class ──────────────────────────────────────────────────────────

/// DedrooM pipeline — loop detection and context compression.
///
/// Wraps the full Rust pipeline for use from Python.
#[pyclass(name = "DedrooM")]
pub struct DedrooM {
    pipeline: Pipeline,
    runtime: tokio::runtime::Runtime,
}

#[pymethods]
impl DedrooM {
    /// Create a new DedrooM pipeline from a YAML configuration string.
    ///
    /// Args:
    ///     config_yaml: YAML configuration string for loop detection and compression.
    #[new]
    fn new(config_yaml: &str) -> PyResult<Self> {
        let config = DedrooMConfig::from_yaml_str(config_yaml)
            .map_err(|e| PyValueError::new_err(format!("Invalid configuration YAML: {e}")))?;

        let store: Arc<dyn IntelligenceStore> = Arc::new(InMemoryIntelligenceStore::new());
        let pipeline = Pipeline::new(config, Some(store));

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create async runtime: {e}")))?;

        Ok(Self { pipeline, runtime })
    }

    /// Run loop detection on a tool call.
    ///
    /// Returns an integer verdict:
    ///     0 = Allow, 1 = Warn, 2 = BlockRetry, 3 = BlockHalt
    ///
    /// Args:
    ///     tool: The tool name (e.g. "write_file").
    ///     args: JSON string of tool arguments.
    fn verify(&mut self, tool: &str, args: &str) -> u8 {
        let verdict = self.pipeline.loop_detector.verify(tool, args);
        self.pipeline
            .loop_detector
            .record_result(tool, args, false);
        verdict.to_code()
    }

    /// Compress content using the configured pipeline.
    ///
    /// Args:
    ///     content: The content string to compress.
    ///     content_type: Optional hint ("json", "code", "log", "text", etc.).
    ///                   If empty, auto-detect.
    fn compress(&self, content: &str, content_type: &str) -> String {
        let router = &self.pipeline.content_router;
        let policy = &self.pipeline.compression_policy;
        let ct = if content_type.is_empty() {
            router.detect_type(content)
        } else {
            parse_content_type(content_type)
        };
        let retention = policy.smart_crusher_retention();
        compress_by_type(content, ct, retention)
    }

    /// Full pipeline: loop detect + compress + record.
    ///
    /// Returns a dict with verdict and compression results.
    ///
    /// Args:
    ///     tool: Tool name.
    ///     args: JSON string of tool arguments.
    ///     result: Tool output content (may be empty).
    ///     is_error: Whether the tool call resulted in an error.
    #[pyo3(signature = (tool, args, result="", is_error=false, agent_id=None, agent_thought=None))]
    fn process_tool<'py>(
        &mut self,
        tool: &str,
        args: &str,
        result: &str,
        is_error: bool,
        agent_id: Option<String>,
        agent_thought: Option<String>,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tool_call = ToolCall {
            name: tool.to_string(),
            args: args.to_string(),
            result: if result.is_empty() {
                None
            } else {
                Some(result.to_string())
            },
            is_error,
        };

        if let Some(thought) = agent_thought {
            self.pipeline.judgment_preservation.extract_reflection(&thought);
        }

        let pipeline_result = self
            .runtime
            .block_on(self.pipeline.process_tool_call(&tool_call, agent_id.as_deref()));

        let compressed = pipeline_result
            .compression_results
            .first()
            .map(|cr| cr.content.clone())
            .unwrap_or_default();

        let original_tokens = pipeline_result
            .compression_results
            .first()
            .map(|cr| cr.original_tokens)
            .unwrap_or(0);

        let compressed_tokens = pipeline_result
            .compression_results
            .first()
            .map(|cr| cr.compressed_tokens)
            .unwrap_or(0);

        let content_type_name = pipeline_result
            .compression_results
            .first()
            .map(|cr| cr.content_type.name())
            .unwrap_or("")
            .to_string();

        let verdict_code = verdict_to_code(pipeline_result.loop_verdict);
        let is_blocked = pipeline_result.loop_verdict.is_blocked();

        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("verdict", verdict_code)?;
        dict.set_item("verdict_name", format!("{:?}", pipeline_result.loop_verdict))?;
        dict.set_item("is_blocked", is_blocked)?;
        dict.set_item("compressed_content", compressed)?;
        dict.set_item("original_tokens", original_tokens)?;
        dict.set_item("compressed_tokens", compressed_tokens)?;
        dict.set_item("content_type", content_type_name)?;
        dict.set_item("injection_hint", pipeline_result.injection_hint)?;
        Ok(dict.into_any())
    }

    /// Get a snapshot of compression and loop-block savings.
    fn savings_report<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        let report = self.pipeline.savings_report();
        let dict = PyDict::new(py);
        dict.set_item("total_compression_savings", report.total_compression_savings).ok();
        dict.set_item("total_loop_savings", report.total_loop_savings).ok();
        dict.set_item("total_calls_blocked", report.total_calls_blocked).ok();
        dict.set_item("total_original_tokens", report.total_original_tokens).ok();
        dict.set_item("total_compressed_tokens", report.total_compressed_tokens).ok();
        dict.set_item("loop_block_by_tool", report.loop_block_by_tool).ok();
        dict.into_any()
    }

    /// Get a snapshot of loop detection state.
    fn loop_state<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        let summary = self.pipeline.loop_state_summary();
        let dict = PyDict::new(py);
        dict.set_item("total_calls", summary.total_calls).ok();
        dict.set_item("tool_counts", summary.tool_counts).ok();
        dict.set_item("current_max_repeats", summary.current_max_repeats).ok();
        dict.into_any()
    }
}

// ── Standalone functions ───────────────────────────────────────────────────

/// One-shot loop detection without creating a Pipeline instance.
///
/// Args:
///     tool: Tool name.
///     args: JSON string of tool arguments.
///     config_yaml: YAML configuration string.
///
/// Returns:
///     Integer verdict: 0 = Allow, 1 = Warn, 2 = BlockRetry, 3 = BlockHalt
#[pyfunction]
fn detect_loop(tool: &str, args: &str, config_yaml: &str) -> PyResult<u8> {
    let config = DedrooMConfig::from_yaml_str(config_yaml)
        .map_err(|e| PyValueError::new_err(format!("Invalid configuration YAML: {e}")))?;
    let mut detector = LoopDetector::new(&config.loop_detection);
    let verdict = detector.verify(tool, args);
    Ok(verdict.to_code())
}

/// One-shot content compression without creating a Pipeline instance.
///
/// Args:
///     content: Content string to compress.
///     config_yaml: YAML configuration string.
///
/// Returns:
///     Compressed content string.
#[pyfunction(name = "compress_text")]
fn compress_text_oneshot(content: &str, config_yaml: &str) -> PyResult<String> {
    let config = DedrooMConfig::from_yaml_str(config_yaml)
        .map_err(|e| PyValueError::new_err(format!("Invalid configuration YAML: {e}")))?;
    let router = ContentRouter::new(&config.compression.content_router);
    let policy = CompressionPolicy::new(&config.loop_compression_coupling);
    let content_type = router.detect_type(content);
    let retention = policy.smart_crusher_retention();
    Ok(compress_by_type(content, content_type, retention))
}

// ── Module registration ───────────────────────────────────────────────────

/// DedrooM Python bindings — imported as `dedroom._core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DedrooM>()?;
    m.add_function(wrap_pyfunction!(detect_loop, m)?)?;
    m.add_function(wrap_pyfunction!(compress_text_oneshot, m)?)?;
    Ok(())
}
