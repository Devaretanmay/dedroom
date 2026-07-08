#!/usr/bin/env bash
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR/crates/dedroom-core"

clear
echo ""
echo "  DedrooM Self-Healing Demo"
echo "  ========================="
echo ""
sleep 1

echo "  This demo shows what happens when an AI agent"
echo "  keeps making the same failing tool call."
echo ""
echo "  The agent calls write_file() with the same args"
echo "  and gets 'permission denied' each time."
echo ""
sleep 2

cargo run --example demo_self_healing 2>/dev/null

echo ""
echo ""
sleep 1
echo "  Key takeaways:"
echo "  - Call #3 was BLOCKED (wasted tokens avoided)"
echo "  - Call #4 got a healing hint injected into context"
echo "  - Call #5 with different args was ALLOWED (no false positive)"
echo "  - Compression saved 70-74% on tool output"
echo ""
sleep 2
echo "  Demo complete."
