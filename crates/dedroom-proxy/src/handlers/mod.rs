//! Route handler functions for the DedrooM proxy.

use std::sync::Arc;

use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Sse};
use axum::Json as JsonExtractor;
use futures::stream;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::intercept;
use crate::proxy::AppState;

/// POST /v1/chat/completions — OpenAI-compatible chat endpoint.
pub async fn chat_completions(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    JsonExtractor(mut body): JsonExtractor<Value>,
) -> impl IntoResponse {
    let session_id = intercept::get_session_id(&headers);
    let agent_id = intercept::get_agent_id(&headers);

    // Extract tools synchronously before holding the pipeline lock
    let tools = intercept::extract_tool_calls_openai(&body);

    let shadow_mode = state.proxy_config.read().await.shadow_mode;

    // Lock the pipeline and process tools
    let pipeline = state.get_or_create_pipeline(session_id.as_deref()).await;
    let mut pipeline_guard = pipeline.lock().await;

    let (_allowed, blocked) = intercept::process_tools_through_pipeline(
        &mut pipeline_guard,
        tools,
        Some(&state.event_log),
        session_id.as_deref(),
        agent_id.as_deref(),
        shadow_mode,
    )
    .await;
    drop(pipeline_guard);

    if !shadow_mode && let Some(blocked_resp) = blocked {
        return (StatusCode::TOO_MANY_REQUESTS, Json(blocked_resp)).into_response();
    }

    let proxy_cfg = state.proxy_config.read().await.clone();

    // Apply redaction + compression to tool-result payloads before they are
    // forwarded upstream, so secrets and redundant context stay local.
    {
        let guard = pipeline.lock().await;
        intercept::transform_request_body(&mut body, &guard);
    }

    match intercept::forward_to_upstream(
        &headers,
        body,
        intercept::Provider::OpenAI,
        &proxy_cfg,
    )
    .await
    {
        Ok(upstream_resp) => {
            // Re-acquire lock to record telemetry
            let mut pipeline_guard = pipeline.lock().await;
            let modified =
                intercept::record_upstream_response(&mut pipeline_guard, &upstream_resp, &[]).await;
            drop(pipeline_guard);

            if proxy_cfg.force_sse {
                make_sse_response(StatusCode::OK, modified)
            } else {
                (StatusCode::OK, Json(modified)).into_response()
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// POST /v1/messages — Anthropic-compatible messages endpoint.
pub async fn messages(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    JsonExtractor(mut body): JsonExtractor<Value>,
) -> impl IntoResponse {
    let session_id = intercept::get_session_id(&headers);
    let agent_id = intercept::get_agent_id(&headers);
    let shadow_mode = state.proxy_config.read().await.shadow_mode;
    let tools = intercept::extract_tool_calls_anthropic(&body);

    let pipeline = state.get_or_create_pipeline(session_id.as_deref()).await;
    let mut pipeline_guard = pipeline.lock().await;
    let (_allowed, blocked) = intercept::process_tools_through_pipeline(
        &mut pipeline_guard,
        tools,
        Some(&state.event_log),
        session_id.as_deref(),
        agent_id.as_deref(),
        shadow_mode,
    )
    .await;
    drop(pipeline_guard);

    if !shadow_mode && let Some(blocked_resp) = blocked {
        return (StatusCode::TOO_MANY_REQUESTS, Json(blocked_resp)).into_response();
    }

    let proxy_cfg = state.proxy_config.read().await.clone();

    // Apply redaction + compression to tool-result payloads before they are
    // forwarded upstream, so secrets and redundant context stay local.
    {
        let guard = pipeline.lock().await;
        intercept::transform_request_body(&mut body, &guard);
    }

    match intercept::forward_to_upstream(
        &headers,
        body,
        intercept::Provider::Anthropic,
        &proxy_cfg,
    )
    .await
    {
        Ok(upstream_resp) => {
            let mut pipeline_guard = pipeline.lock().await;
            let modified =
                intercept::record_upstream_response(&mut pipeline_guard, &upstream_resp, &[]).await;
            drop(pipeline_guard);

            if proxy_cfg.force_sse {
                make_sse_response(StatusCode::OK, modified)
            } else {
                (StatusCode::OK, Json(modified)).into_response()
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// GET /health — health check with pipeline state summary.
pub async fn health(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let pipeline = state.default_pipeline.lock().await;
    let summary = pipeline.loop_state_summary();
    let loop_state = pipeline.current_loop_state();
    drop(pipeline);

    let uptime_secs = state.startup_instant.elapsed().as_secs();

    Json(json!({
        "status": "ok",
        "service": "dedroom-proxy",
        "uptime_seconds": uptime_secs,
        "pipeline": {
            "total_calls_tracked": summary.total_calls,
            "current_loop_state": format!("{loop_state:?}"),
            "tool_count": summary.tool_counts.len(),
            "max_repeats": summary.current_max_repeats,
        },
    }))
}

/// GET /v1/models — dynamic model discovery proxy.
///
/// Forwards the model list request to the upstream `openai_base_url`.
/// Uses the proxy's configured API key if present, otherwise passes through
/// the `Authorization` header from the client (e.g. OpenCode UI).
pub async fn models(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let proxy_cfg = state.proxy_config.read().await.clone();
    
    let base_clean = proxy_cfg.openai_base_url.trim_end_matches('/');
    let base_clean = if base_clean.ends_with("/v1") {
        base_clean.strip_suffix("/v1").unwrap()
    } else {
        base_clean
    };
    
    let url = format!("{}/v1/models", base_clean);

    let client = reqwest::Client::new();
    let mut req = client.get(&url);

    // Use proxy's API key if available, otherwise forward client's header
    if let Some(ref key) = proxy_cfg.api_key {
        req = req.header("authorization", format!("Bearer {}", key));
    } else if let Some(auth) = headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
    {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(body) => (status, Json(body)).into_response(),
                Err(e) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": format!("Failed to parse JSON from upstream: {}", e) })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Failed to fetch models from upstream: {}", e) })),
        )
            .into_response(),
    }
}

/// GET /admin/attribution — token attribution, waste breakdown, and ROI tracking.
///
/// Returns a detailed report with per-tool breakdown, waste categorization,
/// and cost estimates. Shows what happened to every token that passed through
/// the pipeline.
pub async fn attribution(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let pipeline = state.default_pipeline.lock().await;
    let report = pipeline.attribution_report();
    drop(pipeline);

    Json(json!(report))
}

/// GET /admin/stats — savings ledger report and pipeline telemetry.
pub async fn stats(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let pipeline = state.default_pipeline.lock().await;
    let report = pipeline.savings_report();
    let summary = pipeline.loop_state_summary();
    let att = pipeline.attribution_report();
    drop(pipeline);

    let cfg = state.config.read().await.clone();
    let sessions_len = state.sessions.lock().await.len();

    Json(json!({
        "savings": {
            "total_compression_savings_tokens": report.total_compression_savings,
            "total_loop_savings_tokens": report.total_loop_savings,
            "total_calls_blocked": report.total_calls_blocked,
            "total_original_tokens": report.total_original_tokens,
            "total_compressed_tokens": report.total_compressed_tokens,
            "blocked_by_tool": att.per_tool,
        },
        "loop_state": {
            "total_calls": summary.total_calls,
            "tool_counts": summary.tool_counts,
        },
        "config": {
            "max_repeats": cfg.loop_detection.max_repeats,
            "session_count": sessions_len,
        },
    }))
}

/// GET /admin/events — return recent events from the NDJSON file.
///
/// Reads the last 100 lines from the event log file. Falls back to
/// empty if the file is not yet written or unavailable.
pub async fn events(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let total_events = state.event_log.event_count();
    let path = state.event_log.path().to_path_buf();
    let events_path = path.clone();
    let events: Vec<Value> = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&events_path).ok()?;
        let reader = std::io::BufReader::new(file);
        let lines: Vec<String> = std::io::BufRead::lines(reader)
            .map_while(Result::ok)
            .collect();
        let tail: Vec<Value> = lines.iter().rev().take(100)
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        Some(tail)
    }).await.unwrap_or(None).unwrap_or_default();

    Json(json!({
        "events": events,
        "total_events": total_events,
        "returned": events.len(),
        "file_path": path.to_string_lossy(),
    }))
    .into_response()
}

/// GET /admin/events/stream — SSE stream of live events.
///
/// Subscribes to the EventLog broadcast channel and pushes each event
/// as an SSE `data:` frame as it is recorded. The connection stays open
/// until the client disconnects.
pub async fn events_stream(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let rx = state.event_log.subscribe();

    // Use unfold to convert the broadcast receiver into a stream without
    // requiring tokio-stream's sync feature.
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json_line = match serde_json::to_string(&event) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    return Some((
                        Ok::<_, std::convert::Infallible>(
                            axum::response::sse::Event::default().data(json_line),
                        ),
                        rx,
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[events/stream] client lagged by {n} events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("[events/stream] channel closed, ending stream");
                    return None;
                }
            }
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// GET /admin/learning — healing memory stats.
pub async fn learning(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let pipeline = state.default_pipeline.lock().await;
    let stats = pipeline.healing_engine.memory.stats();
    let total_attempts = pipeline.healing_engine.total_attempts();
    let total_successes = pipeline.healing_engine.successful_recoveries();
    drop(pipeline);

    Json(json!({
        "stats": {
            "total_attempts": total_attempts,
            "total_successes": total_successes,
            "success_rate": if total_attempts > 0 { total_successes as f64 / total_attempts as f64 } else { 0.0 },
            "by_tool": stats,
        },
        "note": "Learning memory uses in-memory HealingMemory with args_hash matching.",
    }))
}

/// GET /admin/instincts — active instinct rules (from config).
pub async fn instincts(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let pipeline = state.default_pipeline.lock().await;
    let ie = pipeline.healing_engine.instincts_engine();
    let rules = ie.list_instincts();
    let count = ie.instinct_count();
    drop(pipeline);

    Json(json!({
        "rules": rules,
        "total": count,
        "note": "Instincts are loaded from config and are in-memory only.",
    }))
}

/// POST /admin/runtime-env — update configuration at runtime.
pub async fn runtime_env(
    Extension(state): Extension<Arc<AppState>>,
    JsonExtractor(update): JsonExtractor<RuntimeEnvUpdate>,
) -> impl IntoResponse {
    let mut p_config = state.proxy_config.write().await;
    
    if let Some(ref url) = update.openai_base_url {
        p_config.openai_base_url = url.clone();
    }
    if let Some(ref url) = update.anthropic_base_url {
        p_config.anthropic_base_url = url.clone();
    }
    if let Some(ref key) = update.api_key {
        p_config.api_key = Some(key.clone());
    }
    if let Some(force_sse) = update.force_sse {
        p_config.force_sse = force_sse;
    }
    drop(p_config); // drop lock before updating config

    if let Some(max_repeats) = update.max_repeats {
        let mut cfg = state.config.read().await.clone();
        cfg.loop_detection.max_repeats = max_repeats;
        state.update_config(cfg).await;
    }

    tracing::info!("Runtime config update applied: {update:?}");

    Json(json!({
        "status": "ok",
        "message": "Configuration updated. Changes apply to new requests.",
    }))
}

/// Update payload for the runtime-env endpoint.
#[derive(Debug, Deserialize)]
pub struct RuntimeEnvUpdate {
    pub max_repeats: Option<u32>,
    pub openai_base_url: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub api_key: Option<String>,
    pub force_sse: Option<bool>,
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build an SSE response wrapping a JSON body.
fn make_sse_response(status: StatusCode, body: Value) -> axum::response::Response {
    let sse_body = crate::intercept::wrap_as_sse(body);
    let stream = stream::once(async move {
        Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(sse_body))
    });
    let sse = Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new());
    (status, sse).into_response()
}
