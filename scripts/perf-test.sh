#!/usr/bin/env bash
# Performance testing script for Zedra Android app.
# Captures logcat for a configurable duration, then reports frame timing,
# memory usage, and descriptor pool health.
#
# Usage: ./scripts/perf-test.sh [duration_seconds]
#   duration_seconds: how long to capture (default: 10)

set -euo pipefail

DURATION="${1:-10}"
PACKAGE="dev.zedra.app"
TMPLOG=$(mktemp /tmp/zedra-perf-XXXXXX.log)

cleanup() { rm -f "$TMPLOG"; }
trap cleanup EXIT

# Check device connected
if ! adb get-state &>/dev/null; then
    echo "ERROR: No Android device connected"
    exit 1
fi

# Check app is running
if ! adb shell pidof "$PACKAGE" &>/dev/null; then
    echo "WARNING: $PACKAGE is not running. Start the app first."
    echo "Launching..."
    adb shell am start -n "$PACKAGE/.MainActivity" 2>/dev/null || true
    sleep 2
fi

echo "=== Zedra Performance Test ==="
echo "Capturing ${DURATION}s of logs..."
echo ""

# Clear logcat buffer and capture for DURATION seconds
adb logcat -c
timeout "$DURATION" adb logcat -v time 2>/dev/null > "$TMPLOG" || true

# --- Memory ---
echo "--- Memory ---"
MEMINFO=$(adb shell dumpsys meminfo "$PACKAGE" 2>/dev/null | grep "TOTAL PSS" || echo "")
if [ -n "$MEMINFO" ]; then
    echo "  $MEMINFO"
else
    # Fallback: try TOTAL line
    MEMINFO=$(adb shell dumpsys meminfo "$PACKAGE" 2>/dev/null | grep "TOTAL:" | head -1 || echo "N/A")
    echo "  $MEMINFO"
fi
echo ""

# --- Frame Timing ---
echo "--- Frame Timing ---"
# Look for GPUI frame timing logs (format varies, check common patterns)
FRAME_LINES=$(grep -i "frame\|TIMING\|draw\|render" "$TMPLOG" | grep -i "zedra\|gpui\|blade" | head -20 || true)
FRAME_COUNT=$(echo "$FRAME_LINES" | grep -c "." || echo "0")
echo "  Frame-related log lines: $FRAME_COUNT"

# Extract any timing values (matches patterns like "42ms", "4.2ms", "42.1ms")
TIMINGS=$(echo "$FRAME_LINES" | grep -oE '[0-9]+(\.[0-9]+)?ms' || true)
if [ -n "$TIMINGS" ]; then
    echo "  Timing samples:"
    echo "$TIMINGS" | sort -n | head -10 | while read -r t; do echo "    $t"; done

    # Compute stats using awk
    echo "$TIMINGS" | sed 's/ms//' | awk '
    {
        vals[NR] = $1; sum += $1; count++
        if (NR == 1 || $1 < min) min = $1
        if (NR == 1 || $1 > max) max = $1
    }
    END {
        if (count == 0) exit
        avg = sum / count
        # Sort for percentile
        n = asort(vals)
        p95_idx = int(n * 0.95)
        if (p95_idx < 1) p95_idx = 1
        printf "  Stats: min=%.1fms  avg=%.1fms  max=%.1fms  p95=%.1fms  samples=%d\n", min, avg, max, vals[p95_idx], count
    }'
else
    echo "  No timing data found in logs."
    echo "  (Add [TIMING] log lines in Rust code for frame-level measurement)"
fi
echo ""

# --- Descriptor Pools ---
echo "--- Descriptor Pools ---"
DESC_LINES=$(grep -i "descriptor pool" "$TMPLOG" || true)
DESC_COUNT=$(echo "$DESC_LINES" | grep -c "." || echo "0")
if [ "$DESC_COUNT" -gt 0 ]; then
    echo "  Descriptor pool events: $DESC_COUNT"
    echo "$DESC_LINES" | tail -5 | while read -r line; do echo "    $line"; done

    # Check for the growth warning
    WARN_COUNT=$(echo "$DESC_LINES" | grep -c "maximum size cap" || echo "0")
    if [ "$WARN_COUNT" -gt 0 ]; then
        echo "  WARNING: Descriptor pool hit maximum size cap ($WARN_COUNT times)"
    fi
else
    echo "  No descriptor pool events (normal if app is idle)"
fi
echo ""

# --- Errors & Warnings ---
echo "--- Errors & Warnings ---"
ERRORS=$(grep -iE "error|panic|crash|fatal|SIGABRT|lowmemory" "$TMPLOG" | grep -iv "no error\|error_count.*0" | head -10 || true)
ERROR_COUNT=$(echo "$ERRORS" | grep -c "." || echo "0")
if [ "$ERROR_COUNT" -gt 0 ]; then
    echo "  Found $ERROR_COUNT error/warning lines:"
    echo "$ERRORS" | while read -r line; do echo "    $line"; done
else
    echo "  No errors or panics detected."
fi
echo ""

# --- GPU Info ---
echo "--- GPU Info ---"
GPU_RENDERER=$(adb shell dumpsys SurfaceFlinger 2>/dev/null | grep -i "GLES" | head -1 || echo "N/A")
echo "  $GPU_RENDERER"
echo ""

echo "=== Done (log saved to $TMPLOG) ==="
echo "Full log: $TMPLOG"
