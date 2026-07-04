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
        let ccr = Self::create_ccr_store(&config);
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

    /// Create the CCR store based on configuration and enabled features.
    ///
    /// When the `sqlite` feature is enabled and `backend` is `"sqlite"`, uses
    /// a persistent [`SqliteStore`]. Falls back to [`InMemoryStore`] in all
    /// other cases.
    fn create_ccr_store(config: &DedrooMConfig) -> CcrStore {
        #[cfg(feature = "sqlite")]
        if config.compression.ccr.backend == "sqlite" {
            let path = config
                .compression
                .ccr
                .path
                .as_deref()
                .unwrap_or("ccr.db");
            match crate::ccr::SqliteStore::new(path, config.compression.ccr.ttl_seconds) {
                Ok(sqlite) => {
                    tracing::info!("CCR using SQLite backend: {path}");
                    return CcrStore::new(
                        std::sync::Arc::new(sqlite),
                        config.compression.ccr.ttl_seconds,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to open SQLite CCR store at {path}: {e}. 
                         Falling back to in-memory."
                    );
                }
            }
        }

        CcrStore::new(
            std::sync::Arc::new(crate::ccr::InMemoryStore::new(
                config.compression.ccr.ttl_seconds,
            )),
            config.compression.ccr.ttl_seconds,
        )
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
                // JSON objects are not array-compressed; pass through as-is.
                Some(content.to_string())
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
            is_error: true,  // errors speed up detection via adaptive threshold
        };

        // With default config: max_repeats=3, error_reduction=1, min_repeats=2.
        // After each error the effective threshold drops by error_reduction (floored at
        // min_repeats=2).  So by the time 2 entries are in history the threshold is 2
        // and the 3rd call is blocked.
        let r1 = pipeline.process_tool_call(&tool).await;
        assert_eq!(r1.loop_verdict, LoopVerdict::Allow);
        let r2 = pipeline.process_tool_call(&tool).await;
        assert_eq!(r2.loop_verdict, LoopVerdict::Allow);

        // 3rd call should now be blocked (adaptive tightened the threshold)
        let r3 = pipeline.process_tool_call(&tool).await;
        assert!(r3.loop_verdict.is_blocked());
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

    // ── SQLite E2E persistence test (behind `sqlite` feature) ────────────

    /// Full end-to-end test: runs a Pipeline with SQLite-backed CCR + loop
    /// detection history, drops it (simulating a restart), then creates a new
    /// Pipeline pointing to the same DB files and verifies both CCR and loop
    /// detection state persisted.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_pipeline_sqlite_persistence_e2e() {
        let dir = tempfile::tempdir().unwrap();
        let ccr_db = dir.path().join("e2e_ccr.db");
        let history_db = dir.path().join("e2e_history.db");

        let yaml = format!(
            r#"
            loop_detection:
              max_repeats: 3
              history_backend: sqlite
              history_path: {history}
            compression:
              ccr:
                backend: sqlite
                path: {ccr}
                ttl_seconds: 3600
            "#,
            history = history_db.to_string_lossy(),
            ccr = ccr_db.to_string_lossy(),
        );

        let config = DedrooMConfig::from_yaml_str(&yaml).unwrap();

        // ── Session 1: populate both backends ────────────────────────────

        let mut pipeline1 = Pipeline::new(config.clone());

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/e2e.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()),
            is_error: true,
        };

        // Push 3 identical error calls — 3rd should be blocked
        let r1 = pipeline1.process_tool_call(&tool).await;
        assert_eq!(r1.loop_verdict, LoopVerdict::Allow);

        let r2 = pipeline1.process_tool_call(&tool).await;
        assert_eq!(r2.loop_verdict, LoopVerdict::Allow);

        let r3 = pipeline1.process_tool_call(&tool).await;
        assert!(r3.loop_verdict.is_blocked(), "3rd call should be blocked");

        // Process a tool with a result to populate CCR
        let search_tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"persistence test"}"#.into(),
            result: Some("result line 1\nresult line 2\nresult line 3".into()),
            is_error: false,
        };
        let r4 = pipeline1.process_tool_call(&search_tool).await;
        assert_eq!(r4.loop_verdict, LoopVerdict::Allow);

        // Verify state in session 1
        let summary1 = pipeline1.loop_state_summary();
        assert_eq!(summary1.total_calls, 4, "4 total calls in session 1");
        assert_eq!(*summary1.tool_counts.get("write_file").unwrap(), 3);
        assert_eq!(*summary1.tool_counts.get("search").unwrap(), 1);

        let ccr_key = hash_tool_call(&search_tool.name, &search_tool.args);
        let ccr_entry = pipeline1.ccr_store.get(&ccr_key).await;
        assert!(ccr_entry.is_some(), "CCR should have stored the search result");
        assert_eq!(ccr_entry.unwrap().original, "result line 1\nresult line 2\nresult line 3");

        // ── Simulate restart: drop pipeline1, create pipeline2 ────────────
        drop(pipeline1);

        let mut pipeline2 = Pipeline::new(config);

        // ── Session 2: verify persistence BEFORE making any calls ─────────

        // State should reflect session 1's 4 calls (3 write_file + 1 search)
        let summary2_before = pipeline2.loop_state_summary();
        assert_eq!(
            summary2_before.total_calls, 4,
            "4 history entries loaded from DB (3 write_file, 1 search)"
        );
        assert_eq!(*summary2_before.tool_counts.get("write_file").unwrap(), 3);
        assert_eq!(*summary2_before.tool_counts.get("search").unwrap(), 1);

        // Loop detection: same tool call should be blocked immediately
        // (history has 3 identical entries, threshold = max_repeats = 3)
        let r_after = pipeline2.process_tool_call(&tool).await;
        assert!(
            r_after.loop_verdict.is_blocked(),
            "Loop detection should persist: same call blocked after restart"
        );

        // CCR: stored data should be retrievable
        let ccr_entry_after = pipeline2.ccr_store.get(&ccr_key).await;
        assert!(
            ccr_entry_after.is_some(),
            "CCR data should persist across restarts"
        );
        assert_eq!(
            ccr_entry_after.unwrap().original,
            "result line 1\nresult line 2\nresult line 3"
        );

        // New tool call with different args should still be allowed
        let new_tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/new.txt"}"#.into(),
            result: Some("ok".into()),
            is_error: false,
        };
        let r_new = pipeline2.process_tool_call(&new_tool).await;
        assert_eq!(
            r_new.loop_verdict,
            LoopVerdict::Allow,
            "Different args should be allowed"
        );
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
