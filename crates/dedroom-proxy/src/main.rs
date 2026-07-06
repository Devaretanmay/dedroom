mod handlers;
mod intercept;
mod proxy;

use std::net::SocketAddr;
use std::path::PathBuf;

use dedroom_core::config::DedrooMConfig;
use tracing_subscriber::EnvFilter;

/// Compile-time check: AppState must be Send + Sync for axum handlers.
const _: fn() = || {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<proxy::AppState>();
    assert_sync::<proxy::AppState>();
};

fn parse_args() -> (u16, PathBuf, bool, Option<String>, Option<String>) {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 8080u16;
    let mut config_path = PathBuf::from("dedroom.yaml");
    let mut shadow_mode = false;
    let mut api_key = None;
    let mut upstream_url = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8080);
                }
            }
            "--config" => {
                i += 1;
                if i < args.len() {
                    config_path = PathBuf::from(&args[i]);
                }
            }
            "--shadow" => {
                shadow_mode = true;
            }
            "--api-key" => {
                i += 1;
                if i < args.len() {
                    api_key = Some(args[i].clone());
                }
            }
            "--upstream-url" => {
                i += 1;
                if i < args.len() {
                    upstream_url = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    (port, config_path, shadow_mode, api_key, upstream_url)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Parse CLI args
    let (port, config_path, shadow_mode, api_key, upstream_url) = parse_args();

    if shadow_mode {
        tracing::info!("[SHADOW] Shadow (ghost) mode enabled — will not block any calls");
    }

    tracing::info!(
        "DedrooM proxy starting — config: {}, port: {}",
        config_path.display(),
        port
    );

    // Load DedrooMConfig from YAML file
    let config = if config_path.exists() {
        DedrooMConfig::from_yaml_path(&config_path)?
    } else {
        tracing::warn!(
            "Config file not found at {}, using defaults",
            config_path.display()
        );
        DedrooMConfig::default()
    };

    // Build Pipeline and proxy state
    let state = proxy::AppState::new(config, shadow_mode, api_key, upstream_url);

    let router = proxy::ProxyRouter::new(state.clone()).build();

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Listening on {addr}");

    // Start UI rendering task for beautiful inline CLI
    let state_arc = Arc::new(state);
    start_ui_task(state_arc).await;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

use owo_colors::OwoColorize;
use tokio::time::{interval, Duration};
use std::sync::Arc;
use std::io::Write;

async fn start_ui_task(state: Arc<proxy::AppState>) {
    let mut rx = state.event_log.subscribe();
    let mut ticker = interval(Duration::from_millis(100));
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut frame_idx = 0;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Ok(event) = rx.recv() => {
                    // Clear the line where the footer is
                    print!("\r\x1B[K");
                    let formatted = format_event(&event);
                    println!("{}", formatted);
                }
                _ = ticker.tick() => {
                    let pipeline = state.default_pipeline.lock().await;
                    let report = pipeline.savings_report();
                    drop(pipeline);

                    let tokens = report.total_compression_savings + report.total_loop_savings;
                    let dollars = (tokens as f64 / 1000.0) * 0.015;
                    let time = (tokens as f64 * 20.0) / 1000.0;

                    let msg = format!(
                        "{} Mega-Architecture Savings: {} | {} | {}",
                        frames[frame_idx].cyan(),
                        format!("${:.2}", dollars).green().bold(),
                        format!("{} tok", tokens).cyan(),
                        format!("{:.1}s", time).yellow()
                    );
                    
                    frame_idx = (frame_idx + 1) % frames.len();
                    print!("\r{}", msg);
                    let _ = std::io::stdout().flush();
                }
            }
        }
    });
}

fn format_event(event: &dedroom_core::telemetry::ProxyEvent) -> String {
    let latency_ms = event.latency_us as f64 / 1000.0;
    
    if event.verdict == "block" {
        let block_line = format!("{} BLOCK ({})", "🟥", event.tool_name.bold());
        let tokens_saved = event.original_tokens.unwrap_or(0);
        let time_saved = (tokens_saved as f64 * 20.0) / 1000.0;
        let dollars_saved = (tokens_saved as f64 / 1000.0) * 0.015;
        let detail_line = format!(
            "└─ Loop Detected! Saved: {} | {}",
            format!("{:.1}s", time_saved).yellow(),
            format!("${:.4}", dollars_saved).green()
        );
        format!("{}\n{}", block_line, detail_line)
    } else if event.verdict == "allow" {
        let allow_line = format!("{} ALLOW ({})", "🟩", event.tool_name.bold());
        
        let detail_line = if let (Some(orig), Some(comp)) = (event.original_tokens, event.compressed_tokens) {
            if orig > comp {
                let saved_dollars = ((orig - comp) as f64 / 1000.0) * 0.015;
                format!(
                    "└─ Latency: {:.1}ms | Original: {} → Compressed: {} tok (Saved {})",
                    latency_ms, orig, comp, format!("${:.4}", saved_dollars).green()
                )
            } else {
                format!(
                    "└─ Latency: {:.1}ms | Tokens: {}",
                    latency_ms, orig
                )
            }
        } else {
            format!("└─ Latency: {:.1}ms", latency_ms)
        };
        
        format!("{}\n{}", allow_line, detail_line)
    } else {
        let line = format!("{} INJECT ({})", "🟪", event.tool_name.bold());
        let detail_line = format!("└─ Latency: {:.1}ms | Context injected", latency_ms);
        format!("{}\n{}", line, detail_line)
    }
}
