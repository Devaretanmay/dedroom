#!/usr/bin/env bash
set -euo pipefail

# Record all 3 demo GIFs using asciinema + agg
# Requires: asciinema, agg (cargo install agg)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CAST_DIR="$SCRIPT_DIR/casts"
GIF_DIR="$SCRIPT_DIR"
IDLE_WAIT=0.5  # seconds between commands

export TERM=xterm-256color
export COLUMNS=90
export LINES=30

mkdir -p "$CAST_DIR"

record_demo() {
    local name="$1"
    local cast_file="$CAST_DIR/$name.cast"
    local gif_file="$GIF_DIR/$name.gif"
    local script_file="$SCRIPT_DIR/${name}.sh"

    echo "=== Recording $name ==="
    
    # Record the demo script
    asciinema rec --stdin --overwrite \
        --cols 90 --rows 30 \
        --env "TERM=COLUMNS=LINES" \
        "$cast_file" \
        -c "bash '$script_file'" 2>&1

    # Convert to GIF
    echo "=== Converting $name.cast -> $name.gif ==="
    agg --speed 1.5 --font-family "SFMono Nerd Font,MonoLisa,monospace" \
        --font-size 14 --line-height 1.3 \
        --rows 30 --cols 90 \
        "$cast_file" "$gif_file" 2>&1

    echo "=== Done: $gif_file ($(stat -f%z "$gif_file" 2>/dev/null || stat -c%s "$gif_file" 2>/dev/null) bytes)"
    echo
}

cd "$REPO_DIR"

# Build everything first
echo "=== Building all binaries ==="
cargo build -p dedroom-cli -p dedroom-proxy 2>&1
cargo build --example demo_self_healing 2>&1
cargo build --example compression_quality 2>&1
cargo build --example bench 2>&1

record_demo "demo1_self_healing"
record_demo "demo2_savings"
record_demo "demo3_quickstart"

echo "=== All demos recorded ==="
ls -lh "$GIF_DIR"/*.gif
