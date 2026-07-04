# DedrooM

**Stop wasting tokens on loops. Compress everything else.**

DedrooM is a unified runtime layer for AI agents that combines loop detection and context compression in a single pipeline. It sits between your agent and the LLM, intercepting every tool call to:

1. **Detect and block loops** in under a microsecond — before the call reaches the API
2. **Compress productive context** by 60–95% — using type-aware algorithms
3. **Do both from one config file** with a shared embedding pipeline and storage backend

## Quick start

```bash
# Rust library
cargo add dedroom-core

# Python
pip install dedroom

# Proxy server
cargo run -p dedroom-proxy -- --port 8787 --config config.yaml
```

```rust
use dedroom_core::config::DedrooMConfig;
use dedroom_core::loop_detection::LoopDetector;

let config = DedrooMConfig::from_yaml_str("loop_detection:\n  max_repeats: 3")?;
let mut detector = LoopDetector::new(&config.loop_detection);
let verdict = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
// 0 = Allow, 1 = Warn, 2 = BlockRetry, 3 = BlockHalt
```

## How it works

Every tool call goes through a multi-stage pipeline:

```
Receive → Cache Align → Loop Detect → Compress → Forward → Record
```

**Loop detection** (4 stages, ~460ns):
1. Exact match against call history
2. Configured volatile field stripping (ignore timestamps, request IDs)
3. Auto-inference of volatile fields from call patterns
4. Semantic similarity via shared embedding pipeline

**Context compression** (type-aware):
- **SmartCrusher** — JSON arrays: greedy row selection by content coverage
- **CodeCompressor** — AST-aware: preserves structure, compresses bodies
- **LogCompressor** — deduplication of repeated lines, preserves errors
- **TextCompressor** — whitespace normalization, ML-powered (optional)

**Key fusion points:**
- Loop signals feed into compression policy (looping → aggressive budget)
- CCR store is shared between compression cache and loop detection memory
- Single embedding pipeline serves both semantic loop detection and vector memory
- Unified savings ledger tracks both compression and loop prevention

## Configuration

```yaml
# dedroom.yaml
loop_detection:
  max_repeats: 3
  strictness: balanced
  volatile_fields:
    auto_inference: true
    configured:
      - tool: search
        fields: [request_id, timestamp]
  semantic:
    enabled: false

compression:
  compressors:
    smart_crusher: true
    code_compressor: true

loop_compression_coupling:
  enabled: true
  on_error_loop:
    compression_budget: maximum
    inject_hint: true
```

## Architecture

```
dedroom/
├── crates/
│   ├── dedroom-core/     # Unified engine: loop detection + compression
│   ├── dedroom-proxy/    # axum reverse proxy
│   ├── dedroom-py/       # PyO3 Python bindings
│   └── dedroom-parity/   # Fixture-based parity tests
├── python/dedroom/       # Python package
├── pyproject.toml        # Maturin build config
└── Cargo.toml            # Rust workspace root
```

## License

Apache 2.0
