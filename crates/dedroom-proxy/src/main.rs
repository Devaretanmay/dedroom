mod connect;
mod handlers;
mod intercept;
mod proxy;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dedroom_core::config::DedrooMConfig;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;



fn parse_args() -> (u16, u16, PathBuf, bool, Option<String>, Option<String>, bool) {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 8080u16;
    let mut connect_port = 8081u16;
    let mut config_path = PathBuf::from("dedroom.yaml");
    let mut shadow_mode = false;
    let mut api_key = None;
    let mut upstream_url = None;
    let mut supervised = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8080);
                }
            }
            "--connect-port" => {
                i += 1;
                if i < args.len() {
                    connect_port = args[i].parse().unwrap_or(8081);
                }
            }
            "--no-connect" => {
                connect_port = 0;
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
            "--supervised" => {
                supervised = true;
            }
            _ => {}
        }
        i += 1;
    }

    (port, connect_port, config_path, shadow_mode, api_key, upstream_url, supervised)
}

// ── Supervisor mode (auto-restart daemon) ───────────────────────────────────
//
// When started with --supervised, the proxy binary acts as a process supervisor.
// It acquires the PID lock for the port, spawns a child copy of itself (without
// --supervised) to run the actual proxy, and monitors+restarts it on crash.
// On SIGTERM/SIGINT, it kills the child and exits cleanly (PidLock Drop cleans
// up the lock file).

use std::process::{Command, Stdio};
use std::time::Duration;

/// PID lock path for a given port. Must match CLI's convention.
fn pid_lock_path(port: u16) -> String {
    format!("/tmp/dedroom_{}.lock", port)
}

/// Atomic PID file lock using O_CREAT | O_EXCL semantics.
struct PidLock {
    path: String,
}

impl Drop for PidLock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

fn acquire_pid_lock(port: u16) -> Result<PidLock, String> {
    let lock_path = pid_lock_path(port);
    loop {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(file) => {
                use std::io::Write;
                writeln!(&file, "{}", std::process::id())
                    .map_err(|e| format!("Failed to write PID: {e}"))?;
                file.sync_all().ok();
                return Ok(PidLock { path: lock_path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Ok(content) = std::fs::read_to_string(&lock_path)
                    && let Ok(pid) = content.trim().parse::<u32>() {
                        let alive = Command::new("kill")
                            .arg("-0")
                            .arg(pid.to_string())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .ok()
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if alive {
                            return Err(format!(
                                "Another proxy is already running on port {} (PID {})",
                                port, pid
                            ));
                        }
                    }
                std::fs::remove_file(&lock_path).ok();
                continue;
            }
            Err(e) => return Err(format!("Failed to acquire lock: {e}")),
        }
    }
}

/// Open the log file in append mode for child process output.
fn open_log() -> Stdio {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/dedroom.log")
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

/// Run as supervisor: lock, spawn child, monitor, restart on crash.
///
/// Handles SIGINT (Ctrl+C) gracefully: forwards the signal to the child
/// proxy so it can shut down cleanly, then waits for it to exit before
/// dropping the PidLock and exiting itself.
fn run_supervised(
    port: u16,
    connect_port: u16,
    config_path: &Path,
    shadow_mode: bool,
    api_key: Option<&str>,
    upstream_url: Option<&str>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    let _lock = acquire_pid_lock(port)?;
    let exe = std::env::current_exe().map_err(|e| format!("Cannot get exe path: {e}"))?;

    // Signal coordination: running flag + current child PID
    // Use Arc so the handler closure and main thread can share them
    let running = Arc::new(AtomicBool::new(true));
    let child_pid = Arc::new(AtomicU32::new(0));

    // Register Ctrl+C handler — forwards SIGINT to child proxy for graceful shutdown
    let running_h = running.clone();
    let child_pid_h = child_pid.clone();
    ctrlc::set_handler(move || {
        running_h.store(false, Ordering::SeqCst);
        let pid = child_pid_h.load(Ordering::SeqCst);
        if pid > 0 {
            // Send SIGINT to child so tokio::signal::ctrl_c() can shut it down gracefully
            #[cfg(unix)]
            let _ = Command::new("kill")
                .arg("-INT")
                .arg(pid.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            #[cfg(not(unix))]
            let _ = Command::new("taskkill")
                .args(&["/PID", &pid.to_string(), "/F"])
                .spawn();
        }
    }).map_err(|e| format!("Failed to set Ctrl+C handler: {e}"))?;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Build child args (same as current, without --supervised)
        let mut args: Vec<String> = vec![
            "--port".to_string(),
            port.to_string(),
            "--config".to_string(),
            config_path.to_string_lossy().to_string(),
        ];
        if connect_port > 0 {
            args.push("--connect-port".to_string());
            args.push(connect_port.to_string());
        }
        if shadow_mode {
            args.push("--shadow".to_string());
        }
        if let Some(key) = api_key {
            args.push("--api-key".to_string());
            args.push(key.to_string());
        }
        if let Some(url) = upstream_url {
            args.push("--upstream-url".to_string());
            args.push(url.to_string());
        }

        let mut child = match Command::new(&exe)
            .args(&args)
            .stdout(open_log())
            .stderr(open_log())
            .stdin(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[supervisor] Failed to start proxy: {e}. Retrying in 5s...");
                let mut waited = 0u64;
                while waited < 5 && running.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_secs(1));
                    waited += 1;
                }
                continue;
            }
        };

        child_pid.store(child.id(), Ordering::SeqCst);
        eprintln!("[supervisor] Proxy started (PID {})", child.id());

        // Poll for child exit instead of blocking, so we can react to signals
        loop {
            if !running.load(Ordering::SeqCst) {
                // Signal received — forward SIGINT to child for graceful shutdown
                #[cfg(unix)]
                let _ = Command::new("kill")
                    .arg("-INT")
                    .arg(child.id().to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                #[cfg(not(unix))]
                let _ = Command::new("taskkill")
                    .args(&["/PID", &child.id().to_string(), "/F"])
                    .spawn();
                // Give child time to shut down gracefully
                for _ in 0..50 {
                    if let Ok(Some(_)) = child.try_wait() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                // Force kill if still alive
                child.kill().ok();
                child.wait().ok();
                child_pid.store(0, Ordering::SeqCst);
                break;
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    child_pid.store(0, Ordering::SeqCst);
                    let exited_cleanly = status.success();
                    if exited_cleanly {
                        eprintln!("[supervisor] Proxy exited cleanly. Stopping.");
                        return Ok(());
                    }
                    eprintln!(
                        "[supervisor] Proxy (PID {}) crashed. Restarting in 1s...",
                        child.id()
                    );
                    std::thread::sleep(Duration::from_secs(1));
                    break; // back to outer loop to restart
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    child_pid.store(0, Ordering::SeqCst);
                    eprintln!("[supervisor] Error waiting for proxy: {e}. Restarting in 5s...");
                    std::thread::sleep(Duration::from_secs(5));
                    break;
                }
            }
        }
    }

    // PidLock dropped here → lock file cleaned up
    Ok(())
}

// ── Normal entry point (non-supervised) ─────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let (port, connect_port, config_path, shadow_mode, api_key, upstream_url, supervised) =
        parse_args();

    if supervised {
        // Run as supervisor — synchronous, no tokio needed
        run_supervised(
            port,
            connect_port,
            &config_path,
            shadow_mode,
            api_key.as_deref(),
            upstream_url.as_deref(),
        )
        .map_err(|e| anyhow::anyhow!("Supervisor error: {e}"))?;
        return Ok(());
    }

    // Normal async main via tokio
    tokio::runtime::Runtime::new()?.block_on(async_main(
        port,
        connect_port,
        config_path,
        shadow_mode,
        api_key,
        upstream_url,
    ))
}

async fn async_main(
    port: u16,
    connect_port: u16,
    config_path: PathBuf,
    shadow_mode: bool,
    api_key: Option<String>,
    upstream_url: Option<String>,
) -> anyhow::Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

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
    let state_arc = Arc::new(state);

    // Start UI rendering task
    start_ui_task(state_arc.clone()).await;

    // ── Axum HTTP server ──
    let http_addr = SocketAddr::from(([0, 0, 0, 0], port));
    let http_listener = TcpListener::bind(http_addr).await?;
    tracing::info!("HTTP API server listening on {http_addr}");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(http_listener, router).await {
            tracing::error!("HTTP server error: {e}");
        }
    });

    // ── CONNECT tunnel server ──
    if connect_port > 0 {
        let connect_addr = SocketAddr::from(([0, 0, 0, 0], connect_port));
        let connect_listener = TcpListener::bind(connect_addr).await?;
        tracing::info!(
            "CONNECT proxy listening on {connect_addr} — set HTTPS_PROXY=http://127.0.0.1:{connect_port} to route any agent through DedrooM"
        );

        tokio::spawn(async move {
            loop {
                match connect_listener.accept().await {
                    Ok((stream, _peer)) => {
                        tokio::spawn(async move {
                            crate::connect::handle_tunnel(stream).await;
                        });
                    }
                    Err(e) => {
                        tracing::error!("CONNECT listener accept error: {e}");
                        break;
                    }
                }
            }
        });
    } else {
        tracing::info!("CONNECT proxy disabled (--no-connect)");
    }

    // Keep main task alive until Ctrl+C
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Shutting down...");
    Ok(())
}

use owo_colors::OwoColorize;
use tokio::time::{interval, Duration as TokioDuration};
use std::io::Write;

async fn start_ui_task(state: Arc<proxy::AppState>) {
    let mut rx = state.event_log.subscribe();
    let mut ticker = interval(TokioDuration::from_millis(100));
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut frame_idx = 0;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Ok(event) = rx.recv() => {
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
        format!("{}\\{}", block_line, detail_line)
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
