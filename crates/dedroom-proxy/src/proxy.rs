//! Proxy state, configuration, and router construction.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use dedroom_core::config::DedrooMConfig;
use dedroom_core::pipeline::Pipeline;
use tokio::sync::{Mutex, RwLock};

use dedroom_core::telemetry::EventLog;

use crate::handlers;

/// Upstream provider configuration.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Base URL for the OpenAI-compatible API.
    pub openai_base_url: String,
    /// Base URL for the Anthropic-compatible API.
    pub anthropic_base_url: String,
    /// Optional API key forwarded to upstream.
    pub api_key: Option<String>,
    /// Whether to force non-streaming upstream and re-wrap as SSE.
    pub force_sse: bool,
    /// Shadow mode: run the full pipeline but never block or modify
    /// requests. Logs what would have happened to the NDJSON event log.
    pub shadow_mode: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            openai_base_url: "https://api.openai.com".to_string(),
            anthropic_base_url: "https://api.anthropic.com".to_string(),
            api_key: None,
            force_sse: true,
            shadow_mode: false,
        }
    }
}

/// Shared application state.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Base DedrooM configuration used to create new pipelines.
    pub config: Arc<RwLock<DedrooMConfig>>,
    /// Proxy-level configuration (upstream URLs, etc.).
    pub proxy_config: Arc<RwLock<ProxyConfig>>,
    /// Per-session pipeline instances, keyed by x-session-id.
    pub sessions: Arc<Mutex<HashMap<String, Arc<Mutex<Pipeline>>>>>,
    /// Default pipeline for requests without a session header.
    pub default_pipeline: Arc<Mutex<Pipeline>>,
    /// Background NDJSON event logger.
    pub event_log: EventLog,
    /// Monotonic timestamp when the proxy started (std::time::Instant).
    pub startup_instant: std::time::Instant,
    /// Optional admin token. When set, all `/admin/*` endpoints require a
    /// matching `X-Dedroom-Token` (or `Authorization: Bearer`) header.
    /// When unset (default, localhost dev), the admin API is open.
    pub admin_token: Option<String>,
}

impl AppState {
    pub fn new(config: DedrooMConfig, shadow_mode: bool, api_key: Option<String>, upstream_url: Option<String>) -> Self {
        let event_log = EventLog::start();
        let mut proxy_config = ProxyConfig {
            shadow_mode,
            api_key,
            ..Default::default()
        };

        if let Some(url) = upstream_url {
            proxy_config.openai_base_url = url.clone();
            proxy_config.anthropic_base_url = url;
        }

        Self {
            default_pipeline: Arc::new(Mutex::new(Pipeline::new(config.clone()))),
            config: Arc::new(RwLock::new(config)),
            proxy_config: Arc::new(RwLock::new(proxy_config)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            event_log,
            startup_instant: std::time::Instant::now(),
            admin_token: std::env::var("DEDROOM_ADMIN_TOKEN").ok().filter(|s| !s.is_empty()),
        }
    }

    /// Get or create a pipeline for the given session ID.
    ///
    /// When `session_id` is `None`, returns the default shared pipeline.
    pub async fn get_or_create_pipeline(&self, session_id: Option<&str>) -> Arc<Mutex<Pipeline>> {
        match session_id {
            Some(id) => {
                let config_clone = self.config.read().await.clone();
                let mut sessions = self.sessions.lock().await;
                sessions
                    .entry(id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(Pipeline::new(config_clone))))
                    .clone()
            }
            None => self.default_pipeline.clone(),
        }
    }

    /// Update the base configuration and rebuild the default pipeline.
    pub async fn update_config(&self, new_config: DedrooMConfig) {
        let mut config_write = self.config.write().await;
        *config_write = new_config.clone();
        
        let mut default_pipeline = self.default_pipeline.lock().await;
        *default_pipeline = Pipeline::new(new_config.clone());
        // Existing sessions keep their old config; only the default is replaced.
    }
}

/// Builds the axum router with all routes and shared state.
#[derive(Debug)]
pub struct ProxyRouter {
    state: Arc<AppState>,
}

impl ProxyRouter {
    pub fn new(state: AppState) -> Self {
        Self {
            state: Arc::new(state),
        }
    }

    /// Construct the route table.
    ///
    /// Routes:
    /// - `POST /v1/chat/completions` — OpenAI-compatible chat endpoint
    /// - `POST /v1/messages` — Anthropic-compatible messages endpoint
    /// - `GET /health` — health check with pipeline state summary
    /// - `GET /admin/stats` — pipeline savings and telemetry report
    /// - `POST /admin/runtime-env` — live config update
    pub fn build(&self) -> Router {
        let admin_routes = Router::new()
            .route("/admin/stats", get(handlers::stats))
            .route("/admin/events", get(handlers::events))
            .route("/admin/events/stream", get(handlers::events_stream))
            .route("/admin/runtime-env", post(handlers::runtime_env))
            .route("/admin/attribution", get(handlers::attribution))
            .route("/admin/learning", get(handlers::learning))
            .route("/admin/instincts", get(handlers::instincts))
            .layer(middleware::from_fn(admin_guard));

        Router::new()
            .route(
                "/v1/chat/completions",
                post(handlers::chat_completions),
            )
            .route("/v1/messages", post(handlers::messages))
            .route("/v1/models", get(handlers::models))
            .route("/health", get(handlers::health))
            .merge(admin_routes)
            .layer(Extension(self.state.clone()))
    }
}

/// Middleware guarding `/admin/*` endpoints.
///
/// If the proxy was started with `DEDROOM_ADMIN_TOKEN` set, every admin
/// request must carry that token via `X-Dedroom-Token` or
/// `Authorization: Bearer <token>`. When no token is configured (the
/// default for local, localhost-bound operation) the API stays open.
async fn admin_guard(req: Request, next: Next) -> Response {
    let token = req
        .extensions()
        .get::<Arc<AppState>>()
        .and_then(|s| s.admin_token.clone());

    if let Some(expected) = token {
        let provided = req
            .headers()
            .get("x-dedroom-token")
            .and_then(|v| v.to_str().ok())
            .or_else(|| req.headers().get("authorization").and_then(|v| v.to_str().ok()))
            .map(|s| s.trim_start_matches("Bearer ").to_string());
        match provided {
            Some(p) if p == expected => next.run(req).await,
            _ => (
                axum::http::StatusCode::UNAUTHORIZED,
                "admin authentication required",
            )
                .into_response(),
        }
    } else {
        next.run(req).await
    }
}
