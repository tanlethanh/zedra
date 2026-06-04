#!/bin/bash
# Forward localhost:$PORT on this Mac to the connected Android device's
# 127.0.0.1:$PORT, where the in-app GPUI devtool HTTP server listens when
# built with `--devtool`.
#
# Usage:
#   ./scripts/devtool-bridge.sh             # default port 9777
#   ./scripts/devtool-bridge.sh 9778        # custom port
#
# After bridging, query with:
#   curl -s localhost:9777/ping
#   curl -s localhost:9777/elements | jq

set -euo pipefail

PORT="${1:-9777}"

if [ -n "${ANDROID_HOME:-}" ] && [ -x "$ANDROID_HOME/platform-tools/adb" ]; then
    ADB="$ANDROID_HOME/platform-tools/adb"
elif command -v adb >/dev/null 2>&1; then
    ADB="$(command -v adb)"
else
    echo "Error: adb not found. Install Android Platform Tools or set ANDROID_HOME." >&2
    exit 1
fi

"$ADB" forward "tcp:$PORT" "tcp:$PORT" >/dev/null
echo "==> Forwarded localhost:$PORT -> device 127.0.0.1:$PORT"

if curl -fsS --max-time 1 "http://localhost:$PORT/ping" >/dev/null 2>&1; then
    echo "==> Devtool responds on localhost:$PORT"
else
    echo "Warning: /ping did not respond. Ensure the app was built with --devtool and is running." >&2
fi
