#!/bin/bash
# ios-log.sh — Stream logs from a USB-connected iOS device or running simulator.
#
# Usage:
#   ./scripts/ios-log.sh [--filter <pattern>] [--select-device] [--simulator]
#
# Options:
#   --filter <pattern>   Additional grep pattern on top of default Zedra filter
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

# ---------------------------------------------------------------------------
# Device selection
# ---------------------------------------------------------------------------
DEVICE_UDID=""
DEVICE_NAME=""
DEVICE_TYPE=""  # "sim" or "dev"

if [[ "$SELECT_DEVICE" == false ]] && [[ -f "$PREF_FILE" ]]; then
    IFS='|' read -r DEVICE_TYPE DEVICE_UDID DEVICE_NAME < "$PREF_FILE"
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

        echo "sim|$DEVICE_UDID|$DEVICE_NAME" > "$PREF_FILE"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    else
        # -----------------------------------------------------------------------
        # Physical device selection
        # -----------------------------------------------------------------------
        echo "==> Enumerating connected devices..."
        DEVICE_LINES=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+' | grep -v Simulator)

        if [[ -z "$DEVICE_LINES" ]]; then
            echo "Error: No physical iOS devices found. Connect a device or use --simulator." >&2
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

        echo "dev|$DEVICE_UDID|$DEVICE_NAME" > "$PREF_FILE"
        echo "==> Selected: $DEVICE_NAME ($DEVICE_UDID)"
    fi
fi

# ---------------------------------------------------------------------------
# Stream logs
# ---------------------------------------------------------------------------
GREP_PATTERN='\[I |\[W |\[E |\[D |panic|PANIC|crash|CRASH|NSException|Terminating'
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

    "$IDEVICESYSLOG" -u "$DEVICE_UDID" -p Zedra \
        | grep -E "$GREP_PATTERN" --line-buffered --color=auto
fi
