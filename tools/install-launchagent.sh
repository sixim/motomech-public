#!/usr/bin/env bash
# install-launchagent.sh — install motomech as a macOS LaunchAgent
#
# Usage:
#   bash tools/install-launchagent.sh [--port 8765]
#
# What it does:
#   1. Fills in MOTOMECH_ROOT in the plist template
#   2. Copies to ~/Library/LaunchAgents/
#   3. Loads with launchctl
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${1:-8765}"
PLIST_SRC="$REPO_ROOT/knowledge/motomech.serve.plist.template"
PLIST_LABEL="com.motomech.serve"
PLIST_DEST="$HOME/Library/LaunchAgents/${PLIST_LABEL}.plist"
BIN="$REPO_ROOT/bin/motomech"

if [ ! -f "$BIN" ]; then
  echo "❌  Binary not found at $BIN"
  echo "   Run: cd motomech && cargo build --release && cp target/release/motomech ../bin/"
  exit 1
fi

mkdir -p "$HOME/Library/LaunchAgents"
mkdir -p "$REPO_ROOT/logs"

sed \
  -e "s|MOTOMECH_ROOT|$REPO_ROOT|g" \
  -e "s|8765|$PORT|g" \
  "$PLIST_SRC" > "$PLIST_DEST"

# Unload existing instance if present
launchctl unload "$PLIST_DEST" 2>/dev/null || true
launchctl load -w "$PLIST_DEST"

echo "✅  Installed and started: $PLIST_LABEL"
echo "   Listening on 127.0.0.1:$PORT"
echo "   Logs: $REPO_ROOT/logs/serve.out.log"
echo ""
echo "   To expose on your Tailnet:"
echo "     tailscale serve --bg $PORT"
