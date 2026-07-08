#!/usr/bin/env bash
set -euo pipefail

# Demo 3: Quick Start Workflow
# Shows the complete init → status → report → stop cycle.

cd "$(dirname "$0")/.."

echo "=== Building CLI ==="
cargo build -p dedroom-cli -p dedroom-proxy 2>&1

echo
echo "=== 1. Show CLI help ==="
echo "(Press any key to continue)"
read -n1
./target/debug/dedroom

echo
echo "=== 2. Start proxy daemon ==="
echo "(Press any key to continue)"
read -n1
eval "$(./target/debug/dedroom init --port 9999)"

echo
echo "=== 3. Check status ==="
echo "(Press any key to continue)"
read -n1
./target/debug/dedroom status --port 9999

echo
echo "=== 4. Generate some activity ==="
echo "(Sending test requests through proxy...)"
echo "(Press any key to continue)"
read -n1
# Use the self-healing example to exercise the pipeline
cargo run --example demo_self_healing 2>&1

echo
echo "=== 5. Run doctor check ==="
echo "(Press any key to continue)"
read -n1
./target/debug/dedroom doctor --port 9999

echo
echo "=== 6. Show report ==="
echo "(Press any key to continue)"
read -n1
./target/debug/dedroom report --port 9999

echo
echo "=== 7. Stop daemon ==="
echo "(Press any key to continue)"
read -n1
./target/debug/dedroom stop --port 9999
echo
echo "=== Demo 3 complete ==="
