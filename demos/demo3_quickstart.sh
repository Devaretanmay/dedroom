#!/usr/bin/env bash
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

clear
echo ""
echo "  DedrooM Quick Start Demo"
echo "  ========================="
echo ""
sleep 1

echo "  Step 1: Install (one pip command)"
echo "  --------------------------------"
echo ""
sleep 1
echo "  \$ pip install dedroom"
echo ""
echo "  That's it. No config files, no setup script."
echo ""
sleep 2

echo "  Step 2: Start + route (eval init)"
echo "  ---------------------------------"
echo ""
sleep 1
echo "  \$ eval \"\$(dedroom init)\""
echo ""
echo "  Starts the proxy daemon in the background"
echo "  and sets the env vars your agent needs."
echo "  Add the exports to ~/.zshrc for permanence."
echo ""
sleep 2

echo "  Step 3: Use your agent normally"
echo "  -------------------------------"
echo ""
sleep 1
echo "  \$ claude              # Claude Code"
echo "  \$ codex               # OpenAI Codex"
echo "  \$ aider               # Aider"
echo "  \$ cursor              # Cursor (GUI)"
echo ""
echo "  Everything routes through the proxy automatically."
echo "  Loop protection, compression, PII redaction —"
echo "  all invisible to the agent and to you."
echo ""
sleep 2

echo "  Step 4: Check what's happening"
echo "  ------------------------------"
echo ""
sleep 1
echo "  \$ dedroom status   # PID, uptime, tokens saved, healing stats"
echo "  \$ dedroom report   # Per-tool savings, top tools, waste"
echo "  \$ dedroom doctor   # Full diagnostics (11 checks)"
echo ""
sleep 2

echo "  Step 5: Stop"
echo "  ------------"
echo ""
sleep 1
echo "  \$ dedroom stop"
echo ""
echo "  Stops the daemon. No cleanup needed."
echo ""
sleep 2

echo "  That's the full workflow:"
echo "    pip install dedroom"
echo "    eval \"\$(dedroom init)\""
echo "    # ... use your agent ..."
echo "    dedroom status"
echo "    dedroom stop"
echo ""
echo "  All in one terminal. No new workflow to learn."
echo ""
sleep 1
