#!/bin/bash
# Automated development cycle: build → install → launch → monitor
# Usage: ./scripts/dev-cycle.sh [--preview] [--emulator] [--no-log]

set -e

BUILD_FLAGS=""
USE_EMULATOR=false
NO_LOG=false
for arg in "$@"; do
    case "$arg" in
        --preview) BUILD_FLAGS="$BUILD_FLAGS --preview" ;;
        --emulator) USE_EMULATOR=true ;;
        --no-log) NO_LOG=true ;;
    esac
done

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║   Zedra Android Development Cycle     ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
echo ""

# Step 1: Pre-flight checks
echo -e "${YELLOW}[1/6] Pre-flight checks...${NC}"

# Check device connection
if ! adb devices | grep -q "device$"; then
  echo -e "${RED}✗ Error: No Android device connected${NC}"
  echo "  Run 'adb devices' to check connection"
  exit 1
fi

# Handle multiple devices
DEVICE_COUNT=$(adb devices | grep -c "device$" || true)
if [ "$DEVICE_COUNT" -gt 1 ]; then
  if $USE_EMULATOR; then
    echo -e "${YELLOW}  Multiple devices detected ($DEVICE_COUNT), selecting emulator...${NC}"
    EMU_DEVICE=$(adb devices | grep "device$" | grep "emulator" | head -1 | awk '{print $1}')
    if [ -n "$EMU_DEVICE" ]; then
      export ANDROID_SERIAL="$EMU_DEVICE"
      echo -e "${GREEN}  → Selected emulator: $EMU_DEVICE${NC}"
    else
      FIRST_DEVICE=$(adb devices | grep "device$" | head -1 | awk '{print $1}')
      export ANDROID_SERIAL="$FIRST_DEVICE"
      echo -e "${YELLOW}  → No emulator found, using: $FIRST_DEVICE${NC}"
    fi
  else
    echo -e "${YELLOW}  Multiple devices detected ($DEVICE_COUNT), selecting physical device...${NC}"
    PHYSICAL_DEVICE=$(adb devices | grep "device$" | grep -v "emulator" | head -1 | awk '{print $1}')
    if [ -n "$PHYSICAL_DEVICE" ]; then
      export ANDROID_SERIAL="$PHYSICAL_DEVICE"
      echo -e "${GREEN}  → Selected physical device: $PHYSICAL_DEVICE${NC}"
    else
      FIRST_DEVICE=$(adb devices | grep "device$" | head -1 | awk '{print $1}')
      export ANDROID_SERIAL="$FIRST_DEVICE"
      echo -e "${YELLOW}  → No physical device, using: $FIRST_DEVICE${NC}"
    fi
  fi
elif $USE_EMULATOR; then
  # Single device — warn if user asked for emulator but it's not one
  SINGLE_DEVICE=$(adb devices | grep "device$" | head -1 | awk '{print $1}')
  if ! echo "$SINGLE_DEVICE" | grep -q "emulator"; then
    echo -e "${YELLOW}⚠ --emulator requested but connected device is not an emulator${NC}"
  fi
fi

DEVICE_MODEL=$(adb shell getprop ro.product.model | tr -d '\r')
echo -e "${GREEN}✓ Device connected: $DEVICE_MODEL${NC}"

# Check Vulkan support
VULKAN_VERSION=$(adb shell getprop ro.hardware.vulkan | tr -d '\r')
if [ -z "$VULKAN_VERSION" ]; then
  echo -e "${YELLOW}⚠ Warning: Could not detect Vulkan version${NC}"
else
  echo -e "${GREEN}✓ Vulkan version: $VULKAN_VERSION${NC}"
fi

# Check available storage
STORAGE=$(adb shell df /data/local/tmp | tail -1 | awk '{print $4}')
echo -e "${GREEN}✓ Available storage: ~${STORAGE}${NC}"

echo ""

# Step 2: Clear old logs
echo -e "${YELLOW}[2/6] Clearing logcat buffer...${NC}"
adb logcat -c
echo -e "${GREEN}✓ Logcat cleared${NC}"
echo ""

# Step 3: Build Rust libraries
echo -e "${YELLOW}[3/6] Building Rust libraries...${NC}"
cd "$PROJECT_ROOT"

CARGO_FEATURES=""
if echo "$BUILD_FLAGS" | grep -q -- "--preview"; then
  CARGO_FEATURES="--features preview"
  echo -e "${BLUE}  Preview mode enabled${NC}"
fi

# Detect target architecture from the connected device
DEVICE_ABI=$(adb shell getprop ro.product.cpu.abi | tr -d '\r')
case "$DEVICE_ABI" in
  arm64-v8a)   NDK_TARGET="arm64-v8a" ;;
  armeabi-v7a) NDK_TARGET="armeabi-v7a" ;;
  x86_64)      NDK_TARGET="x86_64" ;;
  x86)         NDK_TARGET="x86" ;;
  *)           NDK_TARGET="arm64-v8a"; echo -e "${YELLOW}⚠ Unknown ABI '$DEVICE_ABI', defaulting to arm64-v8a${NC}" ;;
esac

echo -e "${BLUE}  Target: $NDK_TARGET (debug)${NC}"
if ! cargo ndk -t "$NDK_TARGET" -o ./android/app/libs build -p zedra --lib $CARGO_FEATURES; then
  echo -e "${RED}✗ Build failed${NC}"
  exit 1
fi

# cargo-ndk may not overwrite libs built with different profiles (e.g. release).
# Explicitly copy the debug .so to ensure the APK gets the latest build.
case "$NDK_TARGET" in
  arm64-v8a)   RUST_TARGET="aarch64-linux-android" ;;
  armeabi-v7a) RUST_TARGET="armv7-linux-androideabi" ;;
  x86_64)      RUST_TARGET="x86_64-linux-android" ;;
  x86)         RUST_TARGET="i686-linux-android" ;;
esac
SRC="target/$RUST_TARGET/debug/libzedra.so"
DST="android/app/libs/$NDK_TARGET/libzedra.so"
if [ -f "$SRC" ] && [ "$SRC" -nt "$DST" ]; then
  cp "$SRC" "$DST"
  echo -e "${GREEN}  → Copied fresh lib to $DST${NC}"
fi

echo -e "${GREEN}✓ Rust build complete${NC}"
echo ""

# Step 4: Install APK
echo -e "${YELLOW}[4/6] Installing APK...${NC}"
cd "$PROJECT_ROOT/android"

if ! ./gradlew installDebug -x buildRustLib 2>&1 | grep -E "BUILD SUCCESSFUL|UP-TO-DATE"; then
  echo -e "${RED}✗ APK installation failed${NC}"
  exit 1
fi

echo -e "${GREEN}✓ APK installed${NC}"
cd "$PROJECT_ROOT"
echo ""

# Step 5: Launch app
echo -e "${YELLOW}[5/6] Launching app...${NC}"

# Force stop existing instance
adb shell am force-stop dev.zedra.app 2>/dev/null || true

# Launch MainActivity
adb shell am start -n dev.zedra.app/.MainActivity

# Wait for app to start
sleep 2

# Check if app is running
if adb shell pidof dev.zedra.app > /dev/null; then
  echo -e "${GREEN}✓ App launched successfully${NC}"
else
  echo -e "${RED}✗ App failed to start${NC}"
  echo "  Checking recent logs..."
  adb logcat -d | grep -E "FATAL|AndroidRuntime" | tail -20
  exit 1
fi

echo ""

if $NO_LOG; then
  echo -e "${GREEN}Done! (logcat skipped with --no-log)${NC}"
  exit 0
fi

# Step 6: Monitor logs
echo -e "${YELLOW}[6/6] Monitoring logs (Ctrl+C to stop)...${NC}"
echo -e "${BLUE}════════════════════════════════════════${NC}"
echo ""

# Show recent logs and continue monitoring
adb logcat -v threadtime | grep --line-buffered -E "zedra|FATAL|VK_ERROR|JNI ERROR|panicked at" | while read line; do
  # Color-code critical errors
  if echo "$line" | grep -qE "FATAL|VK_ERROR|panicked"; then
    echo -e "${RED}$line${NC}"
  elif echo "$line" | grep -q "ERROR"; then
    echo -e "${YELLOW}$line${NC}"
  else
    echo "$line"
  fi
done
