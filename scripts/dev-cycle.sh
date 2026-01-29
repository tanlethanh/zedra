#!/bin/bash
# Automated development cycle: build → install → launch → monitor
# Usage: ./scripts/dev-cycle.sh

set -e

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

if ! ./scripts/build-android.sh; then
  echo -e "${RED}✗ Build failed${NC}"
  exit 1
fi

echo -e "${GREEN}✓ Rust build complete${NC}"
echo ""

# Step 4: Install APK
echo -e "${YELLOW}[4/6] Installing APK...${NC}"
cd "$PROJECT_ROOT/android"

if ! ./gradlew installDebug 2>&1 | grep -E "BUILD SUCCESSFUL|UP-TO-DATE"; then
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
