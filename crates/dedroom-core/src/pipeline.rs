//! Pipeline orchestrator.
//!
//! Wires loop detection, compression, CCR, and telemetry into a single
//! request lifecycle:
//!
//! `Receive → Cache Align → Loop Detect → Compress → Forward → Record`

use crate::config::DedrooMConfig;
use crate::loop_detection::{LoopDetector, LoopVerdict, LoopStateSummary};
use crate::compression::{ContentRouter, CompressionPolicy, CompressionResult, ContentType};
use crate::compression::policy::LoopState;
use crate::compression::smart_crusher::{compress_json_array, estimate_tokens};
use crate::compression::code_compressor::compress_code;
use crate::compression::log_compressor::compress_logs;
use crate::compression::text_compressor::compress_text;
use crate::ccr::{CcrStore, hash_tool_call};
use crate::telemetry::{SavingsLedger, CompressionSaving, LoopBlockSaving};

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
#[derive(Debug)]
pub struct Pipeline {
    pub config: DedrooMConfig,
    pub loop_detector: LoopDetector,
    pub content_router: ContentRouter,
    pub compression_policy: CompressionPolicy,
    pub ccr_store: CcrStore,
    pub savings_ledger: SavingsLedger,
}

impl Pipeline {
    /// Create a new pipeline from configuration with default components.
    pub fn new(config: DedrooMConfig) -> Self {
        let ccr = CcrStore::new(
            std::sync::Arc::new(crate::ccr::InMemoryStore::new(
                config.compression.ccr.ttl_seconds,
            )),
            config.compression.ccr.ttl_seconds,
        );
        Self {
            config: config.clone(),
            loop_detector: LoopDetector::new(&config.loop_detection),
            content_router: ContentRouter::new(&config.compression.content_router),
            compression_policy: crate::compression::CompressionPolicy::new(
                &config.loop_compression_coupling,
            ),
            ccr_store: ccr,
            savings_ledger: SavingsLedger::new(),
        }
    }

    /// Process a tool call through the full pipeline.
    ///
    /// Returns the loop verdict and (if allowed) the compressed result.
    pub async fn process_tool_call(&mut self, tool: &ToolCall) -> PipelineResult {
        // 1. Run loop detection
        let verdict = self.loop_detector.verify(&tool.name, &tool.args);

        // 2. Update compression policy based on loop state
        let loop_state = match verdict {
            LoopVerdict::Allow => {
                // Check if we're approaching a loop
                let summary = self.loop_detector.state_summary();
                let count = summary.tool_counts.get(&tool.name).copied().unwrap_or(0);
                if count > 0 && count >= self.config.loop_detection.max_repeats as usize - 1 {
                    LoopState::Detected
                } else {
                    LoopState::None
                }
            }
            LoopVerdict::Warn | LoopVerdict::BlockRetry => {
                if tool.is_error {
                    LoopState::ErrorLoop
                } else {
                    LoopState::Detected
                }
            }
            LoopVerdict::BlockHalt => LoopState::ErrorLoop,
        };
        self.compression_policy.set_loop_state(loop_state);

        // 3. If blocked, record and return early
        if verdict.is_blocked() {
            self.loop_detector.record_result(&tool.name, &tool.args, tool.is_error);
            self.savings_ledger.record_loop_block(&LoopBlockSaving {
                tool_name: tool.name.clone(),
                calls_prevented: 1,
                estimated_tokens_saved: estimate_tokens(
                    tool.result.as_deref().unwrap_or(""),
                ),
            });
            let hint = if self.compression_policy.should_inject_hint() {
                self.compression_policy.hint_template()
                    .map(|t| t.replace("{tool}", &tool.name))
            } else {
                None
            };
            return PipelineResult {
                messages: Vec::new(),
                loop_verdict: verdict,
                compression_results: Vec::new(),
                injection_hint: hint,
            };
        }

        // 4. Compress the tool result (if present)
        let mut compression_results = Vec::new();
        if let Some(result) = tool.result.as_deref().filter(|r| !r.is_empty()) {
            let content_type = self.content_router.detect_type(result);
            let compressed = self.compress_content(result, content_type);
            if let Some(cr) = compressed {
                // Store original in CCR
                let key = hash_tool_call(&tool.name, &tool.args);
                self.ccr_store.put(
                    key,
                    result.to_string(),
                    tool.is_error,
                ).await;

                self.savings_ledger.record_compression(&CompressionSaving {
                    original_tokens: cr.original_tokens,
                    compressed_tokens: cr.compressed_tokens,
                    content_type: content_type.name().to_string(),
                });
                compression_results.push(cr);
            }
        }

        // 5. Record result in loop detector
        self.loop_detector.record_result(&tool.name, &tool.args, tool.is_error);

        PipelineResult {
            messages: Vec::new(),
            loop_verdict: verdict,
            compression_results,
            injection_hint: None,
        }
    }

    /// Compress content using the appropriate compressor.
    fn compress_content(&self, content: &str, content_type: ContentType) -> Option<CompressionResult> {
        let original_tokens = estimate_tokens(content);
        let retention = self.compression_policy.smart_crusher_retention();

        let compressed = match content_type {
            ContentType::JsonArray => {
                compress_json_array(content, retention).ok().map(|r| r.content)
            }
            ContentType::JsonObject => {
                compress_json_array(content, retention).ok().map(|r| r.content)
            }
            ContentType::Code => {
                Some(if self.config.compression.compressors.code_compressor {
                    compress_code(content, "auto")
                } else {
                    content.to_string()
                })
            }
            ContentType::Log => {
                Some(if self.config.compression.compressors.log_compressor {
                    compress_logs(content)
                } else {
                    content.to_string()
                })
            }
            ContentType::Text => {
                Some(if self.config.compression.compressors.text_compressor {
                    compress_text(content)
                } else {
                    content.to_string()
                })
            }
            _ => {
                // Pass through for unhandled types
                Some(content.to_string())
            }
        };

        compressed.map(|c| {
            let compressed_tokens = estimate_tokens(&c);
            CompressionResult {
                original_tokens,
                compressed_tokens,
                content: c,
                content_type,
            }
        })
    }

    /// Get a snapshot of loop detection state.
    pub fn loop_state_summary(&self) -> LoopStateSummary {
        self.loop_detector.state_summary()
    }

    /// Get the savings report.
    pub fn savings_report(&self) -> crate::telemetry::SavingsReport {
        self.savings_ledger.report()
    }

    /// Get the current compression policy loop state.
    pub fn current_loop_state(&self) -> LoopState {
        self.compression_policy.loop_state()
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
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()),
            is_error: false,
        };
        let result = pipeline.process_tool_call(&tool).await;
        assert_eq!(result.loop_verdict, LoopVerdict::Allow);
    }

    #[tokio::test]
    async fn test_pipeline_blocks_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()),
            is_error: true,  // errors speed up detection
        };

        // First 3 calls
        for _ in 0..3 {
            let result = pipeline.process_tool_call(&tool).await;
            assert_eq!(result.loop_verdict, LoopVerdict::Allow);
        }

        // 4th call should be blocked
        let result = pipeline.process_tool_call(&tool).await;
        assert!(result.loop_verdict.is_blocked());
    }

    #[tokio::test]
    async fn test_pipeline_stores_ccr() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"hello"}"#.into(),
            result: Some("result 1\nresult 2\nresult 3".into()),
            is_error: false,
        };
        let result = pipeline.process_tool_call(&tool).await;
        assert_eq!(result.loop_verdict, LoopVerdict::Allow);

        // Check CCR has stored the original
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
            name: "read_file".into(),
            args: r#"{"path":"/tmp/big.txt"}"#.into(),
            result: Some("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7".into()),
            is_error: false,
        };
        let _ = pipeline.process_tool_call(&tool).await;

        let report = pipeline.savings_report();
        assert!(report.total_original_tokens > 0);
    }

    #[tokio::test]
    async fn test_pipeline_injects_hint_on_error_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("error: no space left".into()),
            is_error: true,
        };

        // Error loop should trigger hint injection
        for _ in 0..4 {
            let _ = pipeline.process_tool_call(&tool).await;
        }
        let result = pipeline.process_tool_call(&tool).await;
        if result.loop_verdict.is_blocked() {
            assert!(result.injection_hint.is_some());
        }
    }
}
