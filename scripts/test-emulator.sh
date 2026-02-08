#!/usr/bin/env bash
# Android emulator E2E test script.
#
# Creates an AVD (if needed), boots the emulator, builds/installs the APK,
# launches the app, and verifies it runs (logcat assertions + screenshot).
#
# Usage:
#   ./scripts/test-emulator.sh           # full flow
#   ./scripts/test-emulator.sh --skip-build  # skip Rust/Gradle build
#   ./scripts/test-emulator.sh --keep     # don't kill emulator on exit
#
# Requires: Android SDK (ANDROID_HOME), NDK, Rust targets, emulator, adb

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

AVD_NAME="zedra_test"
API_LEVEL=31
ABI="x86_64"
PACKAGE="dev.zedra.app"
ACTIVITY=".MainActivity"
TIMEOUT_BOOT=120     # seconds to wait for emulator boot
TIMEOUT_RENDER=30    # seconds to wait for first frame
SKIP_BUILD=false
KEEP_EMULATOR=false

# Parse args
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
        --keep)       KEEP_EMULATOR=true ;;
        *)            echo "Unknown arg: $arg"; exit 1 ;;
    esac
done

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}✓ $1${NC}"; }
fail() { echo -e "${RED}✗ $1${NC}"; exit 1; }
info() { echo -e "${YELLOW}→ $1${NC}"; }

# ---------------------------------------------------------------------------
# 1. Verify environment
# ---------------------------------------------------------------------------

info "Checking prerequisites..."

if [ -z "${ANDROID_HOME:-}" ]; then
    # Try common paths
    for p in "$HOME/Android/Sdk" "$HOME/Library/Android/sdk"; do
        [ -d "$p" ] && export ANDROID_HOME="$p" && break
    done
fi
[ -z "${ANDROID_HOME:-}" ] && fail "ANDROID_HOME not set and SDK not found"

EMULATOR="$ANDROID_HOME/emulator/emulator"
AVDMANAGER="$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager"
SDKMANAGER="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
ADB="$ANDROID_HOME/platform-tools/adb"

[ -x "$ADB" ] || ADB="$(command -v adb)" || fail "adb not found"
[ -x "$EMULATOR" ] || fail "emulator not found at $EMULATOR"

pass "Environment OK (ANDROID_HOME=$ANDROID_HOME)"

# ---------------------------------------------------------------------------
# 2. Create AVD if needed
# ---------------------------------------------------------------------------

if ! "$EMULATOR" -list-avds 2>/dev/null | grep -q "^${AVD_NAME}$"; then
    info "Creating AVD '$AVD_NAME' (API $API_LEVEL, $ABI)..."

    SYSTEM_IMAGE="system-images;android-${API_LEVEL};google_apis;${ABI}"
    "$SDKMANAGER" --install "$SYSTEM_IMAGE" 2>/dev/null || true

    echo "no" | "$AVDMANAGER" create avd \
        -n "$AVD_NAME" \
        -k "$SYSTEM_IMAGE" \
        -d "pixel_4" \
        --force
    pass "AVD created"
else
    pass "AVD '$AVD_NAME' exists"
fi

# ---------------------------------------------------------------------------
# 3. Boot emulator
# ---------------------------------------------------------------------------

# Kill any existing emulator
EMULATOR_PID=""
cleanup() {
    if [ "$KEEP_EMULATOR" = false ] && [ -n "$EMULATOR_PID" ]; then
        info "Shutting down emulator (PID $EMULATOR_PID)..."
        kill "$EMULATOR_PID" 2>/dev/null || true
        "$ADB" -s emulator-5554 emu kill 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Check if an emulator is already running
if "$ADB" devices 2>/dev/null | grep -q "emulator-5554"; then
    info "Emulator already running"
else
    info "Booting emulator..."
    "$EMULATOR" -avd "$AVD_NAME" \
        -no-window -no-audio -no-snapshot-save \
        -gpu swiftshader_indirect \
        -memory 2048 \
        &
    EMULATOR_PID=$!

    # Wait for boot
    info "Waiting for boot (max ${TIMEOUT_BOOT}s)..."
    SECONDS=0
    while [ $SECONDS -lt $TIMEOUT_BOOT ]; do
        if "$ADB" shell getprop sys.boot_completed 2>/dev/null | grep -q "1"; then
            break
        fi
        sleep 2
    done

    if ! "$ADB" shell getprop sys.boot_completed 2>/dev/null | grep -q "1"; then
        fail "Emulator failed to boot within ${TIMEOUT_BOOT}s"
    fi
    pass "Emulator booted"
fi

# ---------------------------------------------------------------------------
# 4. Build and install
# ---------------------------------------------------------------------------

if [ "$SKIP_BUILD" = false ]; then
    info "Building Rust libraries..."
    "$SCRIPT_DIR/build-android.sh"
    pass "Rust build complete"

    info "Building and installing APK..."
    cd "$PROJECT_DIR/android" && ./gradlew installDebug 2>&1 | tail -3
    cd "$PROJECT_DIR"
    pass "APK installed"
else
    info "Skipping build (--skip-build)"
fi

# ---------------------------------------------------------------------------
# 5. Launch app and verify
# ---------------------------------------------------------------------------

info "Clearing logcat..."
"$ADB" logcat -c

info "Launching $PACKAGE/$ACTIVITY..."
"$ADB" shell am start -n "$PACKAGE/$ACTIVITY" -a android.intent.action.MAIN -c android.intent.category.LAUNCHER

info "Waiting for first frame (max ${TIMEOUT_RENDER}s)..."
SECONDS=0
RENDER_OK=false
while [ $SECONDS -lt $TIMEOUT_RENDER ]; do
    if "$ADB" logcat -d -s zedra 2>/dev/null | grep -qi "frame\|render\|draw\|surface"; then
        RENDER_OK=true
        break
    fi
    sleep 1
done

if [ "$RENDER_OK" = true ]; then
    pass "Rendering confirmed via logcat"
else
    # Not fatal — some devices/emulators don't log frames
    info "No frame log found (may be normal for emulator)"
fi

# Check app is still running (didn't crash)
sleep 2
if "$ADB" shell pidof "$PACKAGE" >/dev/null 2>&1; then
    pass "App process alive"
else
    # Dump crash info
    echo ""
    echo "=== Recent crash logs ==="
    "$ADB" logcat -d | grep -i "fatal\|panic\|crash\|SIGABRT\|zedra" | tail -20
    fail "App crashed on launch"
fi

# Screenshot
info "Capturing screenshot..."
SCREENSHOT="/tmp/zedra-emulator-test.png"
"$ADB" shell screencap -p /sdcard/zedra_test.png
"$ADB" pull /sdcard/zedra_test.png "$SCREENSHOT" 2>/dev/null
if [ -f "$SCREENSHOT" ]; then
    SIZE=$(stat -f%z "$SCREENSHOT" 2>/dev/null || stat -c%s "$SCREENSHOT" 2>/dev/null || echo 0)
    if [ "$SIZE" -gt 1000 ]; then
        pass "Screenshot saved: $SCREENSHOT (${SIZE} bytes)"
    else
        info "Screenshot too small — possible black screen"
    fi
fi

# Collect logcat summary
info "Logcat summary (zedra tags):"
"$ADB" logcat -d -s zedra 2>/dev/null | tail -20

echo ""
echo -e "${GREEN}═══════════════════════════════════════${NC}"
echo -e "${GREEN}  Emulator E2E test complete           ${NC}"
echo -e "${GREEN}═══════════════════════════════════════${NC}"
