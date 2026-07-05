//! DedrooM CLI — wrap AI agents with loop detection and context compression.
//!
//! Usage:
//!     dedroom wrap claude                    # Start proxy + launch Claude Code
//!     dedroom wrap claude --port 9999        # Custom proxy port
//!     dedroom wrap claude --config config.yaml
//!     dedroom wrap claude -- --model opus    # Pass args to claude
//!     dedroom wrap codex                     # Start proxy + launch OpenAI Codex CLI
//!     dedroom wrap codex --port 9999
//!     dedroom wrap aider                     # Start proxy + launch aider
//!     dedroom wrap aider --port 9999
//!     dedroom wrap cursor                    # Start proxy + print Cursor config instructions
//!     dedroom wrap cursor --port 9999
//!     dedroom wrap opencode                  # Start proxy + launch OpenCode
//!     dedroom wrap opencode --port 9999
//!     dedroom wrap cline                     # Start proxy + print Cline config instructions
//!     dedroom wrap cline --port 9999
//!     dedroom unwrap codex                   # Restore Codex config from backup
//!     dedroom doctor                         # Run diagnostics
//!     dedroom doctor --port 9999
//!     dedroom doctor --json                  # JSON output
//!     dedroom proxy                          # Start standalone proxy

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::time::sleep;

// ── CLI argument parsing ───────────────────────────────────────────────────

#[derive(Debug)]
enum CliCommand {
    /// Start proxy and wrap an AI agent.
    Wrap {
        agent: String,
        port: u16,
        config: PathBuf,
        agent_args: Vec<String>,
    },
    /// Undo wrap changes for an agent (restore configs, etc.).
    Unwrap {
        agent: String,
        port: u16,
    },
    /// Run diagnostics to verify proxy and agent routing.
    Doctor {
        port: u16,
        emit_json: bool,
    },
    /// Start standalone proxy server.
    Proxy {
        port: u16,
        config: PathBuf,
    },
    /// Launch the TUI dashboard, auto-detecting proxy URL.
    Dash {
        port: u16,
        url: Option<String>,
    },
}

fn parse_args() -> Result<CliCommand> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: dedroom <command> [options] [-- <agent-args>]");
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  wrap <agent>  Start proxy + launch agent (e.g. claude, codex, aider, cursor, opencode, cline)");
        eprintln!("  unwrap <agent> Undo wrap changes (restore configs, etc.)");
        eprintln!("  doctor        Run diagnostics (proxy, routing, savings)");
        eprintln!("  proxy         Start standalone proxy server");
        eprintln!("  dash [url]    Launch TUI dashboard (auto-detect proxy)");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --port <N>    Proxy port (default: 8080)");
        eprintln!("  --config <P>  Config file path (default: dedroom.yaml)");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  dedroom wrap claude");
        eprintln!("  dedroom wrap claude --port 9999 -- --model opus");
        eprintln!("  dedroom wrap codex");
        eprintln!("  dedroom wrap aider");
        eprintln!("  dedroom wrap cursor");
        eprintln!("  dedroom wrap opencode");
        eprintln!("  dedroom wrap cline");
        eprintln!("  dedroom unwrap codex");
        eprintln!("  dedroom doctor");
        eprintln!("  dedroom doctor --port 9999 --json");
        eprintln!("  dedroom proxy --port 8080");
        eprintln!();
        eprintln!("Dash:");
        eprintln!("  dedroom dash                               # auto-detect on port 8080");
        eprintln!("  dedroom dash --port 9090                    # custom port");
        eprintln!("  dedroom dash http://10.0.0.5:9090           # custom URL");
        std::process::exit(1);
    }

    let mut i = 1;
    let command = &args[i];
    i += 1;

    match command.as_str() {
        "unwrap" => {
            if i >= args.len() || args[i].starts_with("--") {
                bail!("Usage: dedroom unwrap <agent>");
            }
            let agent = args[i].clone();
            i += 1;

            let mut port = 8080u16;

            while i < args.len() {
                match args[i].as_str() {
                    "--port" => {
                        i += 1;
                        if i < args.len() {
                            port = args[i].parse().context("--port must be a number")?;
                        }
                    }
                    _ => bail!("Unknown option: {}", args[i]),
                }
                i += 1;
            }

            Ok(CliCommand::Unwrap { agent, port })
        }
        "wrap" => {
            if i >= args.len() || args[i].starts_with("--") {
                bail!("Usage: dedroom wrap <agent> [options] [-- <agent-args>]");
            }
            let agent = args[i].clone();
            i += 1;

            let mut port = 8080u16;
            let mut config = PathBuf::from("dedroom.yaml");
            let mut agent_args: Vec<String> = Vec::new();
            let mut passed_double_dash = false;

            while i < args.len() {
                match args[i].as_str() {
                    "--port" => {
                        i += 1;
                        if i < args.len() {
                            port = args[i].parse().context("--port must be a number")?;
                        }
                    }
                    "--config" => {
                        i += 1;
                        if i < args.len() {
                            config = PathBuf::from(&args[i]);
                        }
                    }
                    "--" => {
                        passed_double_dash = true;
                    }
                    _ => {
                        if passed_double_dash {
                            agent_args.push(args[i].clone());
                        } else {
                            bail!("Unknown option: {}", args[i]);
                        }
                    }
                }
                i += 1;
            }

            Ok(CliCommand::Wrap {
                agent,
                port,
                config,
                agent_args,
            })
        }
        "doctor" => {
            let mut port = 8080u16;
            let mut emit_json = false;

            while i < args.len() {
                match args[i].as_str() {
                    "--port" => {
                        i += 1;
                        if i < args.len() {
                            port = args[i].parse().context("--port must be a number")?;
                        }
                    }
                    "--json" => {
                        emit_json = true;
                    }
                    _ => bail!("Unknown option: {}", args[i]),
                }
                i += 1;
            }

            Ok(CliCommand::Doctor { port, emit_json })
        }
        "proxy" => {
            let mut port = 8080u16;
            let mut config = PathBuf::from("dedroom.yaml");
            while i < args.len() {
                match args[i].as_str() {
                    "--port" => {
                        i += 1;
                        if i < args.len() {
                            port = args[i].parse().context("--port must be a number")?;
                        }
                    }
                    "--config" => {
                        i += 1;
                        if i < args.len() {
                            config = PathBuf::from(&args[i]);
                        }
                    }
                    _ => bail!("Unknown option: {}", args[i]),
                }
                i += 1;
            }
            Ok(CliCommand::Proxy { port, config })
        }
        "dash" => {
            let mut port = 8080u16;
            let mut url: Option<String> = None;

            while i < args.len() {
                match args[i].as_str() {
                    "--port" => {
                        i += 1;
                        if i < args.len() {
                            port = args[i].parse().context("--port must be a number")?;
                        }
                    }
                    _ => {
                        if !args[i].starts_with("--") {
                            url = Some(args[i].clone());
                        } else {
                            bail!("Unknown option: {}", args[i]);
                        }
                    }
                }
                i += 1;
            }

            Ok(CliCommand::Dash { port, url })
        }
        _ => bail!("Unknown command: {command}. Use: wrap | unwrap | proxy | dash"),
    }
}

// ── Proxy management ───────────────────────────────────────────────────────

/// Find the proxy binary path — sibling to the current executable.
fn find_proxy_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Cannot determine executable path")?;
    let exe_dir = exe.parent().context("Cannot find executable directory")?;

    // The proxy binary is a sibling: target/debug/dedroom-cli → target/debug/dedroom-proxy
    let proxy_name = if cfg!(windows) { "dedroom-proxy.exe" } else { "dedroom-proxy" };
    let candidate = exe_dir.join(proxy_name);

    if candidate.exists() {
        return Ok(candidate);
    }

    // Also check just "dedroom-proxy" in PATH
    if let Ok(path) = which::which("dedroom-proxy") {
        return Ok(path);
    }

    bail!(
        "Cannot find dedroom-proxy binary. Expected at: {}. \
         Build it first: cargo build -p dedroom-proxy",
        candidate.display()
    );
}

/// Start the proxy as a background subprocess.
fn start_proxy(port: u16, config: &Path) -> Result<std::process::Child> {
    let proxy_path = find_proxy_binary()?;
    let config_arg = if config.exists() {
        config.to_string_lossy().to_string()
    } else {
        // If the config doesn't exist, point at a fresh one so the proxy
        // starts with defaults but logs the right path
        config.to_string_lossy().to_string()
    };

    eprintln!("  Starting DedrooM proxy on port {}...", port);

    let child = Command::new(&proxy_path)
        .arg("--port")
        .arg(port.to_string())
        .arg("--config")
        .arg(&config_arg)
        .stdout(Stdio::null()) // proxy logs go to its own stdout → hidden
        .stderr(Stdio::inherit()) // proxy errors visible
        .spawn()
        .context(format!(
            "Failed to start proxy at {}",
            proxy_path.display()
        ))?;

    Ok(child)
}

/// Find the TUI dashboard binary — sibling to the current executable.
fn find_tui_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Cannot determine executable path")?;
    let exe_dir = exe.parent().context("Cannot find executable directory")?;

    let tui_name = if cfg!(windows) { "dedroom-tui.exe" } else { "dedroom-tui" };
    let candidate = exe_dir.join(tui_name);

    if candidate.exists() {
        return Ok(candidate);
    }

    // Also check just "dedroom-tui" in PATH
    if let Ok(path) = which::which("dedroom-tui") {
        return Ok(path);
    }

    bail!(
        "Cannot find dedroom-tui binary. Expected at: {}. \
         Build it first: cargo build -p dedroom-tui",
        candidate.display()
    );
}

/// Launch the TUI dashboard with auto-detected proxy URL.
fn launch_dash(port: u16, url_override: Option<String>) -> Result<std::process::Child> {
    let tui_path = find_tui_binary()?;

    let proxy_url = match url_override {
        Some(url) => url,
        None => format!("http://localhost:{}", port),
    };

    eprintln!("  Launching DedrooM dashboard...");
    eprintln!("  Connecting to proxy at {}", proxy_url);

    let child = Command::new(&tui_path)
        .arg(&proxy_url)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context(format!(
            "Failed to launch dashboard at {}",
            tui_path.display()
        ))?;

    Ok(child)
}

/// Poll the proxy /health endpoint until it responds.
async fn wait_for_proxy(port: u16, timeout_secs: u64) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/health", port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    let deadline = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    while start.elapsed() < deadline {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                eprintln!("  Proxy ready on http://127.0.0.1:{}", port);
                return Ok(());
            }
            _ => {
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    bail!(
        "Proxy did not become ready on port {} within {} seconds",
        port,
        timeout_secs
    );
}



// ── Agent binary discovery ────────────────────────────────────────────────

/// Find a binary in PATH with platform-aware name candidates.
fn find_binary(unix_names: &[&str], win_alt: Option<&str>, hint: &str) -> Result<PathBuf> {
    let mut candidates: Vec<&str> = Vec::new();
    if cfg!(windows) && let Some(alt) = win_alt {
        candidates.push(alt);
    }
    candidates.extend_from_slice(unix_names);

    for name in &candidates {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    bail!(
        "Cannot find `{}` binary in PATH. {}",
        unix_names[0],
        hint
    );
}

// ── Agent launchers ────────────────────────────────────────────────────────

/// Launch Claude Code wrapped through the DedrooM proxy.
fn launch_claude(port: u16, extra_args: &[String]) -> Result<std::process::Child> {
    // Default: check PATH for `claude`
    let claude_binary = find_claude_binary()?;

    eprintln!("  Wrapping Claude Code via 127.0.0.1:{}", port);
    eprintln!("  Launching {}...\n", claude_binary.display());

    let proxy_url = format!("http://127.0.0.1:{}", port);

    let mut cmd = Command::new(&claude_binary);
    cmd.env("ANTHROPIC_BASE_URL", &proxy_url)
        .env("ANTHROPIC_CUSTOM_HEADERS", format!("X-Project-Name: dedroom-{}", std::env::current_dir().map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default()))
        .args(extra_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = cmd.spawn().context(format!(
        "Failed to launch Claude Code from: {}.\n\
         Make sure it's installed and available in PATH.\n\
         See: https://docs.anthropic.com/en/docs/claude-code/overview",
        claude_binary.display()
    ))?;

    Ok(child)
}

fn find_claude_binary() -> Result<PathBuf> {
    find_binary(&["claude"], Some("claude.cmd"),
        "Install: npm install -g @anthropic-ai/claude-code\n  \
         or visit: https://docs.anthropic.com/en/docs/claude-code/overview")
}

// ── Codex launcher ─────────────────────────────────────────────────────────

/// Launch OpenAI Codex CLI wrapped through the DedrooM proxy.
fn launch_codex(port: u16, extra_args: &[String]) -> Result<std::process::Child> {
    let codex_binary = find_codex_binary()?;

    let proxy_url = format!("http://127.0.0.1:{}/v1", port);

    eprintln!("  Wrapping Codex CLI via 127.0.0.1:{}", port);
    eprintln!("  Setting OPENAI_BASE_URL={}", proxy_url);
    eprintln!("  Launching {}...\n", codex_binary.display());

    let mut cmd = Command::new(&codex_binary);
    cmd.env("OPENAI_BASE_URL", &proxy_url)
        .args(extra_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = cmd.spawn().context(format!(
        "Failed to launch Codex CLI from: {}.\n\
         Make sure it's installed and available in PATH.\n\
         See: https://codex.so",
        codex_binary.display()
    ))?;

    Ok(child)
}

fn find_codex_binary() -> Result<PathBuf> {
    find_binary(&["codex"], Some("codex.cmd"),
        "Install: https://codex.so")
}

// ── Aider launcher ─────────────────────────────────────────────────────────

/// Launch aider wrapped through the DedrooM proxy.
///
/// Aider supports both OpenAI and Anthropic models, so we set both base URL
/// env vars. The proxy handles OpenAI-style requests at /v1/* and
/// Anthropic-style requests at /v1/* (messages) or directly.
fn launch_aider(port: u16, extra_args: &[String]) -> Result<std::process::Child> {
    let aider_binary = find_aider_binary()?;

    let openai_url = format!("http://127.0.0.1:{}/v1", port);
    let anthropic_url = format!("http://127.0.0.1:{}", port);

    eprintln!("  Wrapping aider via 127.0.0.1:{}", port);
    eprintln!("  Setting OPENAI_API_BASE={}", openai_url);
    eprintln!("  Setting ANTHROPIC_BASE_URL={}", anthropic_url);
    eprintln!("  Launching {}...\n", aider_binary.display());

    let mut cmd = Command::new(&aider_binary);
    // Set both older (OPENAI_API_BASE) and newer (OPENAI_BASE_URL) env var
    // names for compatibility across aider versions.
    cmd.env("OPENAI_API_BASE", &openai_url)
        .env("OPENAI_BASE_URL", &openai_url)
        .env("ANTHROPIC_BASE_URL", &anthropic_url)
        .args(extra_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = cmd.spawn().context(format!(
        "Failed to launch aider from: {}.\n\
         Make sure it's installed and available in PATH.\n\
         See: https://aider.chat",
        aider_binary.display()
    ))?;

    Ok(child)
}

fn find_aider_binary() -> Result<PathBuf> {
    find_binary(&["aider"], Some("aider.cmd"),
        "Install: pip install aider-chat\n  \
         or visit: https://aider.chat/docs/install.html")
}

// ── OpenCode launcher ──────────────────────────────────────────────────────

/// Launch OpenCode through the DedrooM proxy.
///
/// OpenCode uses OPENCODE_CONFIG_CONTENT env var to configure its providers.
/// We inject a JSON payload that routes Anthropic, OpenAI, and the custom
/// headroom provider through the proxy.
fn launch_opencode(port: u16, extra_args: &[String]) -> Result<std::process::Child> {
    let opencode_binary = find_opencode_binary()?;

    let proxy_url_v1 = format!("http://127.0.0.1:{}/v1", port);
    let proxy_url = format!("http://127.0.0.1:{}", port);

    eprintln!("  Wrapping OpenCode via 127.0.0.1:{}", port);
    eprintln!("  Setting OPENCODE_CONFIG_CONTENT with DedrooM provider");
    eprintln!("  Launching {}...\n", opencode_binary.display());

    // Build the OPENCODE_CONFIG_CONTENT JSON payload (compact, no whitespace).
    let config_content = serde_json::json!({
        "provider": {
            "anthropic": {
                "options": { "baseURL": proxy_url_v1 }
            },
            "openai": {
                "options": { "baseURL": proxy_url_v1 }
            },
            "headroom": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "DedrooM Proxy",
                "options": { "baseURL": proxy_url_v1 },
                "models": {
                    "claude-sonnet-4-6": {
                        "name": "Claude Sonnet 4.6",
                        "limit": { "context": 200000, "output": 16384 }
                    },
                    "claude-opus-4-6": {
                        "name": "Claude Opus 4.6",
                        "limit": { "context": 200000, "output": 16384 }
                    },
                    "gpt-4o": {
                        "name": "GPT-4o",
                        "limit": { "context": 128000, "output": 16384 }
                    }
                }
            }
        }
    });

    let mut cmd = Command::new(&opencode_binary);
    cmd.env(
            "OPENCODE_CONFIG_CONTENT",
            serde_json::to_string(&config_content).unwrap(),
        )
        .env("ANTHROPIC_BASE_URL", &proxy_url)
        .env("OPENAI_BASE_URL", &proxy_url_v1)
        .args(extra_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let child = cmd.spawn().context(format!(
        "Failed to launch OpenCode from: {}.\n\
         Make sure it's installed and available in PATH.\n\
         See: https://opencode.ai",
        opencode_binary.display()
    ))?;

    Ok(child)
}

fn find_opencode_binary() -> Result<PathBuf> {
    find_binary(&["opencode"], Some("opencode.cmd"),
        "Install: npm install -g @opencode-ai/opencode\n  \
         or visit: https://opencode.ai")
}

// ── Config management helpers (codex, opencode) ────────────────────────────
//
// Shared backup/restore infrastructure for agent config files. Both Codex
// (TOML) and OpenCode (JSON) follow the same pattern: snapshot pre-wrap
// state to a .dedroom-backup file, then restore/strip on unwrap.

const BACKUP_SUFFIX: &str = ".dedroom-backup";

/// Resolve a config directory with optional env-var override.
///
/// If `env_var` is set, uses that as the directory path directly.
/// Otherwise, returns `~/{default_subdir}`.
fn home_dir_with_override(env_var: &str, default_subdir: &str) -> PathBuf {
    if let Ok(dir) = std::env::var(env_var) {
        PathBuf::from(dir)
    } else {
        let home = if cfg!(windows) {
            std::env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string())
        } else {
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        };
        PathBuf::from(home).join(default_subdir)
    }
}

/// Snapshot a config file before the first injection if it doesn't already
/// have a backup. Creates parent directories if needed.
fn snapshot_before_inject(config_file: &Path, backup_file: &Path) -> Result<()> {
    let config_dir = config_file.parent()
        .ok_or_else(|| anyhow::anyhow!("No parent dir for config: {}", config_file.display()))?;
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("Failed to create {}", config_dir.display()))?;
    if !backup_file.exists() && config_file.exists() {
        std::fs::copy(config_file, backup_file)
            .with_context(|| format!("Failed to backup {}", config_file.display()))?;
    }
    Ok(())
}

/// Restore a config from its .dedroom-backup, or if none exists, apply a
/// clean function to the existing content. Returns (status, config_path).
fn restore_or_clean_config(
    config_file: &Path,
    backup_file: &Path,
    has_marker: impl Fn(&str) -> bool,
    clean_content: impl FnOnce(&str) -> Result<Option<String>>,
) -> Result<(String, PathBuf)> {
    if backup_file.exists() {
        std::fs::copy(backup_file, config_file)
            .with_context(|| format!("Failed to restore {} from backup", config_file.display()))?;
        std::fs::remove_file(backup_file).ok();
        return Ok(("restored".to_string(), config_file.to_path_buf()));
    }

    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)
            .with_context(|| format!("Failed to read {}", config_file.display()))?;
        if has_marker(&content) {
            match clean_content(&content)? {
                Some(cleaned) if cleaned.trim().is_empty() => {
                    std::fs::remove_file(config_file).ok();
                    return Ok(("removed".to_string(), config_file.to_path_buf()));
                }
                Some(cleaned) => {
                    std::fs::write(config_file, &cleaned)
                        .with_context(|| format!("Failed to write cleaned {}", config_file.display()))?;
                    return Ok(("cleaned".to_string(), config_file.to_path_buf()));
                }
                None => {} // clean chose not to act
            }
        }
    }

    Ok(("noop".to_string(), config_file.to_path_buf()))
}

// ── Codex config.toml injection ────────────────────────────────────────────

const CODEX_TOP_MARKER: &str = "# --- DedrooM proxy (auto-injected by dedroom wrap codex) ---";
const CODEX_END_MARKER: &str = "# --- end DedrooM ---";

fn codex_config_paths() -> (PathBuf, PathBuf) {
    let dir = home_dir_with_override("CODEX_HOME", ".codex");
    let config = dir.join("config.toml");
    let backup = dir.join(format!("config.toml{BACKUP_SUFFIX}"));
    (config, backup)
}

/// Inject a Headroom proxy provider into Codex's config.toml.
fn inject_codex_provider_config(port: u16) -> Result<()> {
    let (config_file, backup_file) = codex_config_paths();
    snapshot_before_inject(&config_file, &backup_file)?;

    let block = format!(
        r#"{top}
model_provider = "headroom"
openai_base_url = "http://127.0.0.1:{port}/v1"

[model_providers.headroom]
name = "OpenAI via DedrooM proxy"
base_url = "http://127.0.0.1:{port}/v1"
supports_websockets = true

# Per-project savings header
env_http_headers = {{ "X-Headroom-Project" = "DEDROOM_PROJECT" }}
{end}
"#,
        top = CODEX_TOP_MARKER,
        port = port,
        end = CODEX_END_MARKER,
    );

    let content = if config_file.exists() {
        let existing = std::fs::read_to_string(&config_file)
            .context("Failed to read Codex config")?;
        let cleaned = strip_codex_headroom_blocks(&existing);
        format!("{}\n\n{}", block, cleaned)
    } else {
        block
    };

    std::fs::write(&config_file, &content).context("Failed to write Codex config")?;
    eprintln!("  Codex config: injected Headroom provider into {}", config_file.display());
    Ok(())
}

/// Restore Codex config from backup or strip markers.
fn restore_codex_provider_config() -> Result<(String, PathBuf)> {
    let (config_file, backup_file) = codex_config_paths();
    restore_or_clean_config(
        &config_file,
        &backup_file,
        |c| c.contains(CODEX_TOP_MARKER),
        |c| Ok(Some(strip_codex_headroom_blocks(c))),
    )
}

/// Remove DedrooM-managed marker blocks from Codex config content.
fn strip_codex_headroom_blocks(content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find(CODEX_TOP_MARKER) {
        let after_start = &result[start + CODEX_TOP_MARKER.len()..];
        if let Some(end_offset) = after_start.find(CODEX_END_MARKER) {
            let end_pos = start + CODEX_TOP_MARKER.len() + end_offset + CODEX_END_MARKER.len();
            result.drain(start..end_pos);
        } else {
            break;
        }
    }
    result.lines().map(|l| l.to_string()).collect::<Vec<_>>().join("\n").trim().to_string()
}

// ── OpenCode config injection ─────────────────────────────────────────────

fn opencode_config_paths() -> (PathBuf, PathBuf) {
    let dir = home_dir_with_override("OPENCODE_HOME", ".config/opencode");
    let config = dir.join("opencode.json");
    let backup = dir.join(format!("opencode.json{BACKUP_SUFFIX}"));
    (config, backup)
}

/// Inject a DedrooM provider into OpenCode's opencode.json config.
fn inject_opencode_provider_config(port: u16) -> Result<()> {
    let (config_file, backup_file) = opencode_config_paths();
    snapshot_before_inject(&config_file, &backup_file)?;

    let proxy_url = format!("http://127.0.0.1:{}/v1", port);
    let headroom_entry = serde_json::json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": "DedrooM Proxy",
        "options": { "baseURL": proxy_url },
        "models": {
            "claude-sonnet-4-6": {
                "name": "Claude Sonnet 4.6",
                "limit": { "context": 200000, "output": 16384 }
            },
            "claude-opus-4-6": {
                "name": "Claude Opus 4.6",
                "limit": { "context": 200000, "output": 16384 }
            },
            "gpt-4o": {
                "name": "GPT-4o",
                "limit": { "context": 128000, "output": 16384 }
            }
        }
    });

    let content = if config_file.exists() {
        let existing = std::fs::read_to_string(&config_file)
            .context("Failed to read OpenCode config")?;
        let mut data: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&existing).unwrap_or_default();
        if let Some(providers) = data.get_mut("provider").and_then(|p| p.as_object_mut()) {
            providers.insert("headroom".to_string(), headroom_entry);
        } else {
            data.insert("provider".to_string(), serde_json::json!({ "headroom": headroom_entry }));
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(data))?
    } else {
        serde_json::to_string_pretty(&serde_json::json!({
            "provider": { "headroom": headroom_entry }
        }))?
    };

    std::fs::write(&config_file, &content).context("Failed to write OpenCode config")?;
    eprintln!("  OpenCode config: injected DedrooM provider into {}", config_file.display());
    Ok(())
}

/// Restore OpenCode config from backup or strip DedrooM provider.
fn restore_opencode_provider_config() -> Result<(String, PathBuf)> {
    let (config_file, backup_file) = opencode_config_paths();
    restore_or_clean_config(
        &config_file,
        &backup_file,
        |c| c.contains("DedrooM Proxy") || c.contains("dedroom wrap opencode"),
        |c| {
            let mut data: serde_json::Value = match serde_json::from_str(c) {
                Ok(v) => v,
                Err(_) => return Ok(None), // noop on parse failure
            };
            if let Some(obj) = data.as_object_mut() {
                if let Some(providers) = obj.get_mut("provider").and_then(|p| p.as_object_mut()) {
                    providers.remove("headroom");
                    if providers.is_empty() {
                        obj.remove("provider");
                    }
                }
                if obj.is_empty() {
                    return Ok(Some(String::new())); // triggers "removed" path
                }
            }
            serde_json::to_string_pretty(&data).map(Some)
                .context("Failed to serialize cleaned OpenCode config")
        },
    )
}

// ── Proxy stopping for unwrap ──────────────────────────────────────────────

/// Check if something is listening on a given port by trying a TCP connect.
fn check_port(port: u16) -> bool {
    use std::net::TcpStream;
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        Duration::from_millis(1000),
    )
    .is_ok()
}

/// Query the proxy /health endpoint to get its runtime config (including PID).
fn query_proxy_pid(port: u16) -> Option<u32> {
    let url = format!("http://127.0.0.1:{}/health", port);
    match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?
        .get(&url)
        .send()
    {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().ok()?;
            body.get("config")
                .and_then(|c| c.get("pid"))
                .and_then(|p| p.as_u64())
                .map(|p| p as u32)
        }
        _ => None,
    }
}

// ── Proxy guard (RAII cleanup) ─────────────────────────────────────────────

/// RAII guard that kills the proxy child process when dropped.
struct ProxyGuard {
    child: Option<std::process::Child>,
}

impl ProxyGuard {
    fn new(child: std::process::Child) -> Self {
        Self { child: Some(child) }
    }

    fn kill(&mut self) {
        if let Some(ref mut child) = self.child {
            kill_process_by_id(child.id(), true);
        }
    }
}

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Kill a process by PID and wait for it to stop (platform-aware).
fn kill_process_by_id(pid: u32, wait: bool) {
    #[cfg(unix)]
    let _ = Command::new("kill")
        .arg(pid.to_string())
        .spawn();
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(&["/PID", &pid.to_string(), "/F"])
        .spawn();
    if wait {
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ── Wrap orchestration context ─────────────────────────────────────────────
//
// Shared boilerplate for all wrap_* functions: start proxy, register Ctrl+C
// handler, wait for proxy readiness, and wait for agent or Ctrl+C.

struct WrapContext {
    interrupted: Arc<AtomicBool>,
    _guard: ProxyGuard,
}

impl WrapContext {
    /// Start the proxy, register a Ctrl+C handler, and wait for readiness.
    async fn new(port: u16, config: &Path) -> Result<Self> {
        let proxy = start_proxy(port, config)?;
        let guard = ProxyGuard::new(proxy);
        let interrupted = Arc::new(AtomicBool::new(false));

        // Register Ctrl+C handler
        {
            let interrupted = interrupted.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                eprintln!("\n  Received Ctrl+C. Shutting down...");
                interrupted.store(true, Ordering::SeqCst);
            });
        }

        wait_for_proxy(port, 60).await?;
        Ok(Self { interrupted, _guard: guard })
    }

    /// Wait for a launched agent subprocess to exit, or return early on Ctrl+C.
    async fn wait_for_agent(&self, agent: &mut std::process::Child, name: &str) -> Result<()> {
        wait_for_agent(agent, name, &self.interrupted).await
    }

    /// Poll until Ctrl+C is received (for GUI-only agents like cursor, cline).
    async fn block_until_ctrlc(&self) -> Result<()> {
        loop {
            if self.interrupted.load(Ordering::SeqCst) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

// ── RTK instructions block ──────────────────────────────────────────────────
//
// Injected into .clinerules (or AGENTS.md) so the LLM uses rtk-prefixed
// commands for token-efficient tool calls.

const RTK_INSTRUCTIONS_BLOCK: &str = r#"<!-- headroom:rtk-instructions -->
# RTK (Rust Token Killer) - Token-Optimized Commands

When running shell commands, **always prefix with `rtk`**. This reduces context
usage by 60-90% with zero behavior change. If rtk has no filter for a command,
it passes through unchanged — so it is always safe to use.

## Key Commands
```bash
# Git (59-80% savings)
rtk git status          rtk git diff            rtk git log

# Files & Search (60-75% savings)
rtk ls <path>           rtk read <file>         rtk grep <pattern>
rtk find <pattern>      rtk diff <file>

# Test (90-99% savings) — shows failures only
rtk pytest tests/       rtk cargo test          rtk test <cmd>

# Build & Lint (80-90% savings) — shows errors only
rtk tsc                 rtk lint                rtk cargo build
rtk prettier --check    rtk mypy                rtk ruff check

# Analysis (70-90% savings)
rtk err <cmd>           rtk log <file>          rtk json <file>
rtk summary <cmd>       rtk deps                rtk env

# GitHub (26-87% savings)
rtk gh pr view <n>      rtk gh run list         rtk gh issue list

# Infrastructure (85% savings)
rtk docker ps           rtk kubectl get         rtk docker logs <c>

# Package managers (70-90% savings)
rtk pip list            rtk pnpm install        rtk npm run <script>
```

## Rules
- In command chains, prefix each segment: `rtk git add . && rtk git commit -m "msg"`
- For debugging, use raw command without rtk prefix
- `rtk proxy <cmd>` runs command without filtering but tracks usage
<!-- /headroom:rtk-instructions -->
"#;

// ── Wrap implementations ────────────────────────────────────────────────────

/// Run Claude Code through the DedrooM proxy.
async fn wrap_claude(port: u16, config: &Path, agent_args: &[String]) -> Result<()> {
    let ctx = WrapContext::new(port, config).await?;
    let mut agent = launch_claude(port, agent_args).context("Failed to launch Claude Code")?;
    ctx.wait_for_agent(&mut agent, "Claude Code").await
}

/// Run Codex CLI through the DedrooM proxy.
///
/// Injects a Headroom provider into ~/.codex/config.toml so both API-key
/// and subscription (ChatGPT) users route through the proxy.
async fn wrap_codex(port: u16, config: &Path, agent_args: &[String]) -> Result<()> {
    eprintln!("  Wrapping Codex CLI via DedrooM proxy on port {}...", port);
    inject_codex_provider_config(port)?;
    let ctx = WrapContext::new(port, config).await?;
    let mut agent = match launch_codex(port, agent_args) {
        Ok(c) => c,
        Err(e) => { restore_codex_provider_config().ok(); return Err(e); }
    };
    let result = ctx.wait_for_agent(&mut agent, "Codex CLI").await;
    restore_codex_provider_config().ok();
    result
}

/// Run aider through the DedrooM proxy.
async fn wrap_aider(port: u16, config: &Path, agent_args: &[String]) -> Result<()> {
    let ctx = WrapContext::new(port, config).await?;
    let mut agent = launch_aider(port, agent_args).context("Failed to launch aider")?;
    ctx.wait_for_agent(&mut agent, "aider").await
}

// ── Cursor integration ─────────────────────────────────────────────────────

/// Detect Cursor installation.
fn find_cursor_location() -> Option<PathBuf> {
    // Check if `cursor` CLI is in PATH (works when user has installed it)
    if let Ok(path) = which::which("cursor") {
        return Some(path);
    }

    // macOS: check standard app bundle
    #[cfg(target_os = "macos")]
    {
        let app_bundle = PathBuf::from("/Applications/Cursor.app");
        if app_bundle.exists() {
            return Some(app_bundle);
        }
    }

    // Linux: check AppImage or snap
    #[cfg(target_os = "linux")]
    {
        let snaps = [
            PathBuf::from("/snap/bin/cursor"),
            PathBuf::from("/usr/local/bin/cursor"),
        ];
        for p in &snaps {
            if p.exists() {
                return Some(p.clone());
            }
        }
    }

    None
}

/// Inject proxy settings into Cursor's settings.json.
fn inject_cursor_settings(port: u16) -> Result<()> {
    let settings_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".cursor");
    let settings_file = settings_dir.join("settings.json");

    let mut settings: serde_json::Value = if settings_file.exists() {
        let content = std::fs::read_to_string(&settings_file)
            .unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let openai_url = format!("http://127.0.0.1:{}/v1", port);
    let anthropic_url = format!("http://127.0.0.1:{}", port);

    if let Some(obj) = settings.as_object_mut() {
        obj.insert(
            "cursor.general.overrideOpenAIBaseURL".to_string(),
            serde_json::json!(openai_url),
        );
        obj.insert(
            "cursor.general.overrideAnthropicBaseURL".to_string(),
            serde_json::json!(anthropic_url),
        );
    }

    std::fs::create_dir_all(&settings_dir)?;
    std::fs::write(&settings_file, serde_json::to_string_pretty(&settings)?)?;
    eprintln!("  ✅ Proxy settings injected into {}", settings_file.display());
    Ok(())
}

/// Launch Cursor app bundle on macOS.
fn launch_cursor_app() -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err(anyhow::anyhow!("Automatic launch not supported on this platform"));
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("/Applications/Cursor.app")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to launch Cursor.app — is it installed?")?;
        eprintln!("  🚀 Launched Cursor.app");
        Ok(())
    }
}

/// Run Cursor through the DedrooM proxy.
///
/// Auto-detects Cursor installation, injects proxy settings into
/// ~/.cursor/settings.json, and launches Cursor if possible.
/// Falls back to printing setup instructions.
async fn wrap_cursor(port: u16, config: &Path) -> Result<()> {
    let ctx = WrapContext::new(port, config).await?;
    let openai_url = format!("http://127.0.0.1:{}/v1", port);
    let anthropic_url = format!("http://127.0.0.1:{}", port);

    eprintln!();
    eprintln!("  ╔═════════════════════════════════════════════════╗");
    eprintln!("  ║         DEDROOM WRAP: CURSOR                  ║");
    eprintln!("  ╚═════════════════════════════════════════════════╝");
    eprintln!();

    if find_cursor_location().is_some() {
        // Auto-inject settings
        if let Err(e) = inject_cursor_settings(port) {
            eprintln!("  ⚠️  Settings injection failed: {e}");
        }

        // Launch Cursor
        if let Err(e) = launch_cursor_app() {
            eprintln!("  ⚠️  Could not launch Cursor: {e}");
            eprintln!();
            eprintln!("  To launch Cursor manually and use the proxy:");
            eprintln!("    OpenAI Override Base URL: {}", openai_url);
            eprintln!("    Anthropic Override Base URL: {}", anthropic_url);
        } else {
            eprintln!();
            eprintln!("  Cursor settings have been configured.");
            eprintln!("  Proxy is running. Press Ctrl+C to stop.");
        }
    } else {
        eprintln!("  Cursor not found at standard locations.");
        eprintln!("  To install the 'cursor' command in PATH:");
        eprintln!("    1. Open Cursor");
        eprintln!("    2. Cmd+Shift+P → 'Install cursor command in PATH'");
        eprintln!();
        eprintln!("  Manual proxy configuration:");
        eprintln!("    OpenAI Override Base URL:  {}", openai_url);
        eprintln!("    Anthropic Override Base URL: {}", anthropic_url);
        eprintln!();
        eprintln!("  Cursor Settings > Models > Override OpenAI/Anthropic Base URL");
    }

    eprintln!();
    eprintln!("  Press Ctrl+C to stop the proxy.");
    eprintln!();

    ctx.block_until_ctrlc().await
}

/// Run OpenCode through the DedrooM proxy.
async fn wrap_opencode(port: u16, config: &Path, agent_args: &[String]) -> Result<()> {
    eprintln!("  Wrapping OpenCode via DedrooM proxy on port {}...", port);
    inject_opencode_provider_config(port)?;
    let ctx = WrapContext::new(port, config).await?;
    let mut agent = match launch_opencode(port, agent_args) {
        Ok(c) => c,
        Err(e) => { restore_opencode_provider_config().ok(); return Err(e); }
    };
    let result = ctx.wait_for_agent(&mut agent, "OpenCode").await;
    restore_opencode_provider_config().ok();
    result
}

// ── Cline integration ─────────────────────────────────────────────────────

/// Detect Cline/VS Code installation.
fn find_cline_location() -> Option<PathBuf> {
    // Check if `code` CLI is in PATH
    if let Ok(path) = which::which("code") {
        return Some(path);
    }

    // macOS: standard VS Code app bundle
    #[cfg(target_os = "macos")]
    {
        let app_bundle = PathBuf::from("/Applications/Visual Studio Code.app");
        if app_bundle.exists() {
            return Some(app_bundle);
        }
        // Also check Code - Insiders
        let insiders = PathBuf::from("/Applications/Visual Studio Code - Insiders.app");
        if insiders.exists() {
            return Some(insiders);
        }
    }

    None
}

/// Find the VS Code settings.json path (platform-aware).
fn vscode_settings_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        Some(home.join("Library/Application Support/Code/User/settings.json"))
    }
    #[cfg(target_os = "linux")]
    {
        Some(home.join(".config/Code/User/settings.json"))
    }
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|p| PathBuf::from(p).join("Code/User/settings.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

/// Inject proxy settings into VS Code's settings.json for Cline.
///
/// Cline uses VS Code's settings to configure its API provider.
/// We inject the proxy base URLs into the user settings.
fn inject_cline_settings(port: u16) -> Result<()> {
    let settings_file = vscode_settings_path()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine VS Code settings path"))?;

    // Ensure parent directory exists
    if let Some(parent) = settings_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut settings: serde_json::Value = if settings_file.exists() {
        let content = std::fs::read_to_string(&settings_file)
            .unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let anthropic_url = format!("http://127.0.0.1:{}", port);
    let openai_url = format!("http://127.0.0.1:{}/v1", port);

    if let Some(obj) = settings.as_object_mut() {
        // Cline API provider settings (contributed by the extension)
        obj.insert(
            "cline.apiProvider".to_string(),
            serde_json::json!("anthropic"),
        );
        obj.insert(
            "cline.anthropicBaseUrl".to_string(),
            serde_json::json!(anthropic_url),
        );
        obj.insert(
            "cline.openAiBaseUrl".to_string(),
            serde_json::json!(openai_url),
        );
        obj.insert(
            "cline.openAiApiKey".to_string(),
            serde_json::json!("sk-dedroom-proxy"),
        );
    }

    std::fs::write(&settings_file, serde_json::to_string_pretty(&settings)?)?;
    eprintln!("  ✅ Proxy settings injected into {}", settings_file.display());
    Ok(())
}

/// Run Cline through the DedrooM proxy.
///
/// Injects RTK guidance into .clinerules, auto-detects VS Code/Cline,
/// injects proxy settings into VS Code settings.json, and launches VS Code.
/// Falls back to printing setup instructions.
async fn wrap_cline(port: u16, config: &Path) -> Result<()> {
    let ctx = WrapContext::new(port, config).await?;

    // Inject RTK instructions into .clinerules (always, even without Cline)
    if let Some(path) = std::env::current_dir().ok().map(|d| d.join(".clinerules")) {
        if path.exists() {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if !existing.contains("<!-- headroom:rtk-instructions -->") {
                std::fs::write(&path, format!("{}\n\n{}", existing.trim(), RTK_INSTRUCTIONS_BLOCK)).ok();
            }
        } else {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
            std::fs::write(&path, RTK_INSTRUCTIONS_BLOCK).ok();
        }
        eprintln!("  rtk instructions injected into .clinerules");
    }

    let anthropic_url = format!("http://127.0.0.1:{}", port);
    let openai_url = format!("http://127.0.0.1:{}/v1", port);

    eprintln!();
    eprintln!("  ╔═════════════════════════════════════════════════╗");
    eprintln!("  ║         DEDROOM WRAP: CLINE                   ║");
    eprintln!("  ╚═════════════════════════════════════════════════╝");
    eprintln!();

    if find_cline_location().is_some() {
        // Auto-inject settings into VS Code
        if let Err(e) = inject_cline_settings(port) {
            eprintln!("  ⚠️  Settings injection failed: {e}");
        }

        // Launch VS Code
        #[cfg(target_os = "macos")]
        {
            match Command::new("open")
                .arg("/Applications/Visual Studio Code.app")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(_) => eprintln!("  🚀 Launched VS Code"),
                Err(e) => eprintln!("  ⚠️  Could not launch VS Code: {e}"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Try `code` CLI command
            if which::which("code").is_ok() {
                match Command::new("code")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(_) => eprintln!("  🚀 Launched VS Code"),
                    Err(_) => {}
                }
            }
        }

        eprintln!();
        eprintln!("  Cline settings have been configured.");
        eprintln!("  Press Ctrl+C to stop the proxy.");
    } else {
        eprintln!("  VS Code not found at standard locations.");
        eprintln!();
        eprintln!("  Manual proxy configuration:");
        eprintln!("    Anthropic Base URL:         {}", anthropic_url);
        eprintln!("    OpenAI Compatible Base URL: {}", openai_url);
        eprintln!();
        eprintln!("  VS Code Settings > Cline > API Provider");
        eprintln!("  Press Ctrl+C to stop the proxy.");
    }

    eprintln!();

    ctx.block_until_ctrlc().await
}

/// Undo wrap changes for a given agent, matching headroom's behaviour exactly.
///
/// * Codex: restores ~/.codex/config.toml from backup or strips markers.
/// * Claude: runtime-only env vars — no persistent state to undo.
/// * Aider: runtime-only env vars — no persistent state to undo.
/// * Cursor: prints instructions to revert manual settings.
///
/// All agents: attempts to stop any running DedrooM proxy on the port.
async fn unwrap_agent(agent: &str, port: u16) -> Result<()> {
    let agent_upper = agent.to_uppercase();

    eprintln!();
    eprintln!("  ╔═════════════════════════════════════════════════╗");
    eprintln!("  ║         DEDROOM UNWRAP: {:<19} ║", agent_upper);
    eprintln!("  ╚═════════════════════════════════════════════════╝");
    eprintln!();

    match agent {
        "codex" => {
            match restore_codex_provider_config()? {
                (status, config_file) if status == "restored" => {
                    eprintln!("  Restored prior {} from pre-wrap backup.", config_file.display());
                }
                (status, config_file) if status == "cleaned" => {
                    eprintln!("  Removed DedrooM block from {}; other content preserved.", config_file.display());
                }
                (status, config_file) if status == "removed" => {
                    eprintln!("  Removed {} (contained only DedrooM-written config).", config_file.display());
                }
                (_status, config_file) => {
                    // noop
                    let codex_hint = if std::env::var("CODEX_HOME").is_err() {
                        ""
                    } else {
                        " If you wrapped Codex with CODEX_HOME, rerun unwrap with the same environment variable."
                    };
                    eprintln!("  Nothing to undo: {} has no DedrooM wrap markers.{}", config_file.display(), codex_hint);
                }
            }
        }
        "claude" => {
            eprintln!("  Claude Code uses runtime-only env vars (ANTHROPIC_BASE_URL).");
            eprintln!("  No persistent state to clean up.");
        }
        "aider" => {
            eprintln!("  Aider uses runtime-only env vars (OPENAI_API_BASE, ANTHROPIC_BASE_URL).");
            eprintln!("  No persistent state to clean up.");
        }
        "cursor" => {
            eprintln!("  Cursor uses manual settings configuration.");
            eprintln!("  To finish unwrapping, revert the base URLs in Cursor Settings:");
            eprintln!("    Settings > Models > OpenAI API Key > Override OpenAI Base URL > clear it");
        }
        "opencode" => {
            match restore_opencode_provider_config()? {
                (status, config_file) if status == "restored" => {
                    eprintln!("  Restored prior {} from pre-wrap backup.", config_file.display());
                }
                (status, config_file) if status == "cleaned" => {
                    eprintln!("  Removed DedrooM provider from {}; other content preserved.", config_file.display());
                }
                (status, config_file) if status == "removed" => {
                    eprintln!("  Removed {} (contained only DedrooM-written config).", config_file.display());
                }
                (_status, config_file) => {
                    eprintln!("  Nothing to undo: {} has no DedrooM wrap markers.", config_file.display());
                }
            }
        }
        "cline" => {
            eprintln!("  Cline uses manual settings configuration.");
            eprintln!("  To finish unwrapping, revert the base URLs in Cline's VS Code settings:");
            eprintln!("    Settings > Cline > API Provider > clear the base URLs");
            eprintln!("  You may also want to remove the rtk instructions from .clinerules.");
        }
        other => bail!("Unsupported agent: {}. Supported: claude, codex, aider, cursor, opencode, cline", other),
    }

    // Stop any running proxy
    eprintln!();
    if check_port(port) {
        if let Some(pid) = query_proxy_pid(port) {
            kill_process_by_id(pid, false);
            // Wait for port to free up (up to 5 seconds)
            let freed = (0..50).any(|_| {
                std::thread::sleep(Duration::from_millis(100));
                !check_port(port)
            });
            if freed {
                eprintln!("  Stopped local DedrooM proxy on port {}.", port);
            } else {
                eprintln!("  Warning: failed to stop DedrooM proxy on port {}; stop it manually.", port);
            }
        } else {
            eprintln!("  Warning: port {} is in use, but it did not look like DedrooM; left it running.", port);
        }
    } else {
        eprintln!("  No local DedrooM proxy detected on port {}.", port);
    }

    eprintln!();
    eprintln!("✓ {} is no longer durably wrapped by DedrooM.", capitalize(agent));
    eprintln!();
    Ok(())
}

// ── Doctor command ─────────────────────────────────────────────────────────
//
// DedrooM Doctor runs diagnostic checks to verify:
//  - Proxy liveness (GET /health)
//  - Claude Code routing (~/.claude/settings.json)
//  - Codex routing (~/.codex/config.toml)
//  - Shell environment (ANTHROPIC_BASE_URL / OPENAI_BASE_URL)
//  - Savings flow (GET /admin/stats)
//
// Exit codes: 0 = all pass, 1 = warnings, 2 = any failure.

const PASS: &str = "pass";
const WARN: &str = "warn";
const FAIL: &str = "fail";
const SKIP: &str = "skip";

struct CheckResult {
    name: String,
    status: String,
    summary: String,
    hint: Option<String>,
}

fn check_proxy_liveness(health: Option<&serde_json::Value>, base_url: &str) -> CheckResult {
    let name = "proxy".to_string();
    match health {
        None => CheckResult {
            name,
            status: FAIL.to_string(),
            summary: format!("not reachable at {}", base_url),
            hint: Some("start it with: dedroom proxy".to_string()),
        },
        Some(body) => {
            let status = body
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            CheckResult {
                name,
                status: PASS.to_string(),
                summary: format!("running at {} (status: {})", base_url, status),
                hint: None,
            }
        }
    }
}

fn check_version_drift(
    health: Option<&serde_json::Value>,
    installed_version: &str,
) -> CheckResult {
    let name = "version".to_string();
    match health {
        None => CheckResult {
            name,
            status: SKIP.to_string(),
            summary: "proxy not reachable".to_string(),
            hint: None,
        },
        Some(body) => {
            let running = body
                .get("service")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            CheckResult {
                name,
                status: PASS.to_string(),
                summary: format!("proxy {} (installed v{})", running, installed_version),
                hint: None,
            }
        }
    }
}

fn claude_settings_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

fn check_claude_routing(port: u16) -> CheckResult {
    let name = "claude".to_string();
    let settings_path = match claude_settings_path() {
        Some(p) => p,
        None => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: "could not determine home directory".to_string(),
                hint: None,
            }
        }
    };

    if !settings_path.exists() {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no ~/.claude/settings.json)".to_string(),
            hint: Some("wrap it: dedroom wrap claude".to_string()),
        };
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!("could not read {}: {}", settings_path.display(), e),
                hint: None,
            }
        }
    };

    let payload: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!("could not parse {}", settings_path.display()),
                hint: None,
            }
        }
    };

    let base_url = payload
        .get("env")
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if base_url.is_empty() {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no ANTHROPIC_BASE_URL in settings env)".to_string(),
            hint: Some("wrap it: dedroom wrap claude".to_string()),
        };
    }

    classify_routing_url(
        &name,
        base_url,
        port,
        &settings_path.display().to_string(),
    )
}

fn check_codex_routing(port: u16) -> CheckResult {
    let name = "codex".to_string();
    let (config_file, _) = codex_config_paths();

    if !config_file.exists() {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no ~/.codex/config.toml)".to_string(),
            hint: Some("wrap it: dedroom wrap codex".to_string()),
        };
    }

    let text = match std::fs::read_to_string(&config_file) {
        Ok(t) => t,
        Err(e) => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!("could not read {}: {}", config_file.display(), e),
                hint: None,
            }
        }
    };

    if !text.contains("[model_providers.headroom]") {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no DedrooM provider in config.toml)".to_string(),
            hint: Some("wrap it: dedroom wrap codex".to_string()),
        };
    }

    let base_url_re =
        regex::Regex::new(r#"base_url\s*=\s*"https?://(?:127\.0\.0\.1|localhost):(\d+)"#)
            .unwrap();
    if let Some(caps) = base_url_re.captures(&text) {
        let found_port: u16 = caps[1].parse().unwrap_or(0);
        if found_port != port {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!(
                    "routed to port {}, but doctor probed port {}",
                    found_port, port
                ),
                hint: Some(format!("re-run with: dedroom doctor --port {}", found_port)),
            };
        }
    }

    CheckResult {
        name,
        status: PASS.to_string(),
        summary: format!("routed ({})", config_file.display()),
        hint: None,
    }
}

fn check_opencode_routing(port: u16) -> CheckResult {
    let name = "opencode".to_string();
    let (config_file, _) = opencode_config_paths();

    if !config_file.exists() {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no ~/.config/opencode/opencode.json)".to_string(),
            hint: Some("wrap it: dedroom wrap opencode".to_string()),
        };
    }

    let content = match std::fs::read_to_string(&config_file) {
        Ok(c) => c,
        Err(e) => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!("could not read {}: {}", config_file.display(), e),
                hint: None,
            }
        }
    };

    // Quick string check before parsing JSON
    if !content.contains("DedrooM Proxy") && !content.contains("dedroom wrap opencode") {
        return CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no DedrooM provider in opencode.json)".to_string(),
            hint: Some("wrap it: dedroom wrap opencode".to_string()),
        };
    }

    // Parse JSON and check the baseURL
    let data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            return CheckResult {
                name,
                status: WARN.to_string(),
                summary: format!("could not parse {}", config_file.display()),
                hint: None,
            }
        }
    };

    if let Some(base_url) = data
        .get("provider")
        .and_then(|p| p.get("headroom"))
        .and_then(|h| h.get("options"))
        .and_then(|o| o.get("baseURL"))
        .and_then(|v| v.as_str())
    {
        let loopback_re =
            regex::Regex::new(r#"^https?://(?:127\.0\.0\.1|localhost):(\d+)/v1$"#).unwrap();
        if let Some(caps) = loopback_re.captures(base_url) {
            let found_port: u16 = caps[1].parse().unwrap_or(0);
            if found_port != port {
                return CheckResult {
                    name,
                    status: WARN.to_string(),
                    summary: format!(
                        "routed to port {}, but doctor probed port {}",
                        found_port, port
                    ),
                    hint: Some(format!("re-run with: dedroom doctor --port {}", found_port)),
                };
            }
        }
    }

    CheckResult {
        name,
        status: PASS.to_string(),
        summary: format!("routed ({})", config_file.display()),
        hint: None,
    }
}

fn check_cursor_routing() -> CheckResult {
    CheckResult {
        name: "cursor".to_string(),
        status: SKIP.to_string(),
        summary: "manual GUI setup — routing cannot be verified programmatically".to_string(),
        hint: Some("run `dedroom wrap cursor` to configure Cursor through the proxy".to_string()),
    }
}

fn check_cline_routing() -> CheckResult {
    let name = "cline".to_string();
    let clinerules = std::env::current_dir().ok().map(|d| d.join(".clinerules"));

    match clinerules {
        Some(path) if path.exists() => {
            match std::fs::read_to_string(&path) {
                Ok(content) if content.contains("<!-- headroom:rtk-instructions -->") => {
                    CheckResult {
                        name,
                        status: PASS.to_string(),
                        summary: format!("rTK guidance present in {} — likely wrapped", path.display()),
                        hint: None,
                    }
                }
                Ok(_) => CheckResult {
                    name,
                    status: WARN.to_string(),
                    summary: format!("{} exists but no DedrooM RTK marker found", path.display()),
                    hint: Some("wrap it: dedroom wrap cline".to_string()),
                },
                Err(e) => CheckResult {
                    name,
                    status: WARN.to_string(),
                    summary: format!("could not read {}: {}", path.display(), e),
                    hint: None,
                },
            }
        }
        _ => CheckResult {
            name,
            status: WARN.to_string(),
            summary: "not routed (no .clinerules in project directory)".to_string(),
            hint: Some("wrap it: dedroom wrap cline".to_string()),
        },
    }
}

fn check_shell_env(port: u16) -> CheckResult {
    check_env_var_routing(
        "shell env",
        &["ANTHROPIC_BASE_URL", "OPENAI_BASE_URL"],
        port,
        &format!("export ANTHROPIC_BASE_URL=http://127.0.0.1:{} (or launch via dedroom wrap)", port),
    )
}

fn check_aider_routing(port: u16) -> CheckResult {
    check_env_var_routing(
        "aider",
        &["OPENAI_API_BASE", "OPENAI_BASE_URL", "ANTHROPIC_BASE_URL"],
        port,
        &format!("export OPENAI_API_BASE=http://127.0.0.1:{}/v1 (or launch via dedroom wrap aider)", port),
    )
}

/// Check if any of the given env vars point at the local proxy on `port`.
fn check_env_var_routing(name: &str, vars: &[&str], port: u16, none_hint: &str) -> CheckResult {
    for var in vars {
        if let Some(value) = std::env::var(var).ok().filter(|v| !v.is_empty()) {
            return classify_routing_url(name, &value, port, var);
        }
    }
    CheckResult {
        name: name.to_string(),
        status: WARN.to_string(),
        summary: format!("{} unset — this shell bypasses the proxy", vars.join(" / ")),
        hint: Some(none_hint.to_string()),
    }
}

fn classify_routing_url(name: &str, url: &str, port: u16, source: &str) -> CheckResult {
    let loopback_re =
        regex::Regex::new(r#"^https?://(?:127\.0\.0\.1|localhost):(\d+)"#).unwrap();

    match loopback_re.captures(url.trim()) {
        None => CheckResult {
            name: name.to_string(),
            status: WARN.to_string(),
            summary: format!(
                "points at {}, not the local DedrooM proxy ({})",
                url, source
            ),
            hint: None,
        },
        Some(caps) => {
            let found_port: u16 = caps[1].parse().unwrap_or(0);
            if found_port != port {
                CheckResult {
                    name: name.to_string(),
                    status: WARN.to_string(),
                    summary: format!(
                        "routed to port {}, but doctor probed port {} ({})",
                        found_port, port, source
                    ),
                    hint: Some(format!(
                        "re-run with: dedroom doctor --port {}",
                        found_port
                    )),
                }
            } else {
                CheckResult {
                    name: name.to_string(),
                    status: PASS.to_string(),
                    summary: format!("routed via {}", source),
                    hint: None,
                }
            }
        }
    }
}

fn check_savings(stats: Option<&serde_json::Value>) -> CheckResult {
    let name = "savings".to_string();

    let savings = stats.and_then(|s| s.get("savings"));

    match savings {
        None => CheckResult {
            name,
            status: WARN.to_string(),
            summary: "no savings recorded yet".to_string(),
            hint: Some(
                "route a client through the proxy and make a request"
                    .to_string(),
            ),
        },
        Some(s) => {
            let tokens = s
                .get("total_compression_savings_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + s.get("total_loop_savings_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            let blocked = s
                .get("total_calls_blocked")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if tokens == 0 && blocked == 0 {
                CheckResult {
                    name,
                    status: WARN.to_string(),
                    summary: "no tokens saved yet".to_string(),
                    hint: Some(
                        "route a client through the proxy and make a request"
                            .to_string(),
                    ),
                }
            } else {
                CheckResult {
                    name,
                    status: PASS.to_string(),
                    summary: format!(
                        "{} tokens saved, {} calls blocked (proxy /admin/stats)",
                        tokens, blocked
                    ),
                    hint: None,
                }
            }
        }
    }
}

fn check_budget(stats: Option<&serde_json::Value>) -> CheckResult {
    let name = "budget".to_string();
    match stats {
        None => CheckResult {
            name,
            status: SKIP.to_string(),
            summary: "proxy not reachable".to_string(),
            hint: None,
        },
        Some(_s) => CheckResult {
            name,
            status: SKIP.to_string(),
            summary: "budget tracking not yet implemented in DedrooM proxy".to_string(),
            hint: None,
        },
    }
}

fn render_doctor(checks: &[CheckResult], port: u16, version: &str) {
    eprintln!();
    eprintln!("  ╔═════════════════════════════════════════════════╗");
    eprintln!("  ║         DEDROOM DOCTOR                         ║");
    eprintln!("  ╚═════════════════════════════════════════════════╝");
    eprintln!("  v{} · port {}", version, port);
    eprintln!();

    for check in checks {
        let glyph = match check.status.as_str() {
            PASS => "✓",
            WARN => "⚠",
            FAIL => "✗",
            SKIP => "·",
            _ => "?",
        };
        eprintln!("  {} {}  {}", glyph, check.status.to_uppercase(), check.name);
        eprintln!("         {}", check.summary);
        if let Some(hint) = &check.hint {
            eprintln!("         > {}", hint);
        }
        eprintln!();
    }

    let fails = checks.iter().filter(|c| c.status == FAIL).count();
    let warns = checks.iter().filter(|c| c.status == WARN).count();

    if fails > 0 || warns > 0 {
        eprintln!("  {} failure(s), {} warning(s)", fails, warns);
    } else {
        eprintln!("  All checks passed ✓");
    }
}

/// Run DedrooM Doctor diagnostics.
///
/// Probes the proxy health endpoint, checks agent routing configs, shell env,
/// and savings flow. Returns exit code: 0 = all pass, 1 = warnings, 2 = failure.
fn doctor(port: u16, emit_json: bool) -> Result<i32> {
    let base_url = format!("http://127.0.0.1:{}", port);

    // Build a single reqwest client for all probe requests
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok();

    // Fetch health and stats with the shared client
    let health = client.as_ref().and_then(|client| {
        client
            .get(format!("{}/health", base_url))
            .send()
            .ok()
            .and_then(|r| {
                if r.status().is_success() {
                    r.json::<serde_json::Value>().ok()
                } else {
                    None
                }
            })
    });

    let stats = health.as_ref().and_then(|_| {
        client.as_ref().and_then(|client| {
            client
                .get(format!("{}/admin/stats", base_url))
                .send()
                .ok()
                .and_then(|r| {
                    if r.status().is_success() {
                        r.json::<serde_json::Value>().ok()
                    } else {
                        None
                    }
                })
        })
    });

    let installed_version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");

    let checks = vec![
        check_proxy_liveness(health.as_ref(), &base_url),
        check_version_drift(health.as_ref(), installed_version),
        check_claude_routing(port),
        check_codex_routing(port),
        check_opencode_routing(port),
        check_aider_routing(port),
        check_cursor_routing(),
        check_cline_routing(),
        check_shell_env(port),
        check_savings(stats.as_ref()),
        check_budget(stats.as_ref()),
    ];

    let has_fail = checks.iter().any(|c| c.status == FAIL);
    let has_warn = checks.iter().any(|c| c.status == WARN);

    let exit_code: i32 = if has_fail {
        2
    } else if has_warn {
        1
    } else {
        0
    };

    if emit_json {
        let json_checks: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "status": c.status,
                    "summary": c.summary,
                    "hint": c.hint,
                })
            })
            .collect();

        let output = serde_json::json!({
            "port": port,
            "version": installed_version,
            "exit_code": exit_code,
            "checks": json_checks,
        });

        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        render_doctor(&checks, port, installed_version);
    }

    Ok(exit_code)
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Poll agent process until it exits or Ctrl+C is received.
async fn wait_for_agent(
    agent: &mut std::process::Child,
    name: &str,
    interrupted: &AtomicBool,
) -> Result<()> {
    loop {
        if interrupted.load(Ordering::SeqCst) {
            let _ = agent.kill();
            let _ = agent.wait();
            bail!("Interrupted by user.");
        }

        match agent.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    eprintln!("\n  {} finished successfully.", name);
                } else if let Some(code) = status.code() {
                    eprintln!("\n  {} exited with code {}", name, code);
                } else {
                    eprintln!("\n  {} terminated by signal.", name);
                }
                return Ok(());
            }
            Ok(None) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => {
                bail!("Failed to check {} status: {}", name, e);
            }
        }
    }
}

// ── Main ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize minimal logging (let the proxy handle detailed logs)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cmd = parse_args()?;

    match cmd {
        CliCommand::Wrap {
            agent,
            port,
            config,
            agent_args,
        } => {
            match agent.as_str() {
                "claude" => wrap_claude(port, &config, &agent_args).await?,
                "codex" => wrap_codex(port, &config, &agent_args).await?,
                "aider" => wrap_aider(port, &config, &agent_args).await?,
                "cursor" => wrap_cursor(port, &config).await?,
                "opencode" => wrap_opencode(port, &config, &agent_args).await?,
                "cline" => wrap_cline(port, &config).await?,
                other => bail!("Unsupported agent: {}. Supported: claude, codex, aider, cursor, opencode, cline", other),
            }
            Ok(())
        }
        CliCommand::Unwrap { agent, port } => {
            unwrap_agent(&agent, port).await?;
            Ok(())
        }
        CliCommand::Doctor { port, emit_json } => {
            let exit_code = doctor(port, emit_json)?;
            std::process::exit(exit_code);
        }
        CliCommand::Proxy { port, config } => {
            // Delegate to dedroom-proxy by re-execing
            let proxy_path = find_proxy_binary()?;

            eprintln!("  Starting DedrooM proxy on port {}...", port);

            let status = Command::new(&proxy_path)
                .arg("--port")
                .arg(port.to_string())
                .arg("--config")
                .arg(config.to_string_lossy().to_string())
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("Failed to start proxy")?;

            if !status.success() && let Some(code) = status.code() {
                bail!("Proxy exited with code {}", code);
            }
            Ok(())
        }
        CliCommand::Dash { port, url } => {
            // Launch the TUI dashboard
            let mut child = launch_dash(port, url)?;
            let status = child.wait()?;
            if !status.success() && let Some(code) = status.code() {
                bail!("Dashboard exited with code {}", code);
            }
            Ok(())
        }
    }
}
