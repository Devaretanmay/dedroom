//! Pipeline orchestrator.
//!
//! Wires loop detection, compression, CCR, and telemetry into a single
//! request lifecycle.

use crate::config::DedrooMConfig;
use crate::loop_detection::{LoopDetector, LoopVerdict, LoopStateSummary};
use crate::compression::{ContentRouter, CompressionResult, ContentType};
use crate::compression::policy::{LoopState, determine_level, should_inject_hint, hint_template, retention_for_level};
use crate::compression::smart_crusher::{compress_json_array, compress_slice};
use crate::compression::code_compressor::{compress_code, detect_language};
use crate::compression::log_compressor::compress_logs;
use crate::compression::text_compressor::compress_text;
use crate::ccr::{CcrStore, hash_tool_call};
use crate::telemetry::SavingsLedger;
use crate::security::RedactionEngine;
use crate::healing::{self, SelfHealingEngine, HealingContext, instincts::InstinctsEngine};
use serde_json::Value;

/// A tool call intercepted by the pipeline.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: String,
    pub result: Option<String>,
    pub is_error: bool,
}

/// A message in the conversation.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Result of processing a request through the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub messages: Vec<Message>,
    pub loop_verdict: LoopVerdict,
    pub compression_results: Vec<CompressionResult>,
    pub injection_hint: Option<String>,
}

/// The main pipeline orchestrator.
pub struct Pipeline {
    pub config: DedrooMConfig,
    pub loop_detector: LoopDetector,
    pub content_router: ContentRouter,
    pub loop_state: LoopState,
    pub ccr_store: CcrStore,
    pub savings_ledger: SavingsLedger,
    pub redaction_engine: RedactionEngine,
    pub healing_engine: SelfHealingEngine,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("config", &self.config)
            .field("loop_detector", &self.loop_detector)
            .field("content_router", &self.content_router)
            .field("loop_state", &self.loop_state)
            .field("ccr_store", &self.ccr_store)
            .field("savings_ledger", &self.savings_ledger)
            .field("redaction_engine", &self.redaction_engine)
            .field("healing_engine", &self.healing_engine)
            .finish()
    }
}

impl Pipeline {
    pub fn new(config: DedrooMConfig) -> Self {
        let ccr = Self::create_ccr_store(&config);
        let redaction_config = crate::security::RedactionConfig {
            enabled: config.security.redaction_enabled,
            context_detection: config.security.context_detection,
            audit_log: config.security.audit_log,
            custom_patterns: Vec::new(),
            redact_strings: Vec::new(),
            sensitive_fields: None,
        };
        Self {
            config: config.clone(),
            loop_detector: LoopDetector::new(&config.loop_detection),
            content_router: ContentRouter::new(&config.compression.content_router),
            loop_state: LoopState::None,
            ccr_store: ccr,
            savings_ledger: SavingsLedger::new(),
            redaction_engine: RedactionEngine::new(redaction_config),
            healing_engine: Self::create_healing_engine(&config),
        }
    }

    fn create_healing_engine(config: &DedrooMConfig) -> SelfHealingEngine {
        let instincts = InstinctsEngine::from_config(&config.self_healing.instincts);
        SelfHealingEngine::new(
            config.self_healing.clone(),
            healing::memory::HealingMemory::new(),
            instincts,
        )
    }

    fn create_ccr_store(config: &DedrooMConfig) -> CcrStore {
        #[cfg(feature = "sqlite")]
        if config.compression.ccr.backend == "sqlite" {
            let path = config.compression.ccr.path.as_deref().unwrap_or("ccr.db");
            match crate::ccr::SqliteStore::new(path, config.compression.ccr.ttl_seconds) {
                Ok(sqlite) => {
                    tracing::info!("CCR using SQLite backend: {path}");
                    return CcrStore::new(std::sync::Arc::new(sqlite), config.compression.ccr.ttl_seconds);
                }
                Err(e) => {
                    tracing::warn!("Failed to open SQLite CCR store at {path}: {e}. Falling back to in-memory.");
                }
            }
        }
        CcrStore::new(
            std::sync::Arc::new(crate::ccr::InMemoryStore::new(config.compression.ccr.ttl_seconds)),
            config.compression.ccr.ttl_seconds,
        )
    }

    pub async fn process_tool_call(&mut self, tool: &ToolCall, _agent_id: Option<&str>) -> PipelineResult {
        let verdict = self.loop_detector.verify(&tool.name, &tool.args);

        let loop_state = match verdict {
            LoopVerdict::Allow => {
                let summary = self.loop_detector.state_summary();
                let count = summary.tool_counts.get(&tool.name).copied().unwrap_or(0);
                if count > 0 && count >= self.config.loop_detection.max_repeats as usize - 1 {
                    LoopState::Detected
                } else {
                    LoopState::None
                }
            }
            LoopVerdict::Warn | LoopVerdict::BlockRetry => {
                if tool.is_error { LoopState::ErrorLoop } else { LoopState::Detected }
            }
            LoopVerdict::BlockHalt => LoopState::ErrorLoop,
        };
        self.loop_state = loop_state;

        if verdict.is_blocked() {
            self.loop_detector.record_result(&tool.name, &tool.args, tool.is_error);
            let blocked_tokens = (tool.result.as_deref().unwrap_or("").len() as f64 / 4.0).ceil() as u64;
            self.savings_ledger.record_loop_block(1, blocked_tokens);

            // Build self-healing context
            let summary = self.loop_detector.state_summary();
            let healing_ctx = HealingContext::new(
                &tool.name,
                &tool.args,
                tool.is_error,
                tool.result.clone(),
                summary.tool_counts.get(&tool.name).copied().unwrap_or(0) as u32,
                summary.tilt_index,
                summary.total_calls,
            );

            // Try self-healing first — generates smarter, context-aware hints
            let healing_hint = self.healing_engine.generate_hint(&healing_ctx);

            let hint: Option<String> = if healing_hint.is_some() {
                healing_hint
            } else {
                // Fall back to generic hint + coaching
                let generic = if should_inject_hint(self.loop_state, &self.config.loop_compression_coupling) {
                    hint_template(self.loop_state, &self.config.loop_compression_coupling)
                        .map(|t| t.replace("{tool}", &tool.name))
                } else {
                    None
                };
                let coaching = if summary.tilt_index > 0.8 {
                    Some("Take a step back. You are repeatedly trying failing actions. Review the documentation or try a completely different approach.".to_string())
                } else if summary.tilt_index > 0.5 {
                    Some("You seem to be stuck. Consider searching the codebase for examples of how to do this correctly.".to_string())
                } else {
                    None
                };
                match (generic, coaching) {
                    (Some(mut g), Some(c)) => { g.push_str("\n\n"); g.push_str(&c); Some(g) }
                    (Some(g), None) => Some(g),
                    (None, Some(c)) => Some(c),
                    (None, None) => None,
                }
            };

            self.savings_ledger.record_tool_call(
                &tool.name, blocked_tokens, 0, true, tool.is_error, false,
            );
            return PipelineResult {
                messages: Vec::new(),
                loop_verdict: verdict,
                compression_results: Vec::new(),
                injection_hint: hint,
            };
        }

        let compress_input = if let Some(result) = tool.result.as_deref().filter(|r| !r.is_empty()) {
            let (redacted, report) = self.redaction_engine.redact(result);
            if report.total_redacted > 0 && self.config.security.audit_log {
                tracing::info!("Redacted {} sensitive item(s) from {} tool result", report.total_redacted, tool.name);
            }
            redacted
        } else {
            String::new()
        };

        let mut compression_results = Vec::new();
        let (original_tokens, compressed_tokens) = if !compress_input.is_empty() {
            let (content_type, parsed_json) = self.content_router.detect_type_with_value(&compress_input);
            let compressed = self.compress_content(&compress_input, content_type, parsed_json.as_ref());
            if let Some(cr) = compressed {
                let key = hash_tool_call(&tool.name, &tool.args);
                let (orig, compr) = (cr.original_tokens, cr.compressed_tokens);
                self.ccr_store.put(key, compress_input, tool.is_error).await;
                self.savings_ledger.record_compression(orig, compr);
                compression_results.push(cr);
                (orig, compr)
            } else {
                (0u64, 0u64)
            }
        } else {
            (0u64, 0u64)
        };

        self.savings_ledger.record_tool_call(
            &tool.name, original_tokens, compressed_tokens, false, tool.is_error, false,
        );
        self.loop_detector.record_result(&tool.name, &tool.args, tool.is_error);

        PipelineResult {
            messages: Vec::new(),
            loop_verdict: verdict,
            compression_results,
            injection_hint: None,
        }
    }

    fn compress_content(&self, content: &str, content_type: ContentType, parsed_json: Option<&Value>) -> Option<CompressionResult> {
        let original_tokens = (content.len() as f64 / 4.0).ceil() as u64;
        let level = determine_level(self.loop_state, &self.config.loop_compression_coupling);
        let retention = retention_for_level(level);
        let compressed = match content_type {
            ContentType::JsonArray => {
                // Reuse parsed value from content routing to avoid a second parse
                if let Some(Value::Array(arr)) = parsed_json {
                    compress_slice(arr, retention, arr.len()).ok().map(|r| r.content)
                } else {
                    compress_json_array(content, retention).ok().map(|r| r.content)
                }
            },
            ContentType::JsonObject => Some(content.to_string()),
            ContentType::Code => Some(if self.config.compression.compressors.code_compressor { compress_code(content, detect_language(content)) } else { content.to_string() }),
            ContentType::Log => Some(if self.config.compression.compressors.log_compressor { compress_logs(content) } else { content.to_string() }),
            ContentType::Text => Some(if self.config.compression.compressors.text_compressor { compress_text(content) } else { content.to_string() }),
            _ => Some(content.to_string()),
        };
        compressed.map(|c| CompressionResult {
            original_tokens,
            compressed_tokens: (c.len() as f64 / 4.0).ceil() as u64,
            content: c,
            content_type,
        })
    }

    /// Redact secrets and compress a tool-result payload WITHOUT recording
    /// telemetry. Returns the transformed content.
    ///
    /// This is the function the proxy uses to actually rewrite a request
    /// body before it is forwarded upstream — so redaction and compression
    /// are applied on the wire, not just computed as telemetry.
    pub fn transform_tool_output(&self, content: &str) -> String {
        if content.is_empty() {
            return String::new();
        }
        let (redacted, _report) = self.redaction_engine.redact(content);
        let (content_type, parsed) = self.content_router.detect_type_with_value(&redacted);
        let retention =
            retention_for_level(determine_level(self.loop_state, &self.config.loop_compression_coupling));
        let compressed = match content_type {
            ContentType::JsonArray => {
                if let Some(Value::Array(arr)) = parsed {
                    compress_slice(&arr, retention, arr.len())
                        .ok()
                        .map(|r| r.content)
                } else {
                    compress_json_array(&redacted, retention)
                        .ok()
                        .map(|r| r.content)
                }
            }
            ContentType::Code => {
                if self.config.compression.compressors.code_compressor {
                    Some(compress_code(&redacted, detect_language(&redacted)))
                } else {
                    None
                }
            }
            ContentType::Log => {
                if self.config.compression.compressors.log_compressor {
                    Some(compress_logs(&redacted))
                } else {
                    None
                }
            }
            ContentType::Text => {
                if self.config.compression.compressors.text_compressor {
                    Some(compress_text(&redacted))
                } else {
                    None
                }
            }
            _ => None,
        };
        compressed.unwrap_or(redacted)
    }

    pub fn loop_state_summary(&self) -> LoopStateSummary {
        self.loop_detector.state_summary()
    }

    pub fn savings_report(&self) -> crate::telemetry::SavingsReport {
        self.savings_ledger.report()
    }

    pub fn current_loop_state(&self) -> LoopState {
        self.loop_state
    }

    pub fn attribution_report(&self) -> crate::telemetry::AttributionReport {
        self.savings_ledger.attribution_report()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_allows_first_call() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);
        let tool = ToolCall {
            name: "write_file".into(), args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()), is_error: false,
        };
        let result = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(result.loop_verdict, LoopVerdict::Allow);
    }

    #[tokio::test]
    async fn test_pipeline_blocks_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);
        let tool = ToolCall {
            name: "write_file".into(), args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()), is_error: true,
        };
        let r1 = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(r1.loop_verdict, LoopVerdict::Allow);
        let r2 = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(r2.loop_verdict, LoopVerdict::Allow);
        let r3 = pipeline.process_tool_call(&tool, None).await;
        assert!(r3.loop_verdict.is_blocked());
    }

    #[tokio::test]
    async fn test_pipeline_stores_ccr() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);
        let tool = ToolCall {
            name: "search".into(), args: r#"{"query":"hello"}"#.into(),
            result: Some("result 1\nresult 2\nresult 3".into()), is_error: false,
        };
        let result = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(result.loop_verdict, LoopVerdict::Allow);
        let key = hash_tool_call(&tool.name, &tool.args);
        let entry = pipeline.ccr_store.get(&key).await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().original, "result 1\nresult 2\nresult 3");
    }

    #[tokio::test]
    async fn test_pipeline_tracks_savings() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);
        let tool = ToolCall {
            name: "read_file".into(), args: r#"{"path":"/tmp/big.txt"}"#.into(),
            result: Some("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7".into()), is_error: false,
        };
        let _ = pipeline.process_tool_call(&tool, None).await;
        let report = pipeline.savings_report();
        assert!(report.total_original_tokens > 0);
    }

    #[tokio::test]
    async fn test_pipeline_injects_hint_on_error_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);
        let tool = ToolCall {
            name: "write_file".into(), args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("error: no space left".into()), is_error: true,
        };
        for _ in 0..4 {
            let _ = pipeline.process_tool_call(&tool, None).await;
        }
        let result = pipeline.process_tool_call(&tool, None).await;
        if result.loop_verdict.is_blocked() {
            assert!(result.injection_hint.is_some());
        }
    }

    #[test]
    fn test_transform_tool_output_redacts_secrets() {
        let pipeline = Pipeline::new(DedrooMConfig::default());
        let payload = "api_key=sk-abcdefghijklmnopqrstuvwxyz0123456789 output done";
        let out = pipeline.transform_tool_output(payload);
        assert!(!out.contains("sk-abcdefghijklmnopqrstuvwxyz0123456789"),
            "secret must be redacted before forwarding");
    }

    #[test]
    fn test_transform_tool_output_compresses_json_array() {
        let pipeline = Pipeline::new(DedrooMConfig::default());
        let rows: Vec<&str> = std::iter::repeat(r#"{"level":"info","msg":"tick"}"#).take(20).collect();
        let payload = format!("[{}]", rows.join(","));
        let out = pipeline.transform_tool_output(&payload);
        assert!(out.len() < payload.len(), "repetitive array should shrink");
        assert!(out.starts_with('[') && out.ends_with(']'));
    }
}
