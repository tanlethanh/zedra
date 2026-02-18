#!/usr/bin/env bash
# Full development session: host daemon + Android build/install/launch + logcat
#
# Usage:
#   ./scripts/dev-session.sh                  # Full build + deploy + monitor
#   ./scripts/dev-session.sh --skip-build     # Restart daemon + install existing APK
#   ./scripts/dev-session.sh --host-only      # Only start the host daemon
#   ./scripts/dev-session.sh --port 3000      # Custom daemon port

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Resolve project root relative to this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
SKIP_BUILD=false
HOST_ONLY=false
PORT=2123
HOST_PID=""

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-build) SKIP_BUILD=true; shift ;;
        --host-only) HOST_ONLY=true; shift ;;
        --port) PORT="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --skip-build   Skip Rust compilation (restart daemon + install existing APK)"
            echo "  --host-only    Only start/restart the host daemon, skip Android build/install"
            echo "  --port PORT    Custom daemon port (default: 2123)"
            echo "  -h, --help     Show this help message"
            exit 0
            ;;
        *) echo -e "${RED}Unknown option: $1${NC}"; exit 1 ;;
    esac
done

# Cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}Shutting down...${NC}"
    if [ -n "$HOST_PID" ] && kill -0 "$HOST_PID" 2>/dev/null; then
        echo -e "${BLUE}Stopping host daemon (PID $HOST_PID)...${NC}"
        kill "$HOST_PID" 2>/dev/null || true
        wait "$HOST_PID" 2>/dev/null || true
    fi
    # Also kill any other zedra-host daemon processes on our port
    pkill -f "zedra-host start.*--port $PORT" 2>/dev/null || true
    echo -e "${GREEN}Clean shutdown.${NC}"
}
trap cleanup EXIT INT TERM

echo -e "${BLUE}======================================${NC}"
echo -e "${BLUE}  Zedra Dev Session${NC}"
echo -e "${BLUE}======================================${NC}"
echo ""

cd "$PROJECT_ROOT"

# --- Step 1: Build host binary (desktop target) ---
if [ "$SKIP_BUILD" = false ]; then
    echo -e "${YELLOW}[1/6] Building zedra-host (desktop)...${NC}"
    cargo build -p zedra-host --release
    echo -e "${GREEN}  Host binary built.${NC}"
else
    echo -e "${YELLOW}[1/6] Skipping host build (--skip-build)${NC}"
    if [ ! -f "$PROJECT_ROOT/target/release/zedra-host" ]; then
        echo -e "${RED}  Error: target/release/zedra-host not found. Run without --skip-build first.${NC}"
        exit 1
    fi
fi
echo ""

# --- Step 2: Kill existing daemon and start fresh ---
echo -e "${YELLOW}[2/6] Starting host daemon on port $PORT...${NC}"
pkill -f "zedra-host start.*--port $PORT" 2>/dev/null || true
sleep 0.5

"$PROJECT_ROOT/target/release/zedra-host" start --port "$PORT" --bind 0.0.0.0 --workdir "$PROJECT_ROOT" &
HOST_PID=$!

# Wait for daemon to be ready
sleep 1
if ! kill -0 "$HOST_PID" 2>/dev/null; then
    echo -e "${RED}  Host daemon failed to start! Check logs above.${NC}"
    exit 1
fi
echo -e "${GREEN}  Host daemon running (PID $HOST_PID)${NC}"
echo ""

# Resolve host LAN IP for connection info
HOST_IP=$(python3 -c "import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.connect(('8.8.8.8',80)); print(s.getsockname()[0]); s.close()" 2>/dev/null || echo "localhost")

# --- Host-only mode: wait and exit ---
if [ "$HOST_ONLY" = true ]; then
    echo -e "${GREEN}=== Host Daemon Running ===${NC}"
    echo -e "${BLUE}  Address : ${HOST_IP}:${PORT}${NC}"
    echo -e "${BLUE}  Workdir : ${PROJECT_ROOT}${NC}"
    echo -e "${BLUE}  PID     : ${HOST_PID}${NC}"
    echo ""
    echo -e "${YELLOW}Press Ctrl+C to stop.${NC}"
    wait "$HOST_PID"
    exit 0
fi

# --- Step 3: Build Android ---
if [ "$SKIP_BUILD" = false ]; then
    echo -e "${YELLOW}[3/6] Building Android APK...${NC}"
    "$SCRIPT_DIR/build-android.sh"
    echo -e "${GREEN}  Android libraries built.${NC}"
else
    echo -e "${YELLOW}[3/6] Skipping Android build (--skip-build)${NC}"
fi
echo ""

# --- Step 4: Install APK ---
echo -e "${YELLOW}[4/6] Installing APK...${NC}"

# Check device connection
if ! adb devices | grep -q "device$"; then
    echo -e "${RED}  Error: No Android device connected.${NC}"
    echo "  Run 'adb devices' to check connection."
    exit 1
fi

# Handle multiple devices - prefer physical device over emulator
DEVICE_COUNT=$(adb devices | grep -c "device$" || true)
if [ "$DEVICE_COUNT" -gt 1 ]; then
    PHYSICAL_DEVICE=$(adb devices | grep "device$" | grep -v "emulator" | head -1 | awk '{print $1}')
    if [ -n "$PHYSICAL_DEVICE" ]; then
        export ANDROID_SERIAL="$PHYSICAL_DEVICE"
        echo -e "${BLUE}  Multiple devices detected, using physical: $PHYSICAL_DEVICE${NC}"
    fi
fi

cd "$PROJECT_ROOT/android" && ./gradlew installDebug
cd "$PROJECT_ROOT"
echo -e "${GREEN}  APK installed.${NC}"
echo ""

# --- Step 5: Launch app ---
echo -e "${YELLOW}[5/6] Launching app...${NC}"
adb shell am force-stop dev.zedra.app 2>/dev/null || true
adb shell am start -n dev.zedra.app/.MainActivity

sleep 2
if adb shell pidof dev.zedra.app > /dev/null 2>&1; then
    echo -e "${GREEN}  App launched successfully.${NC}"
else
    echo -e "${RED}  App may have failed to start. Check logcat output below.${NC}"
fi
echo ""

# --- Connection info ---
echo -e "${GREEN}=== Dev Session Ready ===${NC}"
echo -e "${BLUE}  Host    : ${HOST_IP}:${PORT}${NC}"
echo -e "${BLUE}  Workdir : ${PROJECT_ROOT}${NC}"
echo -e "${BLUE}  Daemon  : PID ${HOST_PID}${NC}"
echo ""
echo -e "${YELLOW}Press Ctrl+C to stop.${NC}"
echo -e "${BLUE}--------------------------------------${NC}"
echo ""

# --- Step 6: Monitor logcat ---
adb logcat -c
adb logcat -v threadtime | grep --line-buffered -E "zedra|GPUI|RPC|session|FATAL|VK_ERROR|panicked" | while IFS= read -r line; do
    if echo "$line" | grep -qE "FATAL|VK_ERROR|panicked"; then
        echo -e "${RED}$line${NC}"
    elif echo "$line" | grep -qiE "ERROR|error"; then
        echo -e "${YELLOW}$line${NC}"
    else
        echo "$line"
    fi
done
