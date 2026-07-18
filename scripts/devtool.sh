#!/bin/bash
# Thin wrapper around the in-app GPUI devtool HTTP server.
#
# The server lives on the app at 127.0.0.1:9777 when built with `--devtool`
# (debug only). This script bridges the port (adb forward on Android, iproxy
# over usbmux on iOS device, nothing needed on iOS simulator) and exposes
# subcommands so any caller — human, Claude, Codex, CI — can drive the UI
# without writing curl ceremony.
#
# Usage:
#   scripts/devtool.sh bridge                  # Android: adb forward + /ping
#   scripts/devtool.sh bridge-ios              # iOS: iproxy (device) or /ping (sim),
#                                               # reads the global device pref
#                                               # written by run-ios.sh/ios-log.sh tail
#   scripts/devtool.sh ping                    # just /ping
#   scripts/devtool.sh list                    # leaf-only element table
#   scripts/devtool.sh elements                # raw JSON
#   scripts/devtool.sh press <element_id>      # fires on_press, by leaf or
#                                               # full path, works on any
#                                               # .id(...)'d element without
#                                               # declaring anything
#   scripts/devtool.sh long-press <element_id> # same, fires on_long_press
#   scripts/devtool.sh tap-xy <x> <y>          # raw screen touch by
#                                               # logical-pixel coords, no
#                                               # element resolution
#   scripts/devtool.sh call <name> ['<params>'] # run a debug function app
#                                               # code registered via
#                                               # App::register_devtool_action
#                                               # (not tied to any element)
#   scripts/devtool.sh sequence '<json-steps>' # run press/long_press/tap_xy/
#                                               # call/wait_for_element steps
#                                               # in order, one round trip,
#                                               # stops at first step that
#                                               # can't resolve
#
# The port is a single fixed number (below) — nothing stops two builds (e.g.
# a simulator bound directly here, and a physical device bridged via iproxy)
# from both listening at once, which silently sends callers to whichever one
# the OS resolves "localhost" to. `bridge-ios` guards against this: it drops
# its own stale tunnel when the target changes, and refuses to bridge a
# device on top of a port something else already occupies. Every response
# also carries a `pid` — check it if two calls ever look like they're
# talking to different app instances.
#
# Every endpoint except /ping requires a token this script reads and sends
# automatically (X-Devtool-Token header) — nothing to configure.
#
# Env:
#   ZEDRA_DEVTOOL_PORT   defaults to 9777
#
# See docs/DEVTOOL.md for the full surface and integration notes.

set -euo pipefail

PORT="${ZEDRA_DEVTOOL_PORT:-9777}"
BASE="http://localhost:$PORT"
TOKEN_FILE="/tmp/gpui-devtool-token-$PORT"
IOS_PREF_FILE="/tmp/zedra-ios-device"
IOS_IPROXY_PIDFILE="/tmp/zedra-ios-devtool-iproxy.pid"
IOS_IPROXY_TARGET_FILE="/tmp/zedra-ios-devtool-iproxy.target"
IOS_IPROXY_LOG="/tmp/zedra-ios-devtool-iproxy.log"
# Which platform the last `bridge`/`bridge-ios` targeted — tells read_token
# where the app's token can actually be found (logcat vs host file/capture).
BRIDGE_TARGET_FILE="/tmp/zedra-devtool-bridge-target-$PORT"

# Every endpoint except /ping requires this. Direct file read works for the
# simulator (a real macOS process sharing the host filesystem); a physical
# device's /tmp is inside its own sandbox, not host-visible, so fall back to
# grepping the log daemon's capture file for the same line the app logs at
# startup (`devtool: token: ...`) — requires ios-log.sh daemon already
# running, which Phase 0 of /debug-ios already sequences before any bridge.
# $TOKEN_FILE can only ever have been written by a *simulator* process (a
# device can't reach host /tmp at all) — so when the saved device pref says
# the current target is a physical device, a file sitting at that path is
# always stale leftover from a past simulator run, never the device's real
# token; skip straight to the log fallback rather than trusting it blindly
# (this bit us: a simulator run's token file silently won over a device's
# real one, producing a persistent 401 with no clue why).
read_token() {
    # Android: the app can't write host /tmp and no ios-log daemon exists —
    # the token only surfaces in logcat, so pull it from there.
    if [ "$(cat "$BRIDGE_TARGET_FILE" 2>/dev/null)" = "android" ]; then
        local adb
        adb="$(resolve_adb 2>/dev/null)" || { echo ""; return; }
        "$adb" logcat -d 2>/dev/null \
            | grep -o 'devtool: token: [0-9a-f]*' | tail -1 | awk '{print $NF}' || true
        return
    fi
    local target_is_device=0
    if [ -f "$IOS_PREF_FILE" ]; then
        local pref_type
        IFS='|' read -r pref_type _ _ < "$IOS_PREF_FILE" || true
        [ "$pref_type" = "dev" ] && target_is_device=1
    fi
    if [ "$target_is_device" -eq 0 ] && [ -f "$TOKEN_FILE" ]; then
        cat "$TOKEN_FILE"
        return
    fi
    local capture="/tmp/zedra-ios-log-daemon/capture.log"
    if [ -f "$capture" ]; then
        grep -o 'devtool: token: [0-9a-f]*' "$capture" 2>/dev/null | tail -1 | awk '{print $NF}'
        return
    fi
    echo ""
}

curl_auth() {
    local token
    token="$(read_token)"
    if [ -n "$token" ]; then
        curl "$@" -H "X-Devtool-Token: $token"
    else
        curl "$@"
    fi
}

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

# Returns the pid field from a live /ping, or empty if unreachable/unparseable.
ping_pid() {
    curl -fsS --max-time 1 "$BASE/ping" 2>/dev/null \
        | grep -o '"pid":[0-9]*' | head -1 | cut -d: -f2 || true
}

check_ping() {
    local pid
    pid="$(ping_pid)"
    if [ -n "$pid" ]; then
        echo "==> Devtool responds on $BASE (pid=$pid)"
    else
        echo "Warning: /ping did not respond. App built with --devtool and running?" >&2
        exit 1
    fi
}

stop_tracked_iproxy() {
    if [ -f "$IOS_IPROXY_PIDFILE" ] && kill -0 "$(cat "$IOS_IPROXY_PIDFILE")" 2>/dev/null; then
        kill "$(cat "$IOS_IPROXY_PIDFILE")" 2>/dev/null || true
    fi
    rm -f "$IOS_IPROXY_PIDFILE" "$IOS_IPROXY_TARGET_FILE"
}

cmd_bridge() {
    local adb
    adb="$(resolve_adb)"
    echo "android" > "$BRIDGE_TARGET_FILE"
    "$adb" forward "tcp:$PORT" "tcp:$PORT" >/dev/null
    echo "==> Forwarded localhost:$PORT -> device 127.0.0.1:$PORT"
    check_ping
}

cmd_bridge_ios() {
    echo "ios" > "$BRIDGE_TARGET_FILE"
    local device_type="" device_id="" device_name=""
    if [ -f "$IOS_PREF_FILE" ]; then
        IFS='|' read -r device_type device_id device_name < "$IOS_PREF_FILE" || true
    fi

    if [ -z "$device_type" ]; then
        # No saved preference at all — enumerate connected physical devices
        # rather than hard-failing when one is plainly connected.
        local devices
        devices="$(idevice_id -l 2>/dev/null || true)"
        local device_count=0
        [ -n "$devices" ] && device_count="$(echo "$devices" | wc -l | tr -d ' ')"
        if [ "$device_count" -eq 1 ]; then
            device_type="dev"
            device_id="$devices"
            echo "==> No saved device preference; using the one connected physical device ($device_id)." >&2
        elif [ "$device_count" -gt 1 ]; then
            echo "Error: no saved iOS device preference ($IOS_PREF_FILE) and multiple physical devices connected — can't guess which one." >&2
            echo "Run ./scripts/run-ios.sh device --select-device --devtool first to save a preference." >&2
            exit 1
        elif xcrun simctl list devices booted 2>/dev/null | grep -q "Booted"; then
            echo "==> No saved device preference, but a simulator is booted — simulator shares localhost, no bridge needed." >&2
            device_type="sim"
        else
            echo "Error: no saved iOS device preference ($IOS_PREF_FILE), no connected physical device, no booted simulator." >&2
            echo "Run ./scripts/run-ios.sh sim|device --devtool first." >&2
            exit 1
        fi
    fi

    if [ "$device_type" = "sim" ]; then
        # Simulator shares the host's loopback — no tunnel needed. If a
        # physical-device tunnel from a previous target is still running,
        # drop it: it and the simulator's own direct binding would both
        # listen on the same port, and whichever one "localhost" resolves
        # to wins silently.
        if [ -f "$IOS_IPROXY_PIDFILE" ] && kill -0 "$(cat "$IOS_IPROXY_PIDFILE")" 2>/dev/null; then
            echo "==> Stopping stale device tunnel (current target is simulator, not $(cat "$IOS_IPROXY_TARGET_FILE" 2>/dev/null))"
            stop_tracked_iproxy
            sleep 0.3
        fi
        echo "==> Simulator target ($device_name) shares localhost — no bridge needed."
        check_ping
        return
    fi

    if [ "$device_type" != "dev" ] || [ -z "$device_id" ]; then
        echo "Error: could not parse device preference from $IOS_PREF_FILE." >&2
        exit 1
    fi

    if ! command -v iproxy >/dev/null 2>&1; then
        echo "Error: iproxy not found. Install with: brew install libimobiledevice" >&2
        exit 1
    fi

    if [ -f "$IOS_IPROXY_PIDFILE" ] && kill -0 "$(cat "$IOS_IPROXY_PIDFILE")" 2>/dev/null \
        && [ "$(cat "$IOS_IPROXY_TARGET_FILE" 2>/dev/null)" = "$device_id" ]; then
        echo "==> iproxy already running for $device_id (pid $(cat "$IOS_IPROXY_PIDFILE"))"
    else
        # Drop our own tunnel if it's bridging a different (stale) target —
        # safe, we started it. Then check the port isn't ALREADY occupied
        # by something we don't own (e.g. a simulator bound directly) before
        # laying a second listener on top of it.
        if [ -f "$IOS_IPROXY_PIDFILE" ] && kill -0 "$(cat "$IOS_IPROXY_PIDFILE")" 2>/dev/null; then
            echo "==> Stopping iproxy for previous target ($(cat "$IOS_IPROXY_TARGET_FILE" 2>/dev/null))"
            stop_tracked_iproxy
            sleep 0.3
        fi
        local existing_pid
        existing_pid="$(ping_pid)"
        if [ -n "$existing_pid" ]; then
            echo "Error: $BASE already has a live devtool responding (pid=$existing_pid) that isn't our own tunnel — likely a simulator built with --devtool, or a foreign process." >&2
            echo "Bridging device $device_id on top would create two listeners on the same port with unpredictable results (whichever one \"localhost\" resolves to wins silently)." >&2
            echo "Stop whatever's using the port first, or set ZEDRA_DEVTOOL_PORT to a different port for this device." >&2
            exit 1
        fi
        iproxy -u "$device_id" "$PORT:$PORT" >"$IOS_IPROXY_LOG" 2>&1 &
        echo $! > "$IOS_IPROXY_PIDFILE"
        echo "$device_id" > "$IOS_IPROXY_TARGET_FILE"
        disown
        sleep 0.5
        echo "==> iproxy forwarding localhost:$PORT -> $device_name ($device_id) 127.0.0.1:$PORT (pid $!)"
    fi
    check_ping
}

cmd_ping() {
    curl -fsS --max-time 1 "$BASE/ping"
    echo
}

cmd_elements() {
    curl_auth -fsS --max-time 2 "$BASE/elements"
    echo
}

cmd_list() {
    local body
    body="$(curl_auth -fsS --max-time 2 "$BASE/elements")"
    # No general JSON parser here — the response is our own serde_json
    # output on one line with a fixed, known field order per entry (path,
    # instance, x, y, w, h, ...; see entry_json() in gpui_devtool.rs), not
    # arbitrary/adversarial input, so extracting each field's values in
    # document order and zipping them positionally is safe and avoids a
    # dependency on jq/python for a debug-only table.
    local frame_id pid
    frame_id="$(grep -o '"frame_id":[0-9]*' <<< "$body" | head -1 | cut -d: -f2)"
    pid="$(grep -o '"pid":[0-9]*' <<< "$body" | head -1 | cut -d: -f2)"
    local paths xs ys ws hs count
    paths="$(grep -o '"path":"[^"]*"' <<< "$body" | sed 's/^"path":"//; s/"$//')"
    xs="$(grep -o '"x":-\?[0-9.]*' <<< "$body" | cut -d: -f2)"
    ys="$(grep -o '"y":-\?[0-9.]*' <<< "$body" | cut -d: -f2)"
    ws="$(grep -o '"w":-\?[0-9.]*' <<< "$body" | cut -d: -f2)"
    hs="$(grep -o '"h":-\?[0-9.]*' <<< "$body" | cut -d: -f2)"
    count="$([ -z "$paths" ] && echo 0 || wc -l <<< "$paths" | tr -d ' ')"

    echo "frame ${frame_id:-?} pid=${pid:-?} ($count entries)"
    printf '%-40s %7s %7s %6s %6s\n' "leaf" "x" "y" "w" "h"
    [ "$count" -eq 0 ] && return
    paste -d'|' <(echo "$paths") <(echo "$xs") <(echo "$ys") <(echo "$ws") <(echo "$hs") \
        | while IFS='|' read -r path x y w h; do
            local leaf="${path##*/}"
            [[ "$leaf" == view\#* ]] && continue
            printf '  %-38s %7.1f %7.1f %6.1f %6.1f\n' "$leaf" "$x" "$y" "$w" "$h"
        done
}

# POST a JSON payload and always print the response body, success or error —
# unlike `curl -f`, which swallows the body on non-2xx responses (the exact
# case where the body matters most: {"ok":false,"error":"..."}). Returns
# non-zero on non-2xx so callers under `set -e` still fail loudly.
post_json() {
    local endpoint="$1" payload="$2"
    local response status body
    response="$(curl_auth -sS --max-time 2 -w '\n%{http_code}' -X POST -H 'Content-Type: application/json' -d "$payload" "$BASE$endpoint")"
    status="${response##*$'\n'}"
    body="${response%$'\n'*}"
    echo "$body"
    case "$status" in
        2??) return 0 ;;
        *) return 1 ;;
    esac
}

cmd_press() {
    if [ $# -lt 1 ]; then
        echo "Usage: $0 press <element_id>" >&2
        exit 1
    fi
    local payload
    payload=$(printf '{"element_id":"%s"}' "$1")
    post_json "/press" "$payload"
}

cmd_long_press() {
    if [ $# -lt 1 ]; then
        echo "Usage: $0 long-press <element_id>" >&2
        exit 1
    fi
    local payload
    payload=$(printf '{"element_id":"%s"}' "$1")
    post_json "/long_press" "$payload"
}

cmd_tap_xy() {
    if [ $# -lt 2 ]; then
        echo "Usage: $0 tap-xy <x> <y>" >&2
        exit 1
    fi
    local payload
    payload=$(printf '{"x":%s,"y":%s}' "$1" "$2")
    post_json "/tap_xy" "$payload"
}

cmd_call() {
    if [ $# -lt 1 ]; then
        echo "Usage: $0 call <name> ['<json-params>']" >&2
        exit 1
    fi
    local payload
    if [ $# -ge 2 ]; then
        payload=$(printf '{"name":"%s","params":%s}' "$1" "$2")
    else
        payload=$(printf '{"name":"%s"}' "$1")
    fi
    post_json "/call" "$payload"
}

cmd_sequence() {
    if [ $# -lt 1 ]; then
        echo "Usage: $0 sequence '<json-steps-array>'  (or @file.json to read from a file)" >&2
        echo 'Steps: {"type":"press|long_press","element_id":"..."}' >&2
        echo '       {"type":"tap_xy","x":N,"y":N}' >&2
        echo '       {"type":"call","name":"...","params":{...}}' >&2
        echo '       {"type":"wait_for_element","element_id":"...","timeout_ms":N}' >&2
        exit 1
    fi
    local steps="$1"
    if [[ "$steps" == @* ]]; then
        steps="$(cat "${steps#@}")"
    fi
    local payload
    payload=$(printf '{"steps":%s}' "$steps")
    post_json "/sequence" "$payload"
}

sub="${1:-}"
shift || true
case "$sub" in
    bridge)     cmd_bridge "$@" ;;
    bridge-ios) cmd_bridge_ios "$@" ;;
    ping)      cmd_ping "$@" ;;
    elements)  cmd_elements "$@" ;;
    list)      cmd_list "$@" ;;
    press)     cmd_press "$@" ;;
    long-press) cmd_long_press "$@" ;;
    tap-xy)    cmd_tap_xy "$@" ;;
    call)      cmd_call "$@" ;;
    sequence)  cmd_sequence "$@" ;;
    ""|-h|--help)
        sed -n '2,51p' "$0"
        ;;
    *)
        echo "Error: unknown subcommand '$sub'." >&2
        sed -n '2,51p' "$0" >&2
        exit 1
        ;;
esac
