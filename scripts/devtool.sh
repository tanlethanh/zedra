#!/bin/bash
# Thin wrapper around the in-app GPUI devtool HTTP server.
#
# The server lives on the device at 127.0.0.1:9777 when the app is built with
# `--devtool` (debug only). This script forwards the port via adb and exposes
# four subcommands so any caller — human, Claude, Codex, CI — can drive the
# UI without writing curl ceremony.
#
# Usage:
#   scripts/devtool.sh bridge                  # adb forward + /ping
#   scripts/devtool.sh ping                    # just /ping
#   scripts/devtool.sh list                    # leaf-only element table
#   scripts/devtool.sh elements                # raw JSON
#   scripts/devtool.sh tap <element_id>        # tap by leaf or full path
#   scripts/devtool.sh tap-xy <x> <y>          # tap by logical-pixel coords
#
# Env:
#   ZEDRA_DEVTOOL_PORT   defaults to 9777
#
# See docs/DEVTOOL.md for the full surface and integration notes.

set -euo pipefail

PORT="${ZEDRA_DEVTOOL_PORT:-9777}"
BASE="http://localhost:$PORT"

resolve_adb() {
    if [ -n "${ANDROID_HOME:-}" ] && [ -x "$ANDROID_HOME/platform-tools/adb" ]; then
        echo "$ANDROID_HOME/platform-tools/adb"
        return
    fi
    if command -v adb >/dev/null 2>&1; then
        command -v adb
        return
    fi
    echo "Error: adb not found. Install Android Platform Tools or set ANDROID_HOME." >&2
    exit 1
}

cmd_bridge() {
    local adb
    adb="$(resolve_adb)"
    "$adb" forward "tcp:$PORT" "tcp:$PORT" >/dev/null
    echo "==> Forwarded localhost:$PORT -> device 127.0.0.1:$PORT"
    if curl -fsS --max-time 1 "$BASE/ping" >/dev/null 2>&1; then
        echo "==> Devtool responds on $BASE"
    else
        echo "Warning: /ping did not respond. App built with --devtool and running?" >&2
        exit 1
    fi
}

cmd_ping() {
    curl -fsS --max-time 1 "$BASE/ping"
    echo
}

cmd_elements() {
    curl -fsS --max-time 2 "$BASE/elements"
    echo
}

cmd_list() {
    local body
    body="$(curl -fsS --max-time 2 "$BASE/elements")"
    DEVTOOL_BODY="$body" python3 <<'PY'
import json, os
d = json.loads(os.environ["DEVTOOL_BODY"])
print(f"frame {d['frame_id']} ({len(d['entries'])} entries)")
print(f"{'leaf':<40} {'x':>7} {'y':>7} {'w':>6} {'h':>6}")
for e in d["entries"]:
    leaf = e["path"].rsplit("/", 1)[-1]
    if leaf.startswith("view#"):
        continue
    print(f"  {leaf:<38} {e['x']:>7.1f} {e['y']:>7.1f} {e['w']:>6.1f} {e['h']:>6.1f}")
PY
}

cmd_tap() {
    if [ $# -lt 1 ]; then
        echo "Usage: $0 tap <element_id>" >&2
        exit 1
    fi
    local id="$1"
    local payload
    payload=$(printf '{"element_id":"%s"}' "$id")
    curl -fsS --max-time 2 -X POST -H 'Content-Type: application/json' -d "$payload" "$BASE/tap"
    echo
}

cmd_tap_xy() {
    if [ $# -lt 2 ]; then
        echo "Usage: $0 tap-xy <x> <y>" >&2
        exit 1
    fi
    local payload
    payload=$(printf '{"x":%s,"y":%s}' "$1" "$2")
    curl -fsS --max-time 2 -X POST -H 'Content-Type: application/json' -d "$payload" "$BASE/tap_xy"
    echo
}

sub="${1:-}"
shift || true
case "$sub" in
    bridge)    cmd_bridge "$@" ;;
    ping)      cmd_ping "$@" ;;
    elements)  cmd_elements "$@" ;;
    list)      cmd_list "$@" ;;
    tap)       cmd_tap "$@" ;;
    tap-xy)    cmd_tap_xy "$@" ;;
    ""|-h|--help)
        sed -n '2,20p' "$0"
        ;;
    *)
        echo "Error: unknown subcommand '$sub'." >&2
        sed -n '2,20p' "$0" >&2
        exit 1
        ;;
esac
