# Changelog

## v0.5.0 (2026-07-08)

Self-healing, TUI dashboard, CONNECT tunnel, and major reliability improvements.

### Features

- **Self-healing**: adaptive sliding-window loop detection automatically blocks repeated failing tool calls. Cross-session learning stores failure signatures across sessions and injects context-aware hints when patterns reappear. Instincts engine tightens error-rate thresholds as trust score drops. Healing mutations trigger hints that nudge the agent to adapt.
- **TUI dashboard**: `dedroom dash` command shows per-tool savings, loop detection stats, and healing mutation success rates in a real-time terminal dashboard. Supports remote proxy access.
- **CONNECT tunnel**: HTTPS proxy support via HTTP CONNECT tunneling. Automatic CONNECT port allocation (port + 1).
- **Dynamic model discovery**: OpenCode provider config now fetches available models from the upstream API when --upstream-url is provided.

### Bug Fixes

- **Async migration**: all `reqwest::blocking::Client` usages replaced with async `reqwest::Client` — fixes "Cannot drop a runtime in a context where blocking is not allowed" panic in `status`, `doctor`, `stop`, and `unwrap` commands.
- `strip_suffix("/v1")` in `forward_to_upstream` now uses `unwrap_or(base_clean)` instead of `unwrap()` for defensive safety.
- All `std::thread::sleep` calls in async functions replaced with `tokio::time::sleep().await`.
- Thread safety fixes: atomic state management, dead code cleanup.
- 12 clippy warnings fixed.

### Performance

- Pipeline throughput: ~5.4 us (in-memory), ~260 us (SQLite).
- End-to-end intercept latency: ~1.3 ms median.
- Compression ratios: 70-94% per compressor (SmartCrusher, CodeCompressor, LogCompressor, TextCompressor).
- Loop detection on iterative debugging: up to 64% token reduction.

### Python Package

- Bundled CLI binaries in wheel — `pip install dedroom` gives a fully working `dedroom` command.
- Python API: `DedrooM`, `detect_loop`, `compress_text`.
- Full example: `examples/security_audit_agent.py`.

### Dependencies

- Added: `ratatui`, `crossterm` for TUI dashboard.
- Added: `futures`, `tokio-stream`, `bytes` for streaming proxy support.

---

## v0.4.0 (2026-07-06)

Attribution engine, PII redaction, SQLite persistence, and benchmarks.

### Features

- **Attribution engine**: per-tool token savings tracking. Shows exactly how much each compressor saves per tool call.
- **PII redaction**: 14 regex patterns plus entropy-based detection for API keys, tokens, and secrets. Runs locally before forwarding.
- **SQLite persistence**: optional backend for CCR deduplication and loop detection history. WAL mode, batch pruning, persisted adaptive thresholds.
- **Shadow mode**: process tool calls in ghost mode — log verdicts to event stream but never block requests.
- **Azure OpenAI support**: dynamic model discovery from upstream API.

### Performance

- Added `cargo bench --features sqlite` benchmarks for pipeline and loop history.
- Compression quality benchmarks across multiple languages.

### Python Package

- Initial Python bindings via PyO3.
- `maturin` build system for wheel generation.

---

## v0.3.x (2026-07-05)

Loop detection, event logging, and proxy improvements.

### v0.3.5

- Event log format improvements.

### v0.3.4

- Config file discovery fixes.

### v0.3.3

- Proxy stability improvements.

### v0.3.2

- Improved error messages in CLI.

### v0.3.1

- Bug fixes in loop detection edge cases.

### v0.3.0

- **Shadow mode**: event logging captures loop detection verdicts, compression ratios, and trust scores in NDJSON format.
- Improved loop detection with adaptive thresholds.
- Telemetry: event log tracks tilt index, compression ratios, and per-tool savings.

---

## v0.2.x (2026-07-05)

Agent wrapping, config management, and diagnostics.

### v0.2.1

- Fixed Codex config injection path resolution.

### v0.2.0

- **Agent wrapping**: `dedroom wrap claude`, `dedroom wrap codex`, `dedroom wrap aider`, `dedroom wrap cursor`, `dedroom wrap opencode`, `dedroom wrap cline`.
- **Config management**: backup/restore for agent config files (Codex config.toml, OpenCode opencode.json).
- **Diagnostics**: `dedroom doctor` runs 11 health checks (proxy liveness, agent routing, shell env, savings flow).
- **Status command**: `dedroom status` shows running state, PID, uptime, and savings.
- RTK (Rust Token Killer) instructions injected into .clinerules for token-efficient tool calls.
- Multi-provider support: route through Anthropic, OpenAI, DeepSeek, OpenRouter, or Ollama.

---

## v0.1.x (2026-07-04)

Initial release and infrastructure.

### v0.1.2

- Daemon mode: `dedroom init` starts background proxy with auto-restart supervisor.
- PID lock file management, log rotation.

### v0.1.1

- Config file support (`dedroom.yaml`).
- Compression pipeline: SmartCrusher (JSON), CodeCompressor (AST-aware), LogCompressor (dedup), TextCompressor.

### v0.1.0

- Initial release.
- Core proxy server with loop detection and context compression.
- `dedroom proxy` standalone mode.
- Basic CLI with init, status, stop, doctor commands.
- OpenAI and Anthropic API format support.
