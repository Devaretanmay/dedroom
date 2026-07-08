//! Core pipeline interception logic.
//!
//! Orchestrates the DedrooM pipeline on every proxied request:
//! 1. Parse the request body to extract tool calls
//! 2. Run each tool call through `Pipeline::process_tool_call`
//! 3. Block or allow based on loop verdict (or shadow-log in ghost mode)
//! 4. Forward allowed calls to upstream with compressed context
//! 5. Record telemetry and emit events to the NDJSON log

use axum::http::HeaderMap;
use dedroom_core::ccr::hash_tool_call;
use dedroom_core::pipeline::{Pipeline, ToolCall};
use dedroom_core::telemetry::{EventLog, ProxyEvent};
use serde_json::{json, Value};

use crate::proxy::ProxyConfig;

/// The target LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Anthropic,
}

/// A tool invocation extracted from the request messages.
#[derive(Debug, Clone)]
pub struct ExtractedTool {
    pub id: Option<String>,
    pub name: String,
    pub args: String,
    pub result: Option<String>,
    pub is_error: bool,
}

/// Extract tool calls from an OpenAI-formatted request body.
///
/// Looks for `tool_calls` entries in assistant messages.
pub fn extract_tool_calls_openai(body: &Value) -> Vec<ExtractedTool> {
    let mut tools = Vec::new();

    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            // Assistant messages may contain tool_calls
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                for tc in tool_calls {
                    let id = tc
                        .get("id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let args = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    tools.push(ExtractedTool {
                        id,
                        name,
                        args,
                        result: None,
                        is_error: false,
                    });
                }
            }

            // Tool result messages contain the actual output
            if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string());
                
                let content = msg
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let is_error = msg
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                
                // Match by tool_call_id first, fallback to positional matching (first unmatched tool)
                let matched_tool = if let Some(ref id) = tool_call_id {
                    tools.iter_mut().find(|t| t.id.as_ref() == Some(id) && t.result.is_none())
                } else {
                    None
                };
                
                let tool_to_update = match matched_tool {
                    Some(t) => Some(t),
                    None => tools.iter_mut().find(|t| t.result.is_none())
                };

                if let Some(tool) = tool_to_update {
                    tool.result = Some(content.to_string());
                    tool.is_error = is_error;
                }
            }
        }
    }

    tools
}

/// Extract tool calls from an Anthropic-formatted request body.
///
/// Anthropic uses `content` blocks with `type: "tool_use"` / `type: "tool_result"`.
pub fn extract_tool_calls_anthropic(body: &Value) -> Vec<ExtractedTool> {
    let mut tools = Vec::new();

    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            if let Some(content_blocks) = msg.get("content").and_then(|c| c.as_array()) {
                for block in content_blocks {
                    let block_type = block
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");

                    match block_type {
                        "tool_use" => {
                            let name = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let args = block
                                .get("input")
                                .map(|i| i.to_string())
                                .unwrap_or_else(|| "{}".to_string());
                            tools.push(ExtractedTool { id: None,
                                name,
                                args,
                                result: None,
                                is_error: false,
                            });
                        }
                        "tool_result" => {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("");
                            let content_str: String = match block.get("content") {
                                Some(c) if c.is_string() => c.as_str().unwrap_or("").to_string(),
                                Some(c) => c.to_string(),
                                None => String::new(),
                            };
                            let is_error = block
                                .get("is_error")
                                .and_then(|e| e.as_bool())
                                .unwrap_or(false);
                            // Match tool results by id or position
                            if !tool_use_id.is_empty() {
                                // TODO: map id -> tool
                            }
                            if let Some(tool) = tools
                                .iter_mut()
                                .rev()
                                .find(|t| t.result.is_none())
                            {
                                tool.result = Some(content_str.to_string());
                                tool.is_error = is_error;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    tools
}

/// Process a set of extracted tools through the DedrooM pipeline.
///
/// When `shadow_mode` is `true`, the pipeline runs in ghost mode — every
/// tool call is processed, loop verdicts are logged to the NDJSON event
/// stream, but NO tool calls are ever blocked. The request always passes
/// through as if nothing happened.
///
/// Returns `(allowed_tools, blocked_response)` where `blocked_response` is
/// `Some` when at least one tool call was blocked AND we are NOT in shadow
/// mode. In shadow mode, `blocked_response` is always `None`.
pub async fn process_tools_through_pipeline(
    pipeline: &mut Pipeline,
    tools: Vec<ExtractedTool>,
    event_log: Option<&EventLog>,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    shadow_mode: bool,
) -> (Vec<ToolCall>, Option<Value>) {
    // Drain pending healing outcomes from the PREVIOUS request.
    // These were stored by `SelfHealingEngine::generate_hint` when it
    // injected a healing hint. We evaluate them against THIS request's
    // results to determine if the mutation succeeded.
    let pending_outcomes = pipeline.healing_engine.drain_pending_outcomes();

    let mut allowed_tools = Vec::new();
    let mut blocked_tools = Vec::new();

    for tool in tools {
        let t0 = std::time::Instant::now();
        let tc = ToolCall {
            name: tool.name.clone(),
            args: tool.args,
            result: tool.result.clone(),
            is_error: tool.is_error,
        };

        let result = pipeline.process_tool_call(&tc, agent_id).await;
        let latency_us = t0.elapsed().as_micros() as u64;

        // Compute tilt_index: how close this tool is to being blocked
        // (repeat_count / max_repeats), clamped to [0.0, 1.0].
        let summary = pipeline.loop_state_summary();
        let repeat_count = summary
            .tool_counts
            .get(&tc.name)
            .copied()
            .unwrap_or(0);
        let max_repeats = pipeline.config.loop_detection.max_repeats;
        let tilt_index = if max_repeats > 0 {
            Some((repeat_count as f64 / max_repeats as f64).min(1.0))
        } else {
            None
        };

        // Extract compression stats if available, or 100% if blocked
        let (original_tokens, compressed_tokens, compression_ratio) = if result.loop_verdict.is_blocked() {
            let orig = (tool.result.as_deref().unwrap_or("").len() as f64 / 4.0).ceil() as u64;
            (Some(orig), Some(0), Some(1.0))
        } else {
            result
                .compression_results
                .first()
                .map(|cr| {
                    let ratio = if cr.original_tokens > 0 {
                        Some(1.0 - cr.compressed_tokens as f64 / cr.original_tokens as f64)
                    } else {
                        None
                    };
                    (
                        Some(cr.original_tokens),
                        Some(cr.compressed_tokens),
                        ratio,
                    )
                })
                .unwrap_or((None, None, None))
        };

        // Determine verdict string
        let verdict_str = if result.loop_verdict.is_blocked() {
            if result.injection_hint.is_some() {
                "inject"
            } else {
                "block"
            }
        } else {
            "allow"
        };

        // Compute args_hash using the same BLAKE3 hash as CCR
        let args_hash = hash_tool_call(&tc.name, &tc.args).to_hex().to_string();

        // Emit event if event_log is available
        if let Some(log) = event_log {
            log.record(ProxyEvent {
                timestamp: EventLog::now_millis(),
                session_id: session_id.map(|s| s.to_string()),
                agent_id: agent_id.map(|s| s.to_string()),
                tool_name: tc.name.clone(),
                args_hash: Some(args_hash),
                verdict: verdict_str.to_string(),
                compression_ratio,
                original_tokens,
                compressed_tokens,
                tilt_index,
                latency_us,
            });
        }

        if result.loop_verdict.is_blocked() {
            if shadow_mode {
                // Ghost mode: log the block but still allow the tool through
                tracing::debug!(
                    "[shadow] would block {} ({:?}) — passing through",
                    tc.name,
                    result.loop_verdict
                );
                allowed_tools.push(tc.clone());
            }
            blocked_tools.push((tc, result));
        } else {
            allowed_tools.push(tc);
        }
    }

    // Evaluate pending healing outcomes from the previous request.
    // A tool is a "success" if it appears in allowed_tools but NOT in
    // blocked_tools, meaning the agent broke the loop.
    for (tool_name, strategy) in pending_outcomes {
        let is_blocked = blocked_tools.iter().any(|(tc, _)| tc.name == tool_name);
        let is_allowed = allowed_tools.iter().any(|tc| tc.name == tool_name);
        if is_blocked || is_allowed {
            let success = is_allowed && !is_blocked;
            pipeline.healing_engine.report_outcome(&tool_name, &strategy, success);
        }
    }

    let blocked_response = if blocked_tools.is_empty() || shadow_mode {
        // In shadow mode we NEVER return a blocked response
        None
    } else {
        Some(build_blocked_response(&blocked_tools, pipeline))
    };

    (allowed_tools, blocked_response)
}

/// Build a structured response indicating which tool calls were blocked.
fn build_blocked_response(
    blocked: &[(ToolCall, dedroom_core::pipeline::PipelineResult)],
    pipeline: &Pipeline,
) -> Value {
    let details: Vec<Value> = blocked
        .iter()
        .map(|(tc, pr)| {
            json!({
                "tool": tc.name,
                "verdict": format!("{:?}", pr.loop_verdict),
                "reason": "Repeated tool call detected by DedrooM loop detection",
                "injection_hint": pr.injection_hint,
            })
        })
        .collect();

    json!({
        "error": "tool_loop_detected",
        "message": format!(
            "{} tool call(s) blocked by DedrooM loop detection",
            blocked.len()
        ),
        "blocked_calls": details,
        "loop_state_summary": json!({
            "total_calls": pipeline.loop_state_summary().total_calls,
            "tool_counts": pipeline.loop_state_summary().tool_counts,
        }),
    })
}

/// Forward an allowed request to the upstream provider.
///
/// Forces non-streaming upstream and returns the upstream response body.
pub async fn forward_to_upstream(
    headers: &HeaderMap,
    body: Value,
    provider: Provider,
    proxy_config: &ProxyConfig,
) -> Result<Value, String> {
    let (base_url, path) = match provider {
        Provider::OpenAI => (&proxy_config.openai_base_url, "/v1/chat/completions"),
        Provider::Anthropic => (&proxy_config.anthropic_base_url, "/v1/messages"),
    };

    let base_clean = base_url.trim_end_matches('/');
    let base_clean = if base_clean.ends_with("/v1") && path.starts_with("/v1/") {
        base_clean.strip_suffix("/v1").unwrap()
    } else {
        base_clean
    };
    
    let url = format!("{}{}", base_clean, path);

    // Force non-streaming — always set stream=false
    let mut upstream_body = body.clone();
    if let Some(obj) = upstream_body.as_object_mut() {
        obj.insert("stream".to_string(), json!(false));
    }

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&upstream_body);

    // Use proxy's API key if available, otherwise forward client's header
    if let Some(ref key) = proxy_config.api_key {
        let auth_header = match provider {
            Provider::OpenAI => format!("Bearer {}", key),
            Provider::Anthropic => format!("Bearer {}", key),
        };
        req = req.header("authorization", &auth_header);
    } else if let Some(auth) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        req = req.header("authorization", auth);
    }

    // Forward content-type
    if let Some(ct) = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        req = req.header("content-type", ct);
    }

    // Anthropic needs the anthropic-version header
    if provider == Provider::Anthropic {
        req = req.header("anthropic-version", "2023-06-01");
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                serde_json::from_str(&body_text).map_err(|e| e.to_string())
            } else {
                Err(format!("upstream {} error: {} — {}", status.as_u16(), status.as_str(), body_text))
            }
        }
        Err(e) => Err(format!("upstream request failed: {e}")),
    }
}

/// Intercept an upstream response and pass it through the pipeline for
/// telemetry recording. Returns modified response data.
pub async fn record_upstream_response(
    pipeline: &mut Pipeline,
    upstream_response: &Value,
    tools: &[ToolCall],
) -> Value {
    // Record tool results in pipeline (compression telemetry)
    for tool in tools {
        if let Some(_result) = &tool.result {
            let _ = pipeline.process_tool_call(tool, None).await;
        }
    }

    // Inject any pipeline hints into the response
    let mut resp = upstream_response.clone();
    if let Some(obj) = resp.as_object_mut() {
        // Add telemetry metadata
        let report = pipeline.savings_report();
        obj.insert(
            "_dedroom".to_string(),
            json!({
                "total_compression_savings": report.total_compression_savings,
                "total_loop_savings": report.total_loop_savings,
                "total_calls_blocked": report.total_calls_blocked,
            }),
        );
    }

    resp
}

/// Build an SSE event stream from a completed (non-streaming) response.
///
/// This wraps the full JSON response as individual SSE data events,
/// mimicking a streaming reply for clients that expect one.
pub fn wrap_as_sse(response: Value) -> String {
    let mut events = String::new();

    // Send the full response as a single SSE data event
    let line = response.to_string();
    for chunk in line.as_bytes().chunks(4096) {
        let fragment = std::str::from_utf8(chunk).unwrap_or("");
        events.push_str(&format!("data: {fragment}\n"));
    }
    events.push_str("data: [DONE]\n\n");

    events
}

/// Determine the session ID from request headers.
pub fn get_session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Determine the agent identifier from request headers.
///
/// Checks `x-agent-id` first, falls back to `User-Agent`.
pub fn get_agent_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-agent-id")
        .or_else(|| headers.get("user-agent"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dedroom_core::loop_detection::LoopVerdict;
    use serde_json::json;
    use std::io::BufRead;

    #[test]
    fn test_extract_tool_calls_openai() {
        let body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "function": {
                                "name": "write_file",
                                "arguments": "{\"path\":\"/tmp/test.txt\",\"content\":\"hello\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "name": "write_file",
                    "content": "File written successfully"
                }
            ]
        });

        let tools = extract_tool_calls_openai(&body);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "write_file");
        assert_eq!(tools[0].result.as_deref(), Some("File written successfully"));
    }

    #[test]
    fn test_extract_tool_calls_anthropic() {
        let body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "tu_1",
                            "name": "read_file",
                            "input": {"path": "/tmp/test.txt"}
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "tu_1",
                            "content": "file contents here"
                        }
                    ]
                }
            ]
        });

        let tools = extract_tool_calls_anthropic(&body);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read_file");
    }

    #[test]
    fn test_build_blocked_response() {
        use dedroom_core::DedrooMConfig;

        let config = DedrooMConfig::default();
        let pipeline = Pipeline::new(config);

        let blocked = vec![(
            ToolCall {
                name: "write_file".to_string(),
                args: "{}".to_string(),
                result: Some("data".to_string()),
                is_error: false,
            },
            dedroom_core::pipeline::PipelineResult {
                messages: vec![],
                loop_verdict: LoopVerdict::BlockRetry,
                compression_results: vec![],
                injection_hint: Some("Try a different approach".to_string()),
            },
        )];

        let resp = build_blocked_response(&blocked, &pipeline);
        assert_eq!(resp["error"], "tool_loop_detected");
        assert_eq!(
            resp["blocked_calls"][0]["tool"],
            "write_file"
        );
    }

    /// Verifies that `process_tools_through_pipeline` with `shadow_mode=true`:
    /// 1. Returns ALL tool calls as "allowed" even when loop detection would block them
    /// 2. Returns `None` for blocked_response (no 429 response is returned)
    /// 3. Writes events to the NDJSON event log capturing the real verdicts
    #[tokio::test]
    async fn test_shadow_mode_passes_through_blocked_calls() {
        use dedroom_core::DedrooMConfig;

        // ── Setup: low max_repeats so we hit a block quickly ────────────

        let yaml = r#"
            loop_detection:
              max_repeats: 2
        "#;
        let config = DedrooMConfig::from_yaml_str(yaml).unwrap();
        let mut pipeline = Pipeline::new(config);

        // ── Setup: tempdir-backed event log ─────────────────────────────

        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("shadow-test.ndjson");
        let event_log = EventLog::start_with_path(log_path.clone());

        // ── Build 4 identical error-producing tool calls ────────────────
        // With max_repeats=3 and errors, calls 3+ should be blocked.

        let tools: Vec<ExtractedTool> = (0..4)
            .map(|_| ExtractedTool { id: None,
                name: "write_file".into(),
                args: r#"{"path":"/tmp/test.txt"}"#.into(),
                result: Some("error: permission denied".into()),
                is_error: true,
            })
            .collect();

        // ── Process with shadow_mode = true ─────────────────────────────

        let (allowed_tools, blocked_response) = process_tools_through_pipeline(
            &mut pipeline,
            tools,
            Some(&event_log),
            Some("test-session"),
            Some("test-agent"),
            true, // shadow_mode = true
        )
        .await;

        // ── Assertions ──────────────────────────────────────────────────

        // 1. ALL 4 tools are returned as allowed (shadow bypasses blocks)
        assert_eq!(
            allowed_tools.len(),
            4,
            "shadow mode should return ALL tools as allowed, not just the first 2"
        );
        assert_eq!(allowed_tools[0].name, "write_file");
        assert_eq!(allowed_tools[3].name, "write_file");

        // 2. blocked_response is None (no 429 returned in shadow mode)
        assert!(
            blocked_response.is_none(),
            "shadow mode must NOT return a blocked response"
        );

        // 3. The event log captured all 4 events with correct verdicts
        // Give the background thread time to flush
        std::thread::sleep(std::time::Duration::from_millis(150));

        let file = std::fs::File::open(&log_path).unwrap();
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

        assert_eq!(lines.len(), 4, "event log should have 4 entries");

        // Parse each line and check verdicts
        let events: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        // First call allowed
        assert_eq!(
            events[0]["verdict"].as_str().unwrap(),
            "allow",
            "first tool call should be allowed"
        );
        // Second call allowed (history has 1 entry, threshold still high)
        assert_eq!(
            events[1]["verdict"].as_str().unwrap(),
            "allow",
            "second tool call should be allowed"
        );
        // Third+ calls blocked — error-causing calls trigger the injection
        // hint, so verdict is "inject" rather than bare "block"
        assert_eq!(
            events[2]["verdict"].as_str().unwrap(),
            "inject",
            "third tool call should be blocked with injection hint"
        );
        assert_eq!(
            events[3]["verdict"].as_str().unwrap(),
            "inject",
            "fourth tool call should be blocked with injection hint"
        );

        // Verify session_id and agent_id are in the log
        assert_eq!(
            events[0]["session_id"].as_str().unwrap(),
            "test-session"
        );
        assert_eq!(
            events[0]["agent_id"].as_str().unwrap(),
            "test-agent"
        );

        // Verify tilt_index: should increase as repeat count grows
        let tilt_0 = events[0]["tilt_index"].as_f64().unwrap();
        let tilt_3 = events[3]["tilt_index"].as_f64().unwrap();
        assert!(
            tilt_3 > tilt_0,
            "tilt_index should increase from first to fourth call ({} -> {})",
            tilt_0,
            tilt_3
        );

        // args_hash should be present and deterministic for identical calls
        let hash_0 = events[0]["args_hash"].as_str().unwrap();
        let hash_1 = events[1]["args_hash"].as_str().unwrap();
        assert_eq!(hash_0, hash_1, "identical calls should have same args_hash");
        assert!(!hash_0.is_empty(), "args_hash should be a non-empty hex string");
    }

    /// Verifies the self-healing outcome feedback loop:
    ///
    /// 1. Request N: Tool A loops → blocked with healing hint → strategy stored
    /// 2. Request N+1: Tool A returns with different args → allowed
    /// 3. Pending outcome is drained at start of Request N+1
    /// 4. After processing, the outcome is evaluated: Tool A is allowed, not blocked → SUCCESS
    /// 5. `report_outcome()` records it in memory
    #[tokio::test]
    async fn test_healing_outcome_feedback_loop_reports_success() {
        use dedroom_core::DedrooMConfig;

        // ── Setup: low max_repeats so we hit a block quickly ────────────
        let yaml = r#"
            loop_detection:
              max_repeats: 2
            self_healing:
              enabled: true
              mode: Balanced
        "#;
        let config = DedrooMConfig::from_yaml_str(yaml).unwrap();
        let mut pipeline = Pipeline::new(config);

        // ── Request 1-3: Same tool loops until blocked ──────────────────
        let looping_tool = ExtractedTool { id: None,
            name: "list_files".into(),
            args: r#"{"path":"/"}"#.into(),
            result: Some("error: permission denied".into()),
            is_error: true,
        };

        // Call 1: allowed (first occurrence)
        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![looping_tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 1);
        assert!(blocked.is_none());

        // Call 2: allowed (second occurrence, threshold not yet hit)
        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![looping_tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 1);
        assert!(blocked.is_none());

        // Call 3: BLOCKED with healing hint (max_repeats=2 → 3rd call blocked)
        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![looping_tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 0, "3rd call should be blocked");
        assert!(blocked.is_some(), "3rd call should return blocked response");

        // Memory should be empty — no outcome evaluated yet
        assert_eq!(pipeline.healing_engine.total_attempts(), 0);

        // ── Request 4: Tool comes back with DIFFERENT args (agent adapted) ──
        let adapted_tool = ExtractedTool { id: None,
            name: "list_files".into(),
            args: r#"{"path":"/tmp","pattern":"*.txt"}"#.into(),
            result: Some("found 3 matching files".into()),
            is_error: false,
        };

        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![adapted_tool],
            None, None, None, false,
        ).await;

        // Adapted call is allowed (new args → new hash → no loop detected)
        assert_eq!(allowed.len(), 1);
        assert!(blocked.is_none());

        // The outcome from Request N's healing hint should have been
        // evaluated: Tool was in allowed_tools with different args → SUCCESS
        assert_eq!(
            pipeline.healing_engine.total_attempts(),
            1,
            "healing memory should have 1 recorded outcome"
        );
        assert_eq!(
            pipeline.healing_engine.successful_recoveries(),
            1,
            "the recovery should be recorded as successful"
        );
    }

    /// Verifies the self-healing outcome feedback loop for FAILURE:
    /// When the tool continues to be blocked on the next request, the
    /// outcome should be reported as failure.
    #[tokio::test]
    async fn test_healing_outcome_feedback_loop_reports_failure() {
        use dedroom_core::DedrooMConfig;

        // ── Setup: max_repeats: 2 means calls 1-2 allowed, 3+ blocked ────
        let yaml = r#"
            loop_detection:
              max_repeats: 2
            self_healing:
              enabled: true
              mode: Balanced
        "#;
        let config = DedrooMConfig::from_yaml_str(yaml).unwrap();
        let mut pipeline = Pipeline::new(config);

        // ── Calls 1-3: Same tool loops until blocked ────────────────────
        let tool = ExtractedTool { id: None,
            name: "search".into(),
            args: r#"{"query":"test"}"#.into(),
            result: Some("some result".into()),
            is_error: true,
        };

        // Call 1: allowed
        let (allowed, _blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 1);

        // Call 2: allowed
        let (allowed, _blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 1);

        // Call 3: BLOCKED with healing hint (max_repeats=2 → 3rd call blocked)
        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 0);
        assert!(blocked.is_some());

        // Memory should be empty — no outcome evaluated yet
        assert_eq!(pipeline.healing_engine.total_attempts(), 0);

        // ── Call 4: STILL blocked (agent didn't adapt) ───────────────────
        let (allowed, blocked) = process_tools_through_pipeline(
            &mut pipeline,
            vec![tool.clone()],
            None, None, None, false,
        ).await;
        assert_eq!(allowed.len(), 0);
        assert!(blocked.is_some());

        // The outcome from call 3 should have been evaluated.
        // Tool appeared in blocked_tools → outcome was reported.
        assert_eq!(
            pipeline.healing_engine.total_attempts(),
            1,
            "healing memory should have 1 recorded outcome"
        );
    }

    /// Verifies that `process_tools_through_pipeline` WITHOUT shadow mode
    /// still blocks tools normally (regression guard).
    #[tokio::test]
    async fn test_normal_mode_still_blocks() {
        use dedroom_core::DedrooMConfig;

        let yaml = r#"
            loop_detection:
              max_repeats: 2
        "#;
        let config = DedrooMConfig::from_yaml_str(yaml).unwrap();
        let mut pipeline = Pipeline::new(config);

        let tools: Vec<ExtractedTool> = (0..4)
            .map(|_| ExtractedTool { id: None,
                name: "read_file".into(),
                args: r#"{"path":"/tmp/secret.txt"}"#.into(),
                result: Some("access denied".into()),
                is_error: true,
            })
            .collect();

        // shadow_mode = false — normal blocking behavior
        let (allowed_tools, blocked_response) = process_tools_through_pipeline(
            &mut pipeline,
            tools,
            None,   // no event log
            None,   // no session
            None,   // no agent
            false,  // shadow_mode = false
        )
        .await;

        // Only the first 2 should be allowed (max_repeats=3, errors collapse threshold)
        assert_eq!(
            allowed_tools.len(),
            2,
            "normal mode should only allow first 2 calls, rest blocked"
        );

        // blocked_response should be Some with details
        assert!(
            blocked_response.is_some(),
            "normal mode should return a blocked response"
        );

        let resp = blocked_response.unwrap();
        assert_eq!(resp["error"], "tool_loop_detected");
        assert_eq!(
            resp["blocked_calls"].as_array().unwrap().len(),
            2,
            "should have 2 blocked calls"
        );
    }
}
