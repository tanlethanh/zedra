#!/bin/bash
# ios-log.sh — Stream logs from a USB-connected iOS device via idevicesyslog.
#
# Usage:
#   ./scripts/ios-log.sh [--filter <pattern>] [--select-device]
#
# Options:
#   --filter <pattern>   Additional grep pattern on top of default Zedra filter
#   --select-device      Ignore saved device preference and re-prompt
#
# Device preference is saved per Claude Code session in /tmp/zedra-ios-device-$PPID.
# Run with --select-device to switch devices within the same session.
#
# Requires: libimobiledevice (brew install libimobiledevice) and USB-connected device.

set -euo pipefail

FILTER=""
SELECT_DEVICE=false
PREF_FILE="/tmp/zedra-ios-device-$PPID"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)
            FILTER="$2"; shift 2 ;;
        --select-device)
            SELECT_DEVICE=true; shift ;;
        -h|--help)
            sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Device selection
# ---------------------------------------------------------------------------
DEVICE_UDID=""
DEVICE_NAME=""

if [[ "$SELECT_DEVICE" == false ]] && [[ -f "$PREF_FILE" ]]; then
    IFS='|' read -r DEVICE_UDID DEVICE_NAME < "$PREF_FILE"
    echo "==> Using saved device: $DEVICE_NAME ($DEVICE_UDID)"
fi

if [[ -z "$DEVICE_UDID" ]]; then
    echo "==> Enumerating connected devices..."
    DEVICE_LINES=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+' | grep -v Simulator)

    if [[ -z "$DEVICE_LINES" ]]; then
        echo "Error: No physical iOS devices found. Connect a device and try again." >&2
        exit 1
    fi

    # Build a numbered menu
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

    if [[ -z "$DEVICE_UDID" ]]; then
        echo "Error: Could not parse UDID from: $SELECTED_LINE" >&2
        exit 1
    fi

    echo "$DEVICE_UDID|$DEVICE_NAME" > "$PREF_FILE"
    echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
fi

# ---------------------------------------------------------------------------
# Stream logs via idevicesyslog (USB, no sudo required)
# ---------------------------------------------------------------------------
IDEVICESYSLOG="/opt/homebrew/bin/idevicesyslog"
if [[ ! -x "$IDEVICESYSLOG" ]]; then
    echo "Error: idevicesyslog not found at $IDEVICESYSLOG" >&2
    echo "Install with: brew install libimobiledevice" >&2
    exit 1
fi

# Verify device is visible to libimobiledevice (requires USB pairing)
if ! /opt/homebrew/bin/idevice_id -l 2>/dev/null | grep -q "$DEVICE_UDID"; then
    echo "Error: Device $DEVICE_UDID not visible to idevicesyslog." >&2
    echo "idevicesyslog requires a USB-connected (paired) device." >&2
    exit 1
fi

GREP_PATTERN='\[I |\[W |\[E |\[D |panic|PANIC|crash|CRASH|NSException|Terminating'
if [[ -n "$FILTER" ]]; then
    GREP_PATTERN="$GREP_PATTERN|$FILTER"
fi

echo "==> Streaming logs from $DEVICE_NAME (Ctrl-C to stop)..."
echo "    Level prefix: [I]=Info [W]=Warn [E]=Error [D]=Debug [T]=Trace"
echo ""

"$IDEVICESYSLOG" -u "$DEVICE_UDID" -p Zedra \
    | grep -E "$GREP_PATTERN" --line-buffered --color=auto
