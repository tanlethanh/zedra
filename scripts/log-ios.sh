#!/bin/bash
# log-ios.sh — Stream logs from a USB-connected iOS device or running simulator.
#
# Usage:
#   ./scripts/log-ios.sh [--filter <pattern>] [--select-device] [--simulator]
#
# Options:
#   --filter <pattern>   Additional regex pattern on top of default Zedra filter
#   --with-native        Include native logs
#   --select-device      Ignore saved device preference and re-prompt
#   --simulator          Stream logs from a booted simulator instead of a physical device
#
# Device preference is saved per Claude Code session in /tmp/zedra-ios-device-$PPID.
# Run with --select-device to switch devices within the same session.
#
# Physical device requires: libimobiledevice (brew install libimobiledevice) + USB-paired device.
# Simulator requires: Xcode command-line tools (xcrun).

set -euo pipefail

FILTER=""
WITH_NATIVE=false
SELECT_DEVICE=false
SIMULATOR=false
PREF_FILE="/tmp/zedra-ios-device-$PPID"

find_tool() {
    local name="$1"

    if command -v "$name" >/dev/null 2>&1; then
        command -v "$name"
    elif [[ -x "/opt/homebrew/bin/$name" ]]; then
        echo "/opt/homebrew/bin/$name"
    elif [[ -x "/usr/local/bin/$name" ]]; then
        echo "/usr/local/bin/$name"
    else
        return 1
    fi
}

write_saved_device_pref() {
    printf '%s|%s|%s\n' "$1" "$2" "$3" > "$PREF_FILE"
}

read_saved_device_pref() {
    local first=""
    local second=""
    local third=""

    IFS='|' read -r first second third < "$PREF_FILE" || return 1

    if [[ "$first" == "dev" || "$first" == "sim" ]]; then
        DEVICE_TYPE="$first"
        DEVICE_UDID="${second:-}"
        DEVICE_NAME="${third:-}"
    elif [[ -n "$first" && -n "$second" ]]; then
        # Older run-ios.sh builds saved physical devices as "udid|name".
        DEVICE_TYPE="dev"
        DEVICE_UDID="$first"
        DEVICE_NAME="$second"
        write_saved_device_pref "$DEVICE_TYPE" "$DEVICE_UDID" "$DEVICE_NAME"
    else
        return 1
    fi

    [[ -n "$DEVICE_UDID" ]]
}

visible_physical_device_lines() {
    local idevice_id="$1"
    local udids=""
    local xctrace_lines=""
    local udid=""
    local line=""

    udids=$("$idevice_id" -l 2>/dev/null || true)
    [[ -n "$udids" ]] || return 1

    xctrace_lines=$(xcrun xctrace list devices 2>&1 || true)
    while IFS= read -r udid; do
        [[ -n "$udid" ]] || continue

        line=$(printf '%s\n' "$xctrace_lines" | grep -F "($udid)" | head -1 || true)
        if [[ -n "$line" ]]; then
            printf '%s\n' "$line"
        else
            printf 'iOS Device (unknown) (%s)\n' "$udid"
        fi
    done <<< "$udids"
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)
            FILTER="$2"; shift 2 ;;
        --with-native)
            WITH_NATIVE=true; shift ;;
        --select-device)
            SELECT_DEVICE=true; shift ;;
        --simulator)
            SIMULATOR=true; shift ;;
        -h|--help)
            sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1 ;;
    esac
done

IDEVICE_ID=$(find_tool idevice_id || true)
IDEVICESYSLOG=$(find_tool idevicesyslog || true)

# ---------------------------------------------------------------------------
# Device selection
# ---------------------------------------------------------------------------
DEVICE_UDID=""
DEVICE_NAME=""
DEVICE_TYPE=""  # "sim" or "dev"

if [[ "$SELECT_DEVICE" == false ]] && [[ -f "$PREF_FILE" ]]; then
    if read_saved_device_pref; then
        # If --simulator flag contradicts saved type, re-select
        if [[ "$SIMULATOR" == true && "$DEVICE_TYPE" != "sim" ]] || \
           [[ "$SIMULATOR" == false && "$DEVICE_TYPE" == "sim" ]]; then
            DEVICE_UDID=""
            DEVICE_NAME=""
            DEVICE_TYPE=""
        else
            echo "==> Using saved device: $DEVICE_NAME ($DEVICE_UDID)"
        fi
    fi
fi

if [[ -z "$DEVICE_UDID" ]]; then
    if [[ "$SIMULATOR" == true ]]; then
        # -----------------------------------------------------------------------
        # Simulator selection
        # -----------------------------------------------------------------------
        echo "==> Enumerating booted simulators..."
        # Format: "    iPhone 16 Pro (UDID) (Booted)"
        SIM_LINES=$(xcrun simctl list devices booted 2>/dev/null \
            | grep -E '\(Booted\)' \
            | sed 's/^[[:space:]]*//')

        if [[ -z "$SIM_LINES" ]]; then
            echo "Error: No booted simulators found. Launch a simulator first (Xcode → Simulator)." >&2
            exit 1
        fi

        echo ""
        echo "Booted simulators:"
        i=1
        while IFS= read -r line; do
            echo "  $i. $line"
            i=$((i + 1))
        done <<< "$SIM_LINES"
        echo ""

        COUNT=$(echo "$SIM_LINES" | wc -l | tr -d ' ')
        if [[ "$COUNT" -eq 1 ]]; then
            CHOICE=1
            echo "==> Auto-selecting only simulator."
        else
            read -rp "Select simulator [1-$COUNT]: " CHOICE
        fi

        SELECTED_LINE=$(echo "$SIM_LINES" | sed -n "${CHOICE}p")
        if [[ -z "$SELECTED_LINE" ]]; then
            echo "Error: Invalid selection." >&2
            exit 1
        fi

        # Extract UDID — UUID in parentheses
        DEVICE_UDID=$(echo "$SELECTED_LINE" | grep -oE '[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}' | head -1)
        DEVICE_NAME=$(echo "$SELECTED_LINE" | sed 's/ ([^ ]*) (Booted)$//' | sed 's/ (Booted)$//')
        DEVICE_TYPE="sim"

        if [[ -z "$DEVICE_UDID" ]]; then
            echo "Error: Could not parse UDID from: $SELECTED_LINE" >&2
            exit 1
        fi

        write_saved_device_pref "sim" "$DEVICE_UDID" "$DEVICE_NAME"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    else
        # -----------------------------------------------------------------------
        # Physical device selection
        # -----------------------------------------------------------------------
        if [[ -z "$IDEVICE_ID" ]]; then
            echo "Error: idevice_id not found. Install with: brew install libimobiledevice" >&2
            exit 1
        fi

        echo "==> Enumerating connected devices..."
        DEVICE_LINES=$(visible_physical_device_lines "$IDEVICE_ID" || true)

        if [[ -z "$DEVICE_LINES" ]]; then
            echo "Error: No USB-visible iOS devices found. Connect and trust a device, or use --simulator." >&2
            exit 1
        fi

        echo ""
        echo "Connected iOS devices:"
        i=1
        while IFS= read -r line; do
            echo "  $i. $line"
            i=$((i + 1))
        done <<< "$DEVICE_LINES"
        echo ""

        COUNT=$(echo "$DEVICE_LINES" | wc -l | tr -d ' ')
        if [[ "$COUNT" -eq 1 ]]; then
            CHOICE=1
            echo "==> Auto-selecting only device."
        else
            read -rp "Select device [1-$COUNT]: " CHOICE
        fi

        SELECTED_LINE=$(echo "$DEVICE_LINES" | sed -n "${CHOICE}p")
        if [[ -z "$SELECTED_LINE" ]]; then
            echo "Error: Invalid selection." >&2
            exit 1
        fi

        # Extract UDID — last parenthesised token on the line
        DEVICE_UDID=$(echo "$SELECTED_LINE" | grep -oE '\([A-F0-9a-f-]{25,}\)' | tail -1 | tr -d '()')
        DEVICE_NAME=$(echo "$SELECTED_LINE" | sed 's/ ([^)]*) ([^)]*)$//' | sed 's/ ([^)]*)$//')
        DEVICE_TYPE="dev"

        if [[ -z "$DEVICE_UDID" ]]; then
            echo "Error: Could not parse UDID from: $SELECTED_LINE" >&2
            exit 1
        fi

        write_saved_device_pref "dev" "$DEVICE_UDID" "$DEVICE_NAME"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    fi
fi

# ---------------------------------------------------------------------------
# Stream logs
# ---------------------------------------------------------------------------
GREP_PATTERN='\[I |\[W |\[E |\[D | INFO | WARN | ERROR | DEBUG | TRACE |panic|PANIC|crash|CRASH|NSException|Terminating'
if [[ -n "$FILTER" ]]; then
    GREP_PATTERN="$GREP_PATTERN|$FILTER"
fi

if [[ "$WITH_NATIVE" == true ]]; then
    # Native logs: include level prefixes
    echo "==> Including native logs"
    GREP_PATTERN="$GREP_PATTERN|<I|<W|<E|<D"
fi

echo "==> Streaming logs from $DEVICE_NAME (Ctrl-C to stop)..."
echo "    Level prefix: [I]=Info [W]=Warn [E]=Error [D]=Debug [T]=Trace"
echo ""

if [[ "$DEVICE_TYPE" == "sim" ]]; then
    # Simulator: use xcrun simctl spawn + log stream with OSLog predicate
    PREDICATE='process == "Zedra"'
    xcrun simctl spawn "$DEVICE_UDID" log stream \
        --predicate "$PREDICATE" \
        --level debug \
        --color none \
        2>/dev/null \
        | grep -E "$GREP_PATTERN" --line-buffered --color=auto
else
    # Physical device: use idevicesyslog (USB, no sudo required)
    if [[ -z "$IDEVICESYSLOG" ]]; then
        echo "Error: idevicesyslog not found." >&2
        echo "Install with: brew install libimobiledevice" >&2
        exit 1
    fi
    if [[ -z "$IDEVICE_ID" ]]; then
        echo "Error: idevice_id not found. Install with: brew install libimobiledevice" >&2
        exit 1
    fi

    # Verify device is visible to libimobiledevice (requires USB pairing)
    if ! "$IDEVICE_ID" -l 2>/dev/null | grep -F -q "$DEVICE_UDID"; then
        echo "Error: Device $DEVICE_UDID not visible to idevicesyslog." >&2
        echo "idevicesyslog requires a USB-connected (paired) device." >&2
        exit 1
    fi

    # Run idevicesyslog under a pseudo-terminal. When stdout is a pipe,
    # idevicesyslog can block-buffer and make filtered logs appear stuck.
    python3 - "$IDEVICESYSLOG" "$DEVICE_UDID" "$GREP_PATTERN" <<'PY'
import os
import pty
import re
import select
import signal
import sys

tool, udid, pattern = sys.argv[1:4]
regex = re.compile(pattern)

def stop(_signum, _frame):
    raise KeyboardInterrupt

signal.signal(signal.SIGINT, stop)
signal.signal(signal.SIGTERM, stop)

pid, fd = pty.fork()
if pid == 0:
    os.execv(tool, [tool, "--no-colors", "-u", udid, "-p", "Zedra"])

buffer = b""
try:
    while True:
        readable, _, _ = select.select([fd], [], [])
        if fd not in readable:
            continue
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        buffer += chunk
        while b"\n" in buffer:
            line, buffer = buffer.split(b"\n", 1)
            text = line.decode(errors="replace").rstrip("\r")
            if regex.search(text):
                print(text, flush=True)
except KeyboardInterrupt:
    pass
finally:
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
PY
fi
