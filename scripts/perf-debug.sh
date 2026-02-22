#!/usr/bin/env bash
# perf-debug.sh — Capture and analyze [PERF] + [gpui_wgpu] diagnostic logs from Android device.
#
# Usage:
#   ./scripts/perf-debug.sh [SECONDS]   (default: 30)
#
# Captures logcat for the given duration, then produces a categorized summary.

set -euo pipefail

DURATION="${1:-30}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RAW_LOG="/tmp/zedra-perf-${TIMESTAMP}.log"
FILTERED_LOG="/tmp/zedra-perf-${TIMESTAMP}-filtered.log"

echo "=== Zedra Perf Debug ==="
echo "Duration: ${DURATION}s"
echo "Raw log:  ${RAW_LOG}"
echo ""

# Check device
if ! adb devices 2>/dev/null | grep -q 'device$'; then
    echo "ERROR: No Android device connected"
    exit 1
fi

# Clear logcat
adb logcat -c 2>/dev/null || true

echo "Capturing logcat for ${DURATION}s... (interact with the app now)"
timeout "${DURATION}" adb logcat 2>/dev/null > "${RAW_LOG}" || true

echo "Capture complete. Analyzing..."
echo ""

# Filter relevant lines
grep -E '\[PERF\]|\[gpui_wgpu\]|FATAL|panic|CRASH|ANR' "${RAW_LOG}" > "${FILTERED_LOG}" 2>/dev/null || true

TOTAL_LINES=$(wc -l < "${FILTERED_LOG}" | tr -d ' ')
echo "=== Summary: ${TOTAL_LINES} diagnostic lines ==="
echo ""

# --- Category: Renderer ---
echo "--- Renderer ([gpui_wgpu]) ---"
RENDERER_LINES=$(grep -c '\[gpui_wgpu\]' "${FILTERED_LOG}" 2>/dev/null || echo 0)
echo "  Lines: ${RENDERER_LINES}"

# Extract frame times if present
FRAME_TIMES=$(grep -oP 'frame_ms=\K[0-9.]+' "${FILTERED_LOG}" 2>/dev/null || true)
if [ -n "${FRAME_TIMES}" ]; then
    echo "  Frame times (ms):"
    echo "${FRAME_TIMES}" | awk '{
        sum += $1; count++;
        if (count == 1 || $1 < min) min = $1;
        if ($1 > max) max = $1;
        times[count] = $1;
    } END {
        if (count > 0) {
            printf "    min=%.1f  max=%.1f  avg=%.1f  samples=%d\n", min, max, sum/count, count;
            # p95
            asort(times);
            p95_idx = int(count * 0.95);
            if (p95_idx < 1) p95_idx = 1;
            printf "    p95=%.1f\n", times[p95_idx];
        }
    }'
fi
echo ""

# --- Category: Navigation ---
echo "--- Navigation ---"
grep '\[PERF\] screen:' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
grep '\[PERF\] file loaded:' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Category: Editor ---
echo "--- Editor ---"
EDITOR_REBUILDS=$(grep -c '\[PERF\] editor:' "${FILTERED_LOG}" 2>/dev/null || echo 0)
echo "  Cache rebuilds: ${EDITOR_REBUILDS}"
grep '\[PERF\] editor:' "${FILTERED_LOG}" 2>/dev/null | tail -3 | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Category: Terminal ---
echo "--- Terminal ---"
grep '\[PERF\] terminal' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Category: Touch / Fling ---
echo "--- Touch ---"
FLING_STARTS=$(grep -c '\[PERF\] fling: start' "${FILTERED_LOG}" 2>/dev/null || echo 0)
FLING_ENDS=$(grep -c '\[PERF\] fling: end' "${FILTERED_LOG}" 2>/dev/null || echo 0)
echo "  Flings: ${FLING_STARTS} started, ${FLING_ENDS} ended"
grep '\[PERF\] drawer:' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Category: Transport ---
echo "--- Transport ---"
grep '\[PERF\] transport:' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Category: File Explorer ---
echo "--- File Explorer ---"
grep '\[PERF\] file_explorer:' "${FILTERED_LOG}" 2>/dev/null | while IFS= read -r line; do
    echo "  ${line##*\[PERF\] }"
done
echo ""

# --- Errors / Crashes ---
echo "--- Errors & Crashes ---"
ERRORS=$(grep -ciE 'FATAL|panic|CRASH|ANR' "${RAW_LOG}" 2>/dev/null || echo 0)
echo "  Error lines: ${ERRORS}"
if [ "${ERRORS}" -gt 0 ]; then
    grep -iE 'FATAL|panic|CRASH|ANR' "${RAW_LOG}" 2>/dev/null | head -10 | while IFS= read -r line; do
        echo "  ${line}"
    done
fi
echo ""

# --- Memory ---
echo "--- Memory ---"
MEM_INFO=$(adb shell dumpsys meminfo dev.zedra.app 2>/dev/null | grep 'TOTAL PSS' || true)
if [ -n "${MEM_INFO}" ]; then
    echo "  ${MEM_INFO}"
else
    echo "  (app not running or no meminfo available)"
fi
echo ""

echo "Raw log saved to: ${RAW_LOG}"
echo "Filtered log saved to: ${FILTERED_LOG}"
