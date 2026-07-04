//! Core pipeline interception logic.
//!
//! Orchestrates the DedrooM pipeline on every proxied request:
//! 1. Parse the request body to extract tool calls
//! 2. Run each tool call through `Pipeline::process_tool_call`
//! 3. Block or allow based on loop verdict
//! 4. Forward allowed calls to upstream with compressed context
//! 5. Record telemetry and return the modified response


use axum::http::HeaderMap;
use dedroom_core::pipeline::{Pipeline, ToolCall};
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
                        name,
                        args,
                        result: None,
                        is_error: false,
                    });
                }
            }

            // Tool result messages contain the actual output
            if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
                let name = msg
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let content = msg
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let is_error = msg
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                // Match by position: pair with the most recent unmatched tool
                if let Some(tool) = tools.iter_mut().rev().find(|t| t.name == name && t.result.is_none()) {
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
            if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                for block in content {
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
                            tools.push(ExtractedTool {
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
/// Returns `(allowed_tools, blocked_response)` where `blocked_response` is
/// `Some` when at least one tool call was blocked.
pub async fn process_tools_through_pipeline(
    pipeline: &mut Pipeline,
    tools: Vec<ExtractedTool>,
) -> (Vec<ToolCall>, Option<Value>) {
    let mut allowed_tools = Vec::new();
    let mut blocked_tools = Vec::new();

    for tool in tools {
        let tc = ToolCall {
            name: tool.name.clone(),
            args: tool.args,
            result: tool.result.clone(),
            is_error: tool.is_error,
        };

        let result = pipeline.process_tool_call(&tc).await;

        if result.loop_verdict.is_blocked() {
            blocked_tools.push((tc, result));
        } else {
            allowed_tools.push(tc);
        }
    }

    let blocked_response = if blocked_tools.is_empty() {
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

    let url = format!("{}{}", base_url.trim_end_matches('/'), path);

    // Force non-streaming — always set stream=false
    let mut upstream_body = body.clone();
    if let Some(obj) = upstream_body.as_object_mut() {
        obj.insert("stream".to_string(), json!(false));
    }

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&upstream_body);

    // Forward authorization header if present
    if let Some(auth) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        req = req.header("authorization", auth);
    } else if let Some(ref key) = proxy_config.api_key {
        let auth_header = match provider {
            Provider::OpenAI => format!("Bearer {}", key),
            Provider::Anthropic => format!("Bearer {}", key),
        };
        req = req.header("authorization", &auth_header);
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
            let _ = pipeline.process_tool_call(tool).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use dedroom_core::loop_detection::LoopVerdict;
    use serde_json::json;

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
        let mut pipeline = Pipeline::new(config);

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
}
