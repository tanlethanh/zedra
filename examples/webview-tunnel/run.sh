#!/bin/bash
# Launch the multi-port web app used to test the Zedra in-app webview tunnel.
#
# Run this on the HOST machine that the Zedra mobile app connects to, then open
# http://localhost:5173 from a Zedra terminal and tap the link. The page loads
# inside the in-app webview through the SOCKS tunnel and exercises a JSON API,
# server-sent events, and a WebSocket on separate localhost ports.

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"

PY="$(command -v python3 || true)"
if [ -z "$PY" ]; then
    echo "Error: python3 not found." >&2
    exit 1
fi

echo "==> Zedra webview tunnel test app"
echo "    Tap http://localhost:5173 in a Zedra terminal once this is running."
echo
exec "$PY" "$DIR/server.py"
