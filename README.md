# DedrooM

**Loop detection and context compression for AI agents.**

[![Crates.io](https://img.shields.io/crates/v/dedroom-core.svg)](https://crates.io/crates/dedroom-core)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rustc-1.85%2B-lightgrey)](rust-toolchain.toml)

DedrooM is a unified runtime layer for AI agents that intercepts every tool call to **detect loops before they waste tokens** and **compress productive context by 60–95%**. It runs as a Rust library, a Python package, or a reverse proxy — with persistent state via optional SQLite backends.

---

## Quick start

```toml
# Cargo.toml
[dependencies]
dedroom-core = "0.1"
```

```rust
use dedroom_core::config::DedrooMConfig;
use dedroom_core::loop_detection::LoopDetector;

let config = DedrooMConfig::from_yaml_str("
    loop_detection:
      max_repeats: 3
      strictness: balanced
")?;

let mut detector = LoopDetector::new(&config.loop_detection);
let verdict = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);

match verdict {
    LoopVerdict::Allow => println!("Proceed"),
    LoopVerdict::BlockRetry => println!("Agent should retry differently"),
    _ => println!("Blocked"),
}
```

---

## Features

### Loop detection — 460ns median
- **Exact-match** counting against a sliding window of recent calls
- **Volatile field stripping** — ignore timestamps, request IDs, offsets
- **Auto-inference** — learns which fields vary across repeated calls
- **Adaptive thresholds** — tightens detection on error loops, loosens on recovery
- **Persistent history** (SQLite) — cross-restart loop detection learning

### Context compression — 60–95% token reduction
- **SmartCrusher** — JSON array optimization: greedy row selection by content coverage
- **CodeCompressor** — AST-aware: preserves structure, compresses function bodies
- **LogCompressor** — deduplication of repeated lines, preserves error lines
- **TextCompressor** — whitespace normalization (ML-powered, optional)

### Pipeline integration
Both systems share a unified pipeline:
```
Receive → Cache Align → Loop Detect → Compress → Forward → Record
```

- Loop signals feed into compression policy (looping → aggressive budget)
- CCR store shared between compression cache and loop detection memory
- Unified savings ledger tracks both compression and loop prevention

---

## Configuration

```yaml
# config.yaml
loop_detection:
  max_repeats: 3
  strictness: balanced        # lenient | balanced | strict
  history_backend: sqlite     # memory (default) or sqlite
  adaptive:
    enabled: true
    error_reduction: 1

compression:
  compressors:
    smart_crusher: true
    code_compressor: true
  ccr:
    backend: sqlite           # memory (default) or sqlite
    ttl_seconds: 1800

loop_compression_coupling:
  enabled: true
  on_error_loop:
    compression_budget: maximum
    inject_hint: true
```

---

## Architecture

```
dedroom/
├── crates/
│   ├── dedroom-core/      # Core engine: loop detection + compression
│   │   ├── src/
│   │   │   ├── loop_detection/   # History, adaptive thresholds, semantic
│   │   │   ├── compression/      # SmartCrusher, Code, Log, Text compressors
│   │   │   ├── ccr/              # Content-addressable result cache (memory + SQLite)
│   │   │   ├── pipeline.rs       # Unified Pipeline::process_tool_call()
│   │   │   └── config.rs         # YAML-based configuration
│   │   └── benches/              # Criterion benchmarks (history + pipeline)
│   ├── dedroom-proxy/    # axum reverse proxy
│   ├── dedroom-cli/      # CLI binary with benchmark and proxy controls
│   ├── dedroom-py/       # PyO3 Python bindings
│   └── dedroom-parity/   # Fixture-based parity tests
└── Cargo.toml            # Workspace root
```

---

## Performance

### Pipeline throughput (`Pipeline::process_tool_call`)

| Scenario | In-Memory | SQLite (`:memory:`) | On-Disk SQLite |
|----------|:---------:|:-------------------:|:--------------:|
| Allow (no result, loop detection only) | **5.4 µs** | **260 µs** | **1,517 µs** |
| Compress (with result + CCR write) | **9.6 µs** | **264 µs** | **1,964 µs** |
| Warm (50-entry history, idle) | **3.8 µs** | **24 µs** | **142 µs** |

All overheads are negligible vs LLM roundtrip times (2–10 seconds).

### History push throughput (w=10)

| Backend | Time | vs In-Memory |
|---------|:----:|:------------:|
| In-memory | **42 ns** | 1× |
| SQLite (batched) | **4.7 µs** | ~110× |

Benchmarks run with `cargo bench --features sqlite`.

---

## Backends

DedrooM supports two storage backends for loop history and CCR:

| Backend | Use Case | Persistence |
|---------|----------|:-----------:|
| **In-memory** | Default, fastest | ❌ |
| **SQLite** | Persistent, cross-restart | ✅ |

SQLite backends include:
- **WAL mode** for concurrent read performance
- **Batch pruning** (every N writes) to amortize cleanup cost
- **Adaptive threshold persistence** for cross-restart learning

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be licensed as above, without any additional terms or conditions.
