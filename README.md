# DedrooM

**Loop detection + context compression for AI coding agents — save 60–95% on tokens and never get stuck in a tool loop again.**

[![Crates.io](https://img.shields.io/crates/v/dedroom-core.svg)](https://crates.io/crates/dedroom-core)
[![PyPI version](https://img.shields.io/pypi/v/dedroom.svg)](https://pypi.org/project/dedroom/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Devaretanmay/dedroom/blob/main/LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.85%2B-lightgrey)](rust-toolchain.toml)

DedrooM is a **unified proxy layer** that sits between your AI agent and any LLM provider. It intercepts every tool call to detect loops before they waste tokens, compress productive context, and redact sensitive data — all in real-time with negligible overhead (~1.3ms per call).

---

## Quick Start

### 1. Install

```bash
# Installs both the Python library and the full CLI (wrap, proxy, doctor, dash)
pip install dedroom
```

### 2. Wrap your agent

```bash
# Claude Code
dedroom wrap claude

# OpenAI Codex CLI
dedroom wrap codex

# OpenCode with free models
dedroom wrap opencode \
  --upstream-url https://opencode.ai/zen \
  --api-key "sk-your-key" \
  -- run -m dedroom/deepseek-v4-flash-free "your task"
```

### 3. Press Ctrl+C to stop

That's it. The proxy starts, routes all API calls through the pipeline, and stops when you're done.

---

## Supported Agents

| Agent | Command | How It Works |
|-------|---------|-------------|
| **Claude Code** | `dedroom wrap claude` | Sets `ANTHROPIC_BASE_URL` → proxy |
| **OpenAI Codex** | `dedroom wrap codex` | Injects DedrooM provider into `~/.codex/config.toml` |
| **Aider** | `dedroom wrap aider` | Sets `OPENAI_API_BASE` + `ANTHROPIC_BASE_URL` |
| **Cursor** | `dedroom wrap cursor` | Injects proxy URLs into `~/.cursor/settings.json` |
| **Cline** | `dedroom wrap cline` | Injects RTK instructions into `.clinerules` + VS Code settings |
| **OpenCode** | `dedroom wrap opencode` | Injects DedrooM provider into `~/.config/opencode/opencode.json` |

### Use any LLM provider

DedrooM is **provider-agnostic**. Point it at any OpenAI-compatible API:

```bash
# OpenCode Zen (free models included)
dedroom wrap opencode \
  --upstream-url https://opencode.ai/zen \
  --api-key "sk-your-key" \
  -- run -m dedroom/deepseek-v4-flash-free "your task"

# DeepSeek
dedroom wrap claude \
  --upstream-url https://api.deepseek.com \
  --api-key "sk-your-key"

# OpenRouter
dedroom wrap aider \
  --upstream-url https://openrouter.ai/api/v1 \
  --api-key "sk-your-key"

# Local Ollama (no API key needed)
dedroom wrap codex \
  --upstream-url http://localhost:11434/v1
```

### Free models (OpenCode Zen)

When wrapping OpenCode with `--upstream-url https://opencode.ai/zen`, DedrooM automatically injects these free models into your OpenCode config:

| Model ID | Name | Context |
|----------|------|:-------:|
| `deepseek-v4-flash-free` | DeepSeek V4 Flash **Free** | 32K |
| `mimo-v2.5-free` | MiMo V2.5 **Free** | 32K |
| `north-mini-code-free` | North Mini Code **Free** | 32K |
| `nemotron-3-ultra-free` | Nemotron 3 Ultra **Free** | 32K |
| `big-pickle-free` | Big Pickle **Free** | 32K |

Plus the standard premium models (Claude Opus 4.6, Sonnet 4.6, GPT-4o).

---

## Commands

### `dedroom wrap <agent>` — Start proxy + launch agent

```bash
dedroom wrap claude                   # Default port 8080
dedroom wrap codex --port 9999        # Custom port
dedroom wrap aider -- --model sonnet  # Pass args to agent
dedroom wrap cursor                   # GUI setup (prints instructions)
dedroom wrap cline                    # Injects .clinerules + VS Code settings
dedroom wrap opencode -- run -m ...   # Non-interactive mode
```

### `dedroom unwrap <agent>` — Restore config to pre-wrap state

```bash
dedroom unwrap codex     # Restores ~/.codex/config.toml from backup
dedroom unwrap opencode  # Removes DedrooM provider from opencode.json
dedroom unwrap claude    # Runtime-only — no persistent state
```

### `dedroom doctor` — Run diagnostics

```bash
dedroom doctor                      # 11 health checks
dedroom doctor --port 9999          # Check a different port
dedroom doctor --json               # Machine-readable JSON output
```

Checks proxy liveness, agent routing configs, shell environment variables, and token savings.

### `dedroom proxy` — Standalone proxy server

```bash
dedroom proxy                         # Port 8080, default config
dedroom proxy --port 9999             # Custom port
dedroom proxy --config my-config.yaml # Custom config
```

### `dedroom dash` — TUI dashboard

```bash
dedroom dash                          # Auto-detect proxy on port 8080
dedroom dash --port 9090              # Custom dashboard port
dedroom dash http://10.0.0.5:9090     # Remote proxy URL
```

---

## Benchmark: With vs Without DedrooM

### Token Savings

| Payload | Raw Tokens | With DedrooM | **Reduction** |
|---------|:----------:|:------------:|:-------------:|
| Repeated directory listing (1MB) | 483,672 | 177,245 | **63.4%** |
| Large source file (80KB) | 18,331 | 14,167 | **22.7%** |
| Build log (no redundancy) | 284 | 284 | **0%** |

### Cost Savings (estimated)

| Scenario | Without DedrooM | With DedrooM | **Savings** |
|----------|:---------------:|:------------:|:-----------:|
| 100 tool calls (10 loops) | 500K tokens | 180K tokens | **~$2.40 saved** (Claude Sonnet) |
| Daily usage (50 sessions) | 25M tokens | 9M tokens | **~$120/mo saved**¹ |

> ¹ Based on Claude Sonnet pricing ($3/M input tokens), 50 sessions/day, ~64% average compression on repetitive payloads. Actual savings vary with usage patterns and model choice.

### Latency Overhead

| Metric | Without DedrooM | With DedrooM | **Overhead** |
|--------|:---------------:|:------------:|:------------:|
| Per tool call | 0 ms | **1.315 ms** | Negligible vs 2-10s LLM roundtrip |
| Pipeline (in-memory) | — | 5.4 µs | Instant |
| Pipeline (SQLite) | — | 260 µs | Instant |

### Loop Detection Accuracy

- **Identical repeats blocked:** 100% correct
- **Varied commands not blocked:** 0% false positives
- **Adaptive error thresholds:** Tightens on errors, loosens on recovery

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   Your Agent                     │
│  (Claude Code, Codex, Aider, Cursor, OpenCode)  │
└─────────────────────┬───────────────────────────┘
                      │ HTTP / SSE
                      ▼
┌─────────────────────────────────────────────────┐
│              DedrooM Proxy (axum)                │
│                                                 │
│  ┌─────────┐  ┌──────────┐  ┌────────────────┐ │
│  │Redaction│─▶│  Loop    │─▶│  Compression   │ │
│  │(PII)    │  │Detection │  │  (60-95%)      │ │
│  └─────────┘  └──────────┘  └────────────────┘ │
│                      │                          │
│  ┌───────────────────────────────────────────┐  │
│  │          Savings Ledger + Events          │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────┬───────────────────────────┘
                      │ Forward (OpenAI-compat)
                      ▼
┌─────────────────────────────────────────────────┐
│          LLM Provider (your choice)              │
│  Anthropic │ OpenAI │ DeepSeek │ OpenCode Zen   │
│  Ollama │ OpenRouter │ or any OpenAI-compat API │
└─────────────────────────────────────────────────┘
```

### Internal Pipeline

```
Receive Request → Extract Tools → Trust Check → Redact PII → Loop Detect → Compress → Judgment & Learning → Forward → Record Telemetry
```

- **Trust Verification:** Dynamically drops an agent's `max_repeats` limit to `1` if their trust score tanks from too many failures.
- **Redaction:** 14 regex patterns + entropy detection for API keys, tokens, secrets
- **Loop Detection:** Sliding window, adaptive thresholds, error-aware tightening — 460ns median
- **Compression:** SmartCrusher (JSON), CodeCompressor (AST-aware), LogCompressor (dedup), TextCompressor
- **Judgment Preservation:** Parses the LLM's raw output for `<thinking>` tags and reflection phrases to track cognitive complexity. Dynamically toggles Quality Score.
- **Cross-Session Learning:** Saves exact tool failure signatures and dynamically injects "Wisdom from past sessions" as proactive hints right when the agent repeats a known mistake.
- **Mentor Mode:** Proactively coaches the agent when they start "tilting" and enforces end-of-session reflection.
- **Telemetry:** NDJSON event log with tilt_index, compression ratios, trust scores, and per-tool savings

---

## Python API

```python
from dedroom import DedrooM, detect_loop, compress_text

# Create a pipeline
pipeline = DedrooM("""
loop_detection:
  max_repeats: 3
  adaptive:
    enabled: true
    error_reduction: 1
compression:
  compressors:
    smart_crusher: true
    code_compressor: true
""")

# Check for loops
verdict = pipeline.verify("write_file", '{"path": "/tmp/x.txt"}')
# 0 = Allow, 1 = Warn, 2 = BlockRetry, 3 = BlockHalt

# Full pipeline processing
result = pipeline.process_tool("write_file", '{}', tool_result)
print(f"Blocked: {result['is_blocked']}")
print(f"Compression: {result['original_tokens']} → {result['compressed_tokens']} tokens")

# Standalone functions
verdict = detect_loop("write_file", '{}', max_repeats=3)  # 0-3
compressed = compress_text(tool_output, content_type="code")
```

---

## Configuration

```yaml
# dedroom.yaml
loop_detection:
  max_repeats: 3
  strictness: balanced        # lenient | balanced | strict
  history_backend: memory     # memory or sqlite
  adaptive:
    enabled: true
    error_reduction: 1

compression:
  compressors:
    smart_crusher: true
    code_compressor: true
  ccr:
    backend: memory           # memory or sqlite
    ttl_seconds: 1800

redaction:
  enabled: true
  patterns:
    - "(?i)sk-[a-zA-Z0-9]{20,}"  # OpenAI-style keys
    - "(?i)AKIA[0-9A-Z]{16}"      # AWS access keys
```

---

## Backends

| Backend | Use Case | Persistence |
|---------|----------|:-----------:|
| **In-memory** | Default, fastest | No |
| **SQLite** | Persistent, cross-restart | Yes |

SQLite features WAL mode, batch pruning, and adaptive threshold persistence.

---

## Development

```bash
# Prerequisites
rustup toolchain install stable
pip install maturin

# Clone and build
git clone https://github.com/Devaretanmay/dedroom
cd dedroom

# Build all Rust binaries
cargo build -p dedroom-cli -p dedroom-proxy -p dedroom-tui

# Build Python wheel
maturin build --release -m crates/dedroom-py/Cargo.toml

# Run tests
cargo test -p dedroom-core
cargo test -p dedroom-proxy
pytest python/tests/

# Run benchmarks
cargo bench --features sqlite
```

### Project Structure

```
dedroom/
├── crates/
│   ├── dedroom-core/      # Core engine: loop detection + compression + redaction
│   ├── dedroom-proxy/     # axum reverse proxy (intercepts + forwards)
│   ├── dedroom-cli/       # CLI: wrap, unwrap, doctor, proxy, dash
│   ├── dedroom-py/        # PyO3 Python bindings
│   ├── dedroom-tui/       # Terminal UI dashboard
│   └── dedroom-parity/    # Fixture-based parity tests
├── python/
│   ├── dedroom/           # Python package source
│   └── tests/             # Python tests
└── pyproject.toml         # Python packaging config
```

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be licensed as above, without any additional terms or conditions.
