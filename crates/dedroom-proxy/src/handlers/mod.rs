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
    JsonExtractor(body): JsonExtractor<Value>,
) -> impl IntoResponse {
    let session_id = intercept::get_session_id(&headers);
    let agent_id = intercept::get_agent_id(&headers);

    // Extract tools synchronously before holding the pipeline lock
    let tools = intercept::extract_tool_calls_openai(&body);

    let shadow_mode = state.proxy_config.shadow_mode;

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

    match intercept::forward_to_upstream(
        &headers,
        body,
        intercept::Provider::OpenAI,
        &state.proxy_config,
    )
    .await
    {
        Ok(upstream_resp) => {
            // Re-acquire lock to record telemetry
            let mut pipeline_guard = pipeline.lock().await;
            let modified =
                intercept::record_upstream_response(&mut pipeline_guard, &upstream_resp, &[]).await;
            drop(pipeline_guard);

            if state.proxy_config.force_sse {
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
    JsonExtractor(body): JsonExtractor<Value>,
) -> impl IntoResponse {
    let session_id = intercept::get_session_id(&headers);
    let agent_id = intercept::get_agent_id(&headers);
    let shadow_mode = state.proxy_config.shadow_mode;
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

    match intercept::forward_to_upstream(
        &headers,
        body,
        intercept::Provider::Anthropic,
        &state.proxy_config,
    )
    .await
    {
        Ok(upstream_resp) => {
            let mut pipeline_guard = pipeline.lock().await;
            let modified =
                intercept::record_upstream_response(&mut pipeline_guard, &upstream_resp, &[]).await;
            drop(pipeline_guard);

            if state.proxy_config.force_sse {
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

    Json(json!({
        "status": "ok",
        "service": "dedroom-proxy",
        "pipeline": {
            "total_calls_tracked": summary.total_calls,
            "current_loop_state": format!("{loop_state:?}"),
            "tool_count": summary.tool_counts.len(),
            "max_repeats": summary.current_max_repeats,
        },
    }))
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
    drop(pipeline);

    Json(json!({
        "savings": {
            "total_compression_savings_tokens": report.total_compression_savings,
            "total_loop_savings_tokens": report.total_loop_savings,
            "total_calls_blocked": report.total_calls_blocked,
            "total_original_tokens": report.total_original_tokens,
            "total_compressed_tokens": report.total_compressed_tokens,
            "blocked_by_tool": report
                .loop_block_by_tool
                .iter()
                .map(|(name, count)| json!({ "tool": name, "count": count }))
                .collect::<Vec<_>>(),
        },
        "loop_state": {
            "total_calls": summary.total_calls,
            "tool_counts": summary.tool_counts,
        },
        "config": {
            "max_repeats": state.config.loop_detection.max_repeats,
            "session_count": state.sessions.lock().await.len(),
        },
    }))
}

/// GET /admin/events — return recent events from the in-memory ring buffer.
///
/// Returns the last 100 events as a JSON array, plus metadata.
/// Zero I/O — reads from the EventLog's ring buffer populated during `record()`.
pub async fn events(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let max_events = 100usize;
    let recent = state.event_log.recent_events(max_events);
    let total_events = state.event_log.event_count();

    let events: Vec<Value> = recent
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();

    Json(json!({
        "events": events,
        "total_events": total_events,
        "returned": events.len(),
        "ring_capacity": 1000,
        "file_path": state.event_log.path().to_string_lossy(),
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

/// POST /admin/runtime-env — update configuration at runtime.
pub async fn runtime_env(
    Extension(state): Extension<Arc<AppState>>,
    JsonExtractor(update): JsonExtractor<RuntimeEnvUpdate>,
) -> impl IntoResponse {
    let mut state_clone = (*state).clone();

    if let Some(ref url) = update.openai_base_url {
        state_clone.proxy_config.openai_base_url = url.clone();
    }
    if let Some(ref url) = update.anthropic_base_url {
        state_clone.proxy_config.anthropic_base_url = url.clone();
    }
    if let Some(ref key) = update.api_key {
        state_clone.proxy_config.api_key = Some(key.clone());
    }
    if let Some(force_sse) = update.force_sse {
        state_clone.proxy_config.force_sse = force_sse;
    }
    if let Some(max_repeats) = update.max_repeats {
        let mut cfg = state_clone.config.clone();
        cfg.loop_detection.max_repeats = max_repeats;
        state_clone.update_config(cfg);
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
