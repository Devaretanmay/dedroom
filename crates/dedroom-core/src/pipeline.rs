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
use crate::security::RedactionEngine;
use crate::attribution::{AttributionEngine, ToolCallAttribution};
use crate::intelligence::{MentorMode, TrustVerification, IntelligenceStore, InMemoryIntelligenceStore, JudgmentPreservation, CrossSessionLearning};
use std::sync::Arc;

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
    pub redaction_engine: RedactionEngine,
    pub attribution_engine: AttributionEngine,
    pub mentor: MentorMode,
    pub trust_verification: TrustVerification,
    pub judgment_preservation: JudgmentPreservation,
    pub cross_session_learning: CrossSessionLearning,
}

impl Pipeline {
    /// Create a new pipeline from configuration with default components.
    pub fn new(config: DedrooMConfig, intelligence_store: Option<Arc<dyn IntelligenceStore>>) -> Self {
        let store = intelligence_store.unwrap_or_else(|| Arc::new(InMemoryIntelligenceStore::new()));
        let ccr = Self::create_ccr_store(&config);
        let redaction_config = crate::security::RedactionConfig {
            enabled: config.security.redaction_enabled,
            entropy_threshold: config.security.entropy_threshold,
            entropy_detection: config.security.entropy_detection,
            context_detection: config.security.context_detection,
            audit_log: config.security.audit_log,
            custom_patterns: Vec::new(), // parsed from config.security.custom_patterns
            redact_strings: Vec::new(),
            sensitive_fields: None,
        };
        Self {
            config: config.clone(),
            loop_detector: LoopDetector::new(&config.loop_detection),
            content_router: ContentRouter::new(&config.compression.content_router),
            compression_policy: crate::compression::CompressionPolicy::new(
                &config.loop_compression_coupling,
            ),
            ccr_store: ccr,
            savings_ledger: SavingsLedger::new(),
            redaction_engine: RedactionEngine::new(redaction_config),
            attribution_engine: AttributionEngine::new(),
            mentor: MentorMode::new(true), // Enable mentor mode by default
            trust_verification: TrustVerification::new(store.clone()),
            judgment_preservation: JudgmentPreservation::new(),
            cross_session_learning: CrossSessionLearning::new(store),
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
    pub async fn process_tool_call(&mut self, tool: &ToolCall, agent_id: Option<&str>) -> PipelineResult {
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
                    if let Some(err_msg) = &tool.result {
                        self.cross_session_learning.record_failure(&tool.name, err_msg, "Error loop detected").await;
                    }
                    LoopState::ErrorLoop
                } else {
                    LoopState::Detected
                }
            }
            LoopVerdict::BlockHalt => {
                if tool.is_error {
                    if let Some(err_msg) = &tool.result {
                        self.cross_session_learning.record_failure(&tool.name, err_msg, "Fatal error loop detected").await;
                    }
                }
                LoopState::ErrorLoop
            }
        };
        self.compression_policy.set_loop_state(loop_state);

        // Track start time for latency
        let t0 = std::time::Instant::now();

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
            let mut hint = if self.compression_policy.should_inject_hint() {
                self.compression_policy.hint_template()
                    .map(|t| t.replace("{tool}", &tool.name))
            } else {
                None
            };
            
            // Check Mentor Mode for proactive coaching based on tilt_index
            let summary = self.loop_detector.state_summary();
            if let Some(mentor_hint) = self.mentor.generate_coaching_hint(summary.tilt_index) {
                if let Some(h) = &mut hint {
                    h.push_str("\n\n");
                    h.push_str(&mentor_hint);
                } else {
                    hint = Some(mentor_hint);
                }
            }
            
            // Query CrossSessionLearning for proactive hints for this tool
            let hints = self.cross_session_learning.get_proactive_hints(&tool.name).await;
            if !hints.is_empty() {
                let formatted_hints = format!("Wisdom from past sessions for `{}`:\n- {}", tool.name, hints.join("\n- "));
                if let Some(h) = &mut hint {
                    h.push_str("\n\n");
                    h.push_str(&formatted_hints);
                } else {
                    hint = Some(formatted_hints);
                }
            }
            // Record attribution for blocked call
            let blocked_tokens = estimate_tokens(tool.result.as_deref().unwrap_or(""));
            let latency = t0.elapsed().as_micros() as u64;
            self.attribution_engine.record(ToolCallAttribution {
                tool_name: tool.name.clone(),
                original_tokens: blocked_tokens,
                compressed_tokens: 0,
                tokens_saved: blocked_tokens,
                was_blocked: true,
                was_error: tool.is_error,
                was_cached: false,
                content_type: "blocked".into(),
                latency_us: latency,
            });
            if let Some(id) = agent_id {
                self.trust_verification.update_score(id, false).await;
            }
            return PipelineResult {
                messages: Vec::new(),
                loop_verdict: verdict,
                compression_results: Vec::new(),
                injection_hint: hint,
            };
        }

        // 4. Redact sensitive content before compression so secrets are
        //    never stored in CCR or sent to the LLM.
        let compress_input = if let Some(result) = tool.result.as_deref().filter(|r| !r.is_empty()) {
            let (redacted, report) = self.redaction_engine.redact(result);
            if report.total_redacted > 0 && self.config.security.audit_log {
                tracing::info!(
                    "Redacted {} sensitive item(s) from {} tool result",
                    report.total_redacted,
                    tool.name,
                );
            }
            redacted
        } else {
            String::new()
        };

        // 5. Compress the tool result (if present) using the redacted content
        let mut compression_results = Vec::new();
        let (original_tokens, compressed_tokens) = if !compress_input.is_empty() {
            let content_type = self.content_router.detect_type(&compress_input);
            let compressed = self.compress_content(&compress_input, content_type);
            if let Some(cr) = compressed {
                // Store redacted result in CCR (secrets never persist)
                let key = hash_tool_call(&tool.name, &tool.args);
                // Clone before moving into CCR so we can still read original_tokens
                let (orig, compr) = (cr.original_tokens, cr.compressed_tokens);
                self.ccr_store.put(
                    key,
                    compress_input,
                    tool.is_error,
                ).await;

                self.savings_ledger.record_compression(&CompressionSaving {
                    original_tokens: orig,
                    compressed_tokens: compr,
                    content_type: content_type.name().to_string(),
                });
                compression_results.push(cr);
                (orig, compr)
            } else {
                (0u64, 0u64)
            }
        } else {
            (0u64, 0u64)
        };

        // 6. Record attribution for the processed call
        let content_type = compression_results
            .first()
            .map(|cr| cr.content_type.name().to_string())
            .unwrap_or_else(|| "none".into());
        let latency = t0.elapsed().as_micros() as u64;
        let is_uncompressible = original_tokens == compressed_tokens && original_tokens > 0;

        if let Some(id) = agent_id {
            // Reward trust if the tool call succeeded and was not an error
            if !tool.is_error {
                self.trust_verification.update_score(id, true).await;
            } else {
                self.trust_verification.update_score(id, false).await;
            }
        }
        
        let tokens_saved = if is_uncompressible {
            // Still count as saved from redaction perspective but mark waste
            0
        } else {
            original_tokens.saturating_sub(compressed_tokens)
        };

        self.attribution_engine.record(ToolCallAttribution {
            tool_name: tool.name.clone(),
            original_tokens,
            compressed_tokens,
            tokens_saved,
            was_blocked: false,
            was_error: tool.is_error,
            was_cached: false,
            content_type,
            latency_us: latency,
        });

        // 6. Record result in loop detector
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

    /// Get the attribution report for ROI tracking.
    pub fn attribution_report(&self) -> crate::attribution::AttributionReport {
        self.attribution_engine.report()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_allows_first_call() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config, None);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()),
            is_error: false,
        };
        let result = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(result.loop_verdict, LoopVerdict::Allow);
    }

    #[tokio::test]
    async fn test_pipeline_blocks_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config, None);

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
        let r1 = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(r1.loop_verdict, LoopVerdict::Allow);
        let r2 = pipeline.process_tool_call(&tool, None).await;
        assert_eq!(r2.loop_verdict, LoopVerdict::Allow);

        // 3rd call should now be blocked (adaptive tightened the threshold)
        let r3 = pipeline.process_tool_call(&tool, None).await;
        assert!(r3.loop_verdict.is_blocked());
    }

    #[tokio::test]
    async fn test_pipeline_stores_ccr() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config, None);

        let tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"hello"}"#.into(),
            result: Some("result 1\nresult 2\nresult 3".into()),
            is_error: false,
        };
        let result = pipeline.process_tool_call(&tool, None).await;
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
        let mut pipeline = Pipeline::new(config, None);

        let tool = ToolCall {
            name: "read_file".into(),
            args: r#"{"path":"/tmp/big.txt"}"#.into(),
            result: Some("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7".into()),
            is_error: false,
        };
        let _ = pipeline.process_tool_call(&tool, None).await;

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

        let mut pipeline1 = Pipeline::new(config.clone(), None);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/e2e.txt"}"#.into(),
            result: Some("wrote 10 bytes".into()),
            is_error: true,
        };

        // Push 3 identical error calls — 3rd should be blocked
        let r1 = pipeline1.process_tool_call(&tool, None).await;
        assert_eq!(r1.loop_verdict, LoopVerdict::Allow);

        let r2 = pipeline1.process_tool_call(&tool, None).await;
        assert_eq!(r2.loop_verdict, LoopVerdict::Allow);

        let r3 = pipeline1.process_tool_call(&tool, None).await;
        assert!(r3.loop_verdict.is_blocked(), "3rd call should be blocked");

        // Process a tool with a result to populate CCR
        let search_tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"persistence test"}"#.into(),
            result: Some("result line 1\nresult line 2\nresult line 3".into()),
            is_error: false,
        };
        let r4 = pipeline1.process_tool_call(&search_tool, None).await;
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

        let mut pipeline2 = Pipeline::new(config, None);

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
        let r_after = pipeline2.process_tool_call(&tool, None).await;
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
        let r_new = pipeline2.process_tool_call(&new_tool, None).await;
        assert_eq!(
            r_new.loop_verdict,
            LoopVerdict::Allow,
            "Different args should be allowed"
        );
    }

    #[tokio::test]
    async fn test_pipeline_injects_hint_on_error_loop() {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config, None);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("error: no space left".into()),
            is_error: true,
        };

        // Error loop should trigger hint injection
        for _ in 0..4 {
            let _ = pipeline.process_tool_call(&tool, None).await;
        }
        let result = pipeline.process_tool_call(&tool, None).await;
        if result.loop_verdict.is_blocked() {
            assert!(result.injection_hint.is_some());
        }
    }
}
