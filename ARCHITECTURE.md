
# Dedroom Architecture Overview

## Project Structure

```
dedroom/
├── crates/                    # Rust workspace
│   ├── dedroom-cli/          # Command-line interface
│   ├── dedroom-core/         # Core library: loop detection and context compression
│   ├── dedroom-proxy/        # Axum HTTP reverse proxy
│   ├── dedroom-py/           # Python bindings (PyO3)
│   └── dedroom-parity/       # Fixture-based parity tests
├── python/                   # Python package
│   ├── dedroom/              # Python module
│   └── tests/                # Python test suite
├── Cargo.toml                # Rust workspace config
├── Cargo.lock
├── config.yaml               # Default Dedroom config
├── deny.toml                 # Cargo deny config
├── pyproject.toml            # Python package config
└── README.md                 # Project README
```

## Core Architecture Layers

### 1. Dedroom Core
- `loop_detection/`
  - `engine.rs`: Main loop decision engine
  - `history.rs`: Call history management
  - `canonical.rs`: Argument canonicalization (remove volatile fields)
  - `adaptive.rs`: Adaptive loop thresholds based on errors
  - `semantic.rs`: Semantic similarity detection (embedding-based)
- `compression/`
  - `smart_crusher.rs`: Greedy JSON compression
  - `code_compressor.rs`: Tree-sitter based code compression
  - `log_compressor.rs`: Log line deduplication
  - `text_compressor.rs`: Simple text normalization
  - `router.rs`: Content-type detection and routing
  - `policy.rs`: Compression policy management
- `ccr/`: Compress-Cache-Retrieve (CCR) store
- `embedding/`: Embedding generation utilities
- `telemetry/`: Savings tracking
- `pipeline.rs`: Full pipeline orchestration
- `config.rs`: Configuration management

## 2. Proxy Layer
- `handlers/`: HTTP request handlers
- `intercept.rs`: Request/response interception and processing
- `proxy.rs`: Proxy routing
- `main.rs`: Server entry point

## 3. CLI Layer
- `main.rs`: CLI entry point, supports:
  - `wrap <agent>`: Wraps AI coding agents
  - `unwrap <agent>`: Restores original agent state
  - `doctor`: Runs diagnostics
  - `proxy`: Runs standalone proxy server
