#!/bin/bash
# Automatically analyze crashes and suggest fixes
# Usage: ./scripts/analyze-crash.sh

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     Zedra Crash Analyzer              ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
echo ""

# Get last crash from logcat
echo -e "${YELLOW}Searching for crashes in logcat...${NC}"

CRASH=$(adb logcat -d | grep -A 100 "FATAL EXCEPTION\|AndroidRuntime.*FATAL" | head -150)

if [ -z "$CRASH" ]; then
  echo -e "${GREEN}✓ No crash found in logcat${NC}"
  echo ""
  echo "Checking for other errors..."

  # Check for Vulkan errors
  VK_ERRORS=$(adb logcat -d | grep "VK_ERROR" | tail -10)
  if [ -n "$VK_ERRORS" ]; then
    echo -e "${YELLOW}⚠ Found Vulkan errors:${NC}"
    echo "$VK_ERRORS"
    echo ""
    echo -e "${BLUE}💡 Suggested Fix:${NC}"
    echo "  Check device Vulkan support: adb shell getprop ro.hardware.vulkan"
    echo "  Expected: 1.1.0 or higher"
    exit 0
  fi

  # Check for JNI errors
  JNI_ERRORS=$(adb logcat -d | grep -E "JNI ERROR|No implementation found" | tail -10)
  if [ -n "$JNI_ERRORS" ]; then
    echo -e "${YELLOW}⚠ Found JNI errors:${NC}"
    echo "$JNI_ERRORS"
    echo ""
    echo -e "${BLUE}💡 Suggested Fix:${NC}"
    echo "  Rebuild native libraries: ./scripts/build-android.sh"
    echo "  Check JNI method signatures in src/android_jni.rs"
    exit 0
  fi

  echo -e "${GREEN}No errors detected${NC}"
  exit 0
fi

echo -e "${RED}═══ CRASH DETECTED ═══${NC}"
echo ""
echo "$CRASH"
echo ""
echo -e "${BLUE}═══ ANALYSIS ═══${NC}"
echo ""

# Pattern matching for common issues

# 1. JNI UnsatisfiedLinkError
if echo "$CRASH" | grep -q "UnsatisfiedLinkError"; then
  echo -e "${RED}🔧 Issue: JNI library not found${NC}"
  echo ""
  echo "Common causes:"
  echo "  - Native library not built for target architecture"
  echo "  - Library not copied to android/app/libs/"
  echo "  - JNI method signature mismatch"
  echo ""
  echo -e "${BLUE}💡 Suggested Fix:${NC}"
  echo "  1. Rebuild: ./scripts/build-android.sh"
  echo "  2. Verify libs exist: ls android/app/libs/arm64-v8a/"
  echo "  3. Check JNI exports in src/android_jni.rs match Java declarations"
  echo ""
  echo "Related files:"
  echo "  - src/android_jni.rs (Rust JNI exports)"
  echo "  - android/app/src/main/java/dev/zedra/app/MainActivity.java (Java declarations)"

# 2. Vulkan errors
elif echo "$CRASH" | grep -qE "VK_ERROR|Vulkan"; then
  echo -e "${RED}🔧 Issue: Vulkan error${NC}"
  echo ""
  VK_ERROR_TYPE=$(echo "$CRASH" | grep -oE "VK_ERROR_[A-Z_]+")
  if [ -n "$VK_ERROR_TYPE" ]; then
    echo "Error type: $VK_ERROR_TYPE"
    echo ""
  fi
  echo -e "${BLUE}💡 Suggested Fix:${NC}"
  echo "  1. Check device Vulkan support: adb shell getprop ro.hardware.vulkan"
  echo "  2. Verify Vulkan 1.1+ required"
  echo "  3. Check surface creation in vendor/zed/crates/gpui/src/platform/android/window.rs"
  echo ""
  echo "Common VK_ERROR meanings:"
  echo "  - VK_ERROR_INITIALIZATION_FAILED: Device doesn't support required features"
  echo "  - VK_ERROR_SURFACE_LOST_KHR: Surface destroyed during rendering"
  echo "  - VK_ERROR_OUT_OF_DATE_KHR: Surface size changed (rotation?)"
  echo ""
  echo "Related files:"
  echo "  - vendor/zed/crates/gpui/src/platform/android/window.rs:handle_surface_created()"
  echo "  - vendor/zed/crates/gpui/src/platform/blade/blade_renderer.rs"
  echo ""
  echo "See docs/BLADE_INTEGRATION.md for Vulkan 1.1 compatibility notes"

# 3. Surface lifecycle issues
elif echo "$CRASH" | grep -qE "Surface|EGL"; then
  echo -e "${RED}🔧 Issue: Surface lifecycle problem${NC}"
  echo ""
  echo -e "${BLUE}💡 Suggested Fix:${NC}"
  echo "  1. Check surface creation sequence in logs"
  echo "  2. Verify handle_surface_created() called before rendering"
  echo "  3. Check window.rs surface sizing logic"
  echo ""
  echo "Critical code location:"
  echo "  - vendor/zed/crates/gpui/src/platform/android/window.rs:206-215"
  echo "  - Physical pixels = Logical pixels × scale (3.0)"
  echo ""
  echo "Debug commands:"
  echo "  adb logcat -d | grep 'Surface created'"
  echo "  adb logcat -d | grep 'BladeRenderer'"

# 4. Rust panic
elif echo "$CRASH" | grep -q "panicked at"; then
  echo -e "${RED}🔧 Issue: Rust panic${NC}"
  echo ""
  PANIC_MSG=$(echo "$CRASH" | grep "panicked at" | head -1)
  echo "Panic message:"
  echo "  $PANIC_MSG"
  echo ""

  # Extract file and line if present
  FILE_LINE=$(echo "$PANIC_MSG" | grep -oE "[a-z_/]+\.rs:[0-9]+")
  if [ -n "$FILE_LINE" ]; then
    echo "Location: $FILE_LINE"
    echo ""
  fi

  echo -e "${BLUE}💡 Suggested Fix:${NC}"
  echo "  1. Review panic location and backtrace above"
  echo "  2. Check for threading violations (GPUI must run on main thread)"
  echo "  3. Verify command queue properly isolates JNI from GPUI"
  echo ""
  echo "Common panic causes:"
  echo "  - unwrap() on None/Err without proper error handling"
  echo "  - Threading violation (GPUI called from wrong thread)"
  echo "  - Resource already borrowed (RefCell borrow conflict)"
  echo ""
  echo "Related architecture:"
  echo "  - See docs/ARCHITECTURE.md for threading model"
  echo "  - src/android_command_queue.rs (thread isolation)"

# 5. NullPointerException
elif echo "$CRASH" | grep -q "NullPointerException"; then
  echo -e "${RED}🔧 Issue: Null pointer in Java code${NC}"
  echo ""
  NPE_LOCATION=$(echo "$CRASH" | grep "NullPointerException" -A 3 | grep "at dev.zedra" | head -1)
  if [ -n "$NPE_LOCATION" ]; then
    echo "Location:"
    echo "  $NPE_LOCATION"
    echo ""
  fi
  echo -e "${BLUE}💡 Suggested Fix:${NC}"
  echo "  1. Check null safety in Java files"
  echo "  2. Verify JNI handle (gpuiHandle) is set before use"
  echo "  3. Check surface lifecycle in GpuiSurfaceView.java"
  echo ""
  echo "Related files:"
  echo "  - android/app/src/main/java/dev/zedra/app/MainActivity.java"
  echo "  - android/app/src/main/java/dev/zedra/app/GpuiSurfaceView.java"

# 6. Unknown crash
else
  echo -e "${YELLOW}❓ Unknown crash pattern${NC}"
  echo ""
  echo "The crash doesn't match known patterns. Consider:"
  echo "  1. Reviewing full stack trace above"
  echo "  2. Checking docs/TECHNICAL_DEBT.md for known issues"
  echo "  3. Enabling more verbose logging"
  echo ""
  echo -e "${BLUE}💡 General debugging steps:${NC}"
  echo "  1. Clear logs and reproduce: adb logcat -c && ./scripts/dev-cycle.sh"
  echo "  2. Check all logcat output: adb logcat -d > crash.log"
  echo "  3. Review initialization sequence in android_app.rs"
fi

echo ""
echo -e "${BLUE}════════════════════════════════════════${NC}"
echo ""
echo "Full crash saved. To view again:"
echo "  adb logcat -d | grep -A 100 'FATAL EXCEPTION'"
echo ""
echo "See also:"
echo "  - docs/TECHNICAL_DEBT.md (known issues)"
echo "  - docs/ARCHITECTURE.md (design patterns)"
echo "  - CLAUDE.md (critical code locations)"
