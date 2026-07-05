# DedrooM

**Loop detection + context compression for AI coding agents.**

[![PyPI version](https://img.shields.io/pypi/v/dedroom.svg)](https://pypi.org/project/dedroom/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

DedrooM sits between your AI agent and the LLM provider to:

- **Detect and block infinite loops** — saves wasted API calls when tools repeat
- **Compress context** — reduces token usage by 60–95% without changing behavior
- **Intelligence Engine** — parses thoughts locally, injects proactive mentor coaching, tracks trust scores, and learns from failures
- **Redact sensitive data** — strip API keys, tokens, and secrets from tool outputs
- **Track ROI** — attribution engine shows exactly how much each tool saves

---

## Quick Start

```bash
pip install dedroom
```

**Note:** The CLI commands (`wrap`, `proxy`, `doctor`) require the Rust binary.
Install it from source or use a pre-built release:

```bash
cargo install dedroom-cli
# or build from repo: cargo build -p dedroom-cli -p dedroom-proxy
```

---

## Commands

### Wrap any AI agent through the proxy

```bash
dedroom wrap claude          # Claude Code (Anthropic)
dedroom wrap codex           # OpenAI Codex CLI
dedroom wrap aider           # Aider
dedroom wrap cursor          # Cursor Editor
dedroom wrap cline           # Cline (VS Code extension)
dedroom wrap opencode        # OpenCode
```

### Use any LLM provider (not just OpenAI/Anthropic)

```bash
# OpenCode Zen free models
dedroom wrap opencode \
  --upstream-url https://opencode.ai/zen \
  --api-key "sk-your-key" \
  -- run -m dedroom/deepseek-v4-flash-free "your task"

# DeepSeek API
dedroom wrap claude \
  --upstream-url https://api.deepseek.com \
  --api-key "sk-your-key"

# Local Ollama
dedroom wrap aider \
  --upstream-url http://localhost:11434/v1
```

### Diagnostics & control

```bash
dedroom doctor                # Run health checks
dedroom doctor --json         # JSON output for scripting
dedroom proxy                 # Start standalone proxy
dedroom unwrap <agent>        # Restore config to pre-wrap state
dedroom dash                  # Launch TUI dashboard
```

---

## Python API

```python
from dedroom import DedrooM

pipeline = DedrooM("""
loop_detection:
  max_repeats: 3
""")

# Check for loops (0 = Allow)
verdict = pipeline.verify("write_file", '{"path": "/tmp/x.txt"}')

# Full pipeline
result = pipeline.process_tool("write_file", '{}', tool_result)
print(f"Blocked: {result['is_blocked']}")
print(f"Saved {result['original_tokens'] - result['compressed_tokens']} tokens")
```

---

## Benchmarks

| Payload | Raw Tokens | With DedrooM | Reduction |
|---------|:----------:|:------------:|:---------:|
| Repeated directory listing (1MB) | 483,672 | 177,245 | **63.4%** |
| Large source file | 18,331 | 14,167 | **22.7%** |
| Build log | 284 | 284 | **0%** (no redundancy) |

- **Loop detection latency:** ~1.3ms per tool call (negligible vs 2-10s LLM roundtrip)
- **Pipeline throughput:** 5.4µs (in-memory) / 260µs (SQLite)

---

## Development

```bash
git clone https://github.com/Devaretanmay/dedroom
cd dedroom

# Build Rust binaries
cargo build -p dedroom-cli -p dedroom-proxy

# Install Python package in dev mode
pip install -e .

# Run tests
pytest python/tests/
```
