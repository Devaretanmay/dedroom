#!/usr/bin/env bash
set -euo pipefail

# Demo 1: Self-Healing in Action
# Records the self-healing demo example output.

cd "$(dirname "$0")/.."

echo "=== Building self-healing demo example ==="
cargo build --example demo_self_healing 2>&1

echo
echo "=== Running self-healing demo ==="
echo "(Press any key to continue after each section)"
read -n1

cargo run --example demo_self_healing 2>&1

echo
echo "=== Demo 1 complete ==="
