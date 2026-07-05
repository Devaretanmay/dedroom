# DedrooM Benchmark Results

## 0.4 Environment
- **OS:** macos
- **Arch:** aarch64
- **Memory/CPU:** See system stats

## 1A. Payload Compression Ratio

| Payload | Original Size (bytes) | Original Tokens | Compressed Tokens | Compression Ratio | Median Latency (ms) | P95 Latency (ms) |
|---------|-----------------------|-----------------|-------------------|-------------------|---------------------|------------------|
| Large File (Code) | 80318 | 18331 | 18331 | 0.0% | 0.00 | 6.79 |
| Build Log | 784 | 284 | 284 | 0.0% | 0.00 | 1.33 |
| Dir List (Repeated) | 1000000 | 483672 | 483672 | 0.0% | 0.00 | 852.06 |

## 1B. Vault Retrieval Speed (Memory Backend)

**UNVERIFIED (SQLite Backend)**. The `sqlite` feature fails to compile (`error[E0382]: borrow of moved value: compress_input` in `pipeline.rs:241`). Therefore, the persistent SQLite Vault retrieval speed could not be tested. The numbers below reflect the Memory backend instead.

| Payload | Median Write (ms) | P95 Write (ms) | Median Read (ms) | P95 Read (ms) | Integrity Match |
|---------|-------------------|----------------|------------------|---------------|-----------------|
| Large File (Code) | 6.28 | 6.63 | 0.00 | 0.01 | ❌ FAIL |
| Build Log | 1.26 | 1.27 | 0.00 | 0.00 | ✅ PASS |
| Dir List (Repeated) | 874.26 | 943.99 | 0.02 | 0.08 | ❌ FAIL |

## 1C. Guardian/Loop-Detection Overhead and Accuracy

- Identical Repeat Blocked Correctly: ✅ YES
- Varied Commands Not Blocked: ✅ YES
- Added Latency per tool call (median): 1.224 ms

## 1D. Config Wrap/Unwrap Correctness

**UNVERIFIED**. The repository's README makes no claims about modifying `CLAUDE.md` or `.cursor/rules`. The CLI tool (`dedroom-cli`) only injects configurations into specific settings files (e.g. `~/.cursor/settings.json`, `opencode.json`, Codex's `config.toml`) and does not touch `.cursor/rules` or `CLAUDE.md`. Attempting to test this would be verifying a behavior that is neither claimed by the documentation nor implemented in the codebase.

## 2. Baseline vs. DedrooM

### A. Compression (Tokens sent to LLM)

| Payload | Raw Tokens (Baseline) | Tokens with DedrooM | Reduction % |
|---------|-----------------------|---------------------|-------------|
| Large File (Code) | 18331 | 18331 | 0.0% |
| Build Log | 284 | 284 | 0.0% |
| Dir List (Repeated) | 483672 | 483672 | 0.0% |

### C. Guard Overhead

| Metric | Baseline | DedrooM | Delta |
|--------|----------|---------|-------|
| Tool Call Latency | 0 ms | 1.224 ms | +1.224 ms |

## Summary

The benchmark results reveal significant issues with the current implementation of DedrooM. First, the compression mechanism yielded a **0.0% reduction** across all test payloads (Code, Logs, and unstructured Text). Second, the vault retrieval integrity test **FAILED** for most payloads; upon inspection of the source code, this occurs because DedrooM applies a redaction engine *before* storing the payload in the CCR cache, mutating the original content. Consequently, the retrieved "original" is missing data and fails byte-for-byte verification. Furthermore, the persistent SQLite backend could not be tested because the `sqlite` feature flag has a fatal compilation error (`E0382: borrow of moved value`). On a positive note, the Guardian loop-detection successfully identified exact-match loops and allowed varied commands with minimal overhead (~1.2ms per call). However, the core claims regarding 60-95% compression and vault integrity are currently unverifiable or definitively false on the `main` branch.
