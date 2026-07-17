#!/usr/bin/env bash
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR/crates/dedroom-core"

clear
echo ""
echo "  DedrooM Compression & Savings Demo"
echo "  ==================================="
echo ""
sleep 1

echo "  Benchmark 1: Pipeline Latency"
echo "  -----------------------------"
echo "  (How fast is the pipeline?)"
echo ""
sleep 1
cargo run --example bench 2>/dev/null | grep -E "Pipeline:|block on|first call|error loop|savings" | head -6
echo ""
sleep 2

echo "  Benchmark 2: Loop Detection Speed"
echo "  ---------------------------------"
echo "  (How fast does it detect loops?)"
echo ""
sleep 1
cargo run --example bench 2>/dev/null | grep -E "Cold start|Warm \(|Block |Simple args|History fill" | head -5
echo ""
sleep 2

echo "  Benchmark 3: Compression Quality"
echo "  --------------------------------"
echo "  (How much tokens get saved?)"
echo ""
sleep 1
cargo run --example bench 2>/dev/null | grep -A 8 "^──────────────────────────────────────────────────────$" | tail -9
echo ""
sleep 2

echo "  Benchmark 4: Tree-Sitter AST Quality"
echo "  ------------------------------------"
echo "  (Per-language compression ratios)"
echo ""
sleep 1
cargo run --example compression_quality 2>/dev/null | grep -E "^\s|-- Done|═══" | grep -v "═══.*═══"
echo ""
sleep 2

echo ""
echo "  Summary:"
echo "  --------"
echo "  • Numbers above are measured live from the Rust examples on this machine."
echo "  • Actual savings depend on your workload; re-run 'cargo run --example bench'."
echo ""
sleep 3
echo "  Demo complete."
