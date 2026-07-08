#!/usr/bin/env bash
set -euo pipefail

# Demo 2: Savings & Compression Report
# Shows dedroom status and report output.

cd "$(dirname "$0")/.."

echo "=== Building all binaries ==="
cargo build -p dedroom-cli -p dedroom-proxy 2>&1

echo
echo "=== Starting proxy for demo ==="
echo "(Will start, inject test data, show status/report, then stop)"
echo "(Press any key to continue)"
read -n1

# Start proxy in background
./target/debug/dedroom init --no-daemon --port 9999 &
PROXY_PID=$!
echo "Proxy PID: $PROXY_PID"

# Wait for startup
echo "Waiting for proxy..."
for i in $(seq 1 10); do
  if curl -sf http://127.0.0.1:9999/health >/dev/null 2>&1; then
    echo "Proxy ready."
    break
  fi
  sleep 1
done

echo
echo "=== Showing dedroom doctor ==="
read -n1
./target/debug/dedroom doctor --port 9999

echo
echo "=== Showing dedroom status ==="
read -n1
./target/debug/dedroom status --port 9999

echo
echo "=== Showing dedroom report ==="
read -n1
./target/debug/dedroom report --port 9999

echo
echo "=== Cleaning up ==="
kill $PROXY_PID 2>/dev/null || true
wait $PROXY_PID 2>/dev/null || true
echo "Proxy stopped."
echo "=== Demo 2 complete ==="
