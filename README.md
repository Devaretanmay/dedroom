# DedrooM

**Cuts your AI coding agent's token bill without changing how you work.**

[![PyPI version](https://img.shields.io/pypi/v/dedroom.svg)](https://pypi.org/project/dedroom/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/Devaretanmay/dedroom/blob/main/LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.85%2B-lightgrey)](rust-toolchain.toml)

## Why

Two things quietly inflate an agent session's token spend: the agent retrying a failing command in a near-identical loop, and tool output (file listings, logs, diffs) piling into context that the model has effectively already seen. Neither shows up as a single big line item they show up as your bill being higher than the work should have cost.

DedrooM catches both automatically. One command wraps your existing agent (`dedroom wrap claude`), and from then on repeated failing calls get cut off before they compound, and redundant tool output gets compressed before it reaches the model — no changes to how you invoke your agent, no new workflow to learn.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Requirements](#requirements)
- [Supported Agents](#supported-agents)
- [Commands](#commands)
- [Performance](#performance)
- [Architecture](#architecture)
- [Python API](#python-api)
- [Configuration](#configuration)
- [Backends](#backends)
- [Security & Privacy](#security--privacy)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

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

### 3. Stop with Ctrl+C

That's it the proxy starts, routes all API traffic through the pipeline, and shuts down cleanly when you're done.

---

## Requirements

- Python 3.9+ (for the `pip install dedroom` package and CLI)
- Rust 1.85+ — only needed if you're building from source (see [Development](#development))
- macOS, Linux, or Windows (WSL recommended on Windows)

---

## Supported Agents

| Agent | Command | How It Works |
|---|---|---|
| **Claude Code** | `dedroom wrap claude` | Sets `ANTHROPIC_BASE_URL` to the proxy |
| **OpenAI Codex** | `dedroom wrap codex` | Injects the DedrooM provider into `~/.codex/config.toml` |
| **Aider** | `dedroom wrap aider` | Sets `OPENAI_API_BASE` and `ANTHROPIC_BASE_URL` |
| **Cursor** | `dedroom wrap cursor` | Injects proxy URLs into `~/.cursor/settings.json` |
| **Cline** | `dedroom wrap cline` | Injects rules into `.clinerules` and VS Code settings |
| **OpenCode** | `dedroom wrap opencode` | Injects the DedrooM provider into `~/.config/opencode/opencode.json` |

### Bring your own provider

DedrooM is **provider-agnostic** —> point it at any OpenAI-compatible API:

```bash
# OpenCode Zen (includes free models)
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

# Local Ollama (no API key required)
dedroom wrap codex \
  --upstream-url http://localhost:11434/v1
```

### Free models via OpenCode Zen

When you wrap OpenCode with `--upstream-url https://opencode.ai/zen`, DedrooM automatically registers OpenCode Zen's current free-tier models in your OpenCode config, alongside standard premium models (Claude Opus 4.6, Sonnet 4.6, GPT-4o).

> The specific free models and their names/limits are set by the OpenCode Zen provider, not DedrooM, and change over time check [opencode.ai/zen](https://opencode.ai/zen) for the current list before depending on a specific one.

---

## Commands

### `dedroom wrap <agent>` —> Start the proxy and launch an agent

```bash
dedroom wrap claude                   # Default port 8080
dedroom wrap codex --port 9999        # Custom port
dedroom wrap aider -- --model sonnet  # Pass args through to the agent
dedroom wrap cursor                   # Prints GUI setup instructions
dedroom wrap cline                    # Injects .clinerules + VS Code settings
dedroom wrap opencode -- run -m ...   # Non-interactive mode
```

### `dedroom unwrap <agent>` —> Restore prior configuration

```bash
dedroom unwrap codex     # Restores ~/.codex/config.toml from backup
dedroom unwrap opencode  # Removes the DedrooM provider from opencode.json
dedroom unwrap claude    # Runtime-only — nothing persisted to restore
```

### `dedroom doctor` —> Run diagnostics

```bash
dedroom doctor                      # 11 health checks
dedroom doctor --port 9999          # Check a specific port
dedroom doctor --json               # Machine-readable output
```

Verifies proxy liveness, agent routing configuration, shell environment variables, and token savings.

### `dedroom proxy` Run the proxy standalone

```bash
dedroom proxy                         # Port 8080, default config
dedroom proxy --port 9999             # Custom port
dedroom proxy --config my-config.yaml # Custom config file
```

### `dedroom dash` Terminal dashboard

```bash
dedroom dash                          # Auto-detects proxy on port 8080
dedroom dash --port 9090              # Custom dashboard port
dedroom dash http://10.0.0.5:9090     # Point at a remote proxy
```

---

## Performance

> **A note on these numbers:** the figures below come from a small set of internal test scenarios, not a large-scale or third-party benchmark suite. Savings depend heavily on workload — a session with lots of loop-prone retries or large repetitive tool output will see far more benefit than one that doesn't. Treat these as illustrative, reproduce them on your own workload (`cargo bench --features sqlite`) before using them for capacity planning, and don't take the "60–95%" range in the tagline as a guaranteed outcome — the measured scenarios below span roughly 0–64%.

### Token usage

| Workload | Native Tokens | DedrooM Tokens | Reduction |
|---|---:|---:|---|
| Iterative debugging (10 loops) | 500,000 | 180,000 | `████████████░░░░░░░░` ~64% |
| Large monorepo scanning | 18,331 | 14,167 | `████░░░░░░░░░░░░░░░░` ~22% |
| Dense compilation logs | 284 | 284 | `░░░░░░░░░░░░░░░░░░░░` 0% (lossless fallback) |

The compression ratio is workload-dependent by design: DedrooM only compresses what's genuinely redundant, and falls back to passing content through unchanged (the "lossless fallback" case above) when it isn't.

### Example: payload compression

DedrooM strips redundant metadata and truncates long, repetitive arrays before they reach the upstream LLM, preserving meaning while shrinking payload size.

<table width="100%">
<tr>
<th width="50%">Native payload</th>
<th width="50%">DedrooM payload</th>
</tr>
<tr>
<td>

```json
{
  "role": "tool",
  "name": "list_files",
  "content": "file_a.txt\nfile_b.txt\nfile_c.txt\nfile_d.txt\nfile_e.txt\nfile_f.txt\nfile_g.txt\nfile_h.txt\nfile_i.txt\nfile_j.txt\nfile_k.txt\nfile_l.txt\nfile_m.txt\nfile_n.txt\nfile_o.txt\nfile_p.txt"
}
```

</td>
<td>

```json
{
  "role": "tool",
  "name": "list_files",
  "content": "file_a.txt\nfile_b.txt\nfile_c.txt\n... [10 items truncated for context preservation] ...\nfile_n.txt\nfile_o.txt\nfile_p.txt"
}
```

</td>
</tr>
</table>

### Latency overhead

The interception pipeline is designed to add negligible latency relative to a single LLM round-trip, which typically runs in the low seconds. Median overhead in our microbenchmarks:

| Operation | Median | Target SLA |
|---|:---:|:---|
| End-to-end intercept | ~1.3 ms | < 2 ms |
| In-memory pipeline (Rust core) | single-digit µs | < 10 µs |
| Persistent SQLite logging | ~0.3 ms | < 500 µs |

Run `cargo bench --features sqlite` to reproduce these on your own hardware numbers will vary by CPU and payload shape.

### Loop detection

The loop detector uses an adaptive sliding-window algorithm: repeated identical (or near-identical) tool calls get flagged and eventually blocked, while varied, exploratory tool use is left alone. Thresholds tighten automatically as an agent's error rate rises. As with the token-savings numbers above, exact precision/false-positive rates depend on `strictness` and `max_repeats` settings — tune them for your workload rather than relying on defaults for adversarial or unusual tool-call patterns.

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   Your Agent                    │
│  (Claude Code, Codex, Aider, Cursor, OpenCode)  │
└─────────────────────┬───────────────────────────┘
                      │ HTTP / SSE
                      ▼
┌─────────────────────────────────────────────────┐
│              DedrooM Proxy (axum)               │
│                                                 │
│  ┌─────────┐  ┌──────────┐  ┌────────────────┐  │
│  │Redaction│─▶│  Loop    │─▶│  Compression   │  │
│  │(PII)    │  │Detection │  │  (60–95%)      │  │
│  └─────────┘  └──────────┘  └────────────────┘  │
│                       │                         │
│  ┌───────────────────────────────────────────┐  │
│  │          Savings Ledger + Events          │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────┬───────────────────────────┘
                      │ Forward (OpenAI-compatible)
                      ▼
┌─────────────────────────────────────────────────┐
│              LLM Provider (your choice)         │
│  Anthropic · OpenAI · DeepSeek · OpenCode Zen   │
│ Ollama · OpenRouter · any OpenAI-compatible API │
└─────────────────────────────────────────────────┘
```

### Internal pipeline

```
Receive Request → Extract Tools → Trust Check → Redact PII → Loop Detect → Compress → Judgment & Learning → Forward → Record Telemetry
```

- **Trust verification** — lowers an agent's `max_repeats` limit to `1` when its trust score drops due to repeated failures.
- **Redaction** — 14 regex patterns plus entropy detection for API keys, tokens, and secrets.
- **Loop detection** — sliding window with adaptive, error-aware thresholds.
- **Compression** — SmartCrusher (JSON), CodeCompressor (AST-aware), LogCompressor (dedup), and TextCompressor.
- **Judgment preservation** — a heuristic that counts `<thinking>` tags and reflection-style phrasing in model output as a rough, best-effort proxy for how much deliberate reasoning is happening in a turn. This is a signal, not a validated quality metric — don't treat the resulting score as ground truth.
- **Cross-session learning** — stores tool-call failure signatures (tool name + args + error) and injects a short hint into context if the same signature reappears in a later session.
- **Mentor mode** — when the loop detector's error-rate signal crosses a threshold, injects a prompt nudging the agent to reconsider its approach, and adds a short end-of-session summary prompt.
- **Telemetry** — NDJSON event log capturing tilt index, compression ratios, trust scores, and per-tool savings.

---

## Python API

Integrate DedrooM directly into a LangChain pipeline or a custom Python agent. See the [Security Audit Agent example](examples/security_audit_agent.py) for a full, production-style integration.

![DedrooM Security Audit Agent Demo](audit_demo.gif)

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
|---|---|:---:|
| In-memory | Default, fastest | No |
| SQLite | Persistent, survives restarts | Yes |

The SQLite backend supports WAL mode, batch pruning, and persisted adaptive thresholds.

---

## Security & Privacy

DedrooM sees every tool call and its arguments before they leave your machine — that includes file contents, command output, and anything else your agent passes through. A few things worth knowing before you route production or client code through it:

- **Redaction runs locally**, before the request is forwarded upstream. It relies on 14 regex patterns plus entropy-based detection for high-entropy strings (likely keys/tokens). It is pattern-based, not a guarantee — review the patterns in your `dedroom.yaml` and add your own for anything specific to your environment.
- **Telemetry (NDJSON event log) is written locally** by default. Check your `dedroom.yaml` / backend config before assuming nothing is persisted, especially if you enable the SQLite backend.
- DedrooM forwards traffic to whichever upstream you configure (`--upstream-url`) — it does not send data anywhere else on its own.
- Treat this like any other proxy in your request path: review the source (it's Apache-2.0 and open) rather than taking redaction coverage on faith, especially for regulated or sensitive codebases.

## Troubleshooting

- **Agent isn't routing through the proxy** — run `dedroom doctor` first; it checks proxy liveness and whether the target agent's config actually points at DedrooM.
- **Config wasn't restored after `unwrap`** — confirm a backup exists (`unwrap` restores from one); if the original wrap was interrupted mid-way, the backup may be missing and you'll need to reset the agent's config manually.
- **Port conflicts** — pass `--port` to `wrap`, `proxy`, or `dash` to use a non-default port.
- **Unexpected compression on structured logs** — set the relevant compressor to `false` in `dedroom.yaml` (`smart_crusher`, `code_compressor`, etc.) if a particular workload needs untouched output.
- Still stuck? Open an issue with `dedroom doctor --json` output attached.

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

# Build the Python wheel
maturin build --release -m crates/dedroom-py/Cargo.toml

# Run tests
cargo test -p dedroom-core
cargo test -p dedroom-proxy
pytest python/tests/

# Run benchmarks
cargo bench --features sqlite
```
---

## Contributing

Issues and PRs are welcome. Before opening a PR:

1. Run `cargo test -p dedroom-core -p dedroom-proxy` and `pytest python/tests/` — both should pass.
2. If you're changing loop-detection or compression behavior, add a fixture-based test under `dedroom-parity` so regressions get caught automatically.
3. Keep PRs scoped to one change easier to review, easier to bisect if something breaks.

For anything nontrivial, opening an issue first to discuss approach will save you a rewrite.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be licensed as above, with no additional terms or conditions.
