#!/bin/bash
# Pre-flight checks for Android development
# Verifies device, tools, and environment before building
# Usage: ./scripts/preflight-check.sh

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

ERRORS=0
WARNINGS=0

echo -e "${BLUE}╔════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║    Zedra Pre-Flight Check             ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════╝${NC}"
echo ""

# Check 1: ADB
echo -e "${YELLOW}[1/10] Checking ADB...${NC}"
if command -v adb &> /dev/null; then
  ADB_VERSION=$(adb version | head -1)
  echo -e "${GREEN}✓ ADB found: $ADB_VERSION${NC}"
else
  echo -e "${RED}✗ ADB not found${NC}"
  echo "  Install Android SDK Platform Tools"
  ERRORS=$((ERRORS + 1))
fi
echo ""

# Check 2: Device Connection
echo -e "${YELLOW}[2/10] Checking device connection...${NC}"
DEVICE_COUNT=$(adb devices | grep -c "device$")

if [ "$DEVICE_COUNT" -eq 0 ]; then
  echo -e "${RED}✗ No device connected${NC}"
  echo "  Connect device via USB or enable wireless debugging"
  ERRORS=$((ERRORS + 1))
elif [ "$DEVICE_COUNT" -gt 1 ]; then
  echo -e "${YELLOW}⚠ Multiple devices connected ($DEVICE_COUNT)${NC}"
  echo "  Specify device with: export ANDROID_SERIAL=<device-id>"
  WARNINGS=$((WARNINGS + 1))
  adb devices
else
  DEVICE_MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r')
  DEVICE_ANDROID=$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '\r')
  echo -e "${GREEN}✓ Device: $DEVICE_MODEL (Android $DEVICE_ANDROID)${NC}"
fi
echo ""

# Check 3: Boot Completed (important for CI/CD)
echo -e "${YELLOW}[3/10] Checking boot status...${NC}"
BOOT_STATUS=$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')
if [ "$BOOT_STATUS" = "1" ]; then
  echo -e "${GREEN}✓ Device fully booted${NC}"
else
  echo -e "${YELLOW}⚠ Device still booting or unavailable${NC}"
  WARNINGS=$((WARNINGS + 1))
fi
echo ""

# Check 4: Vulkan Support
echo -e "${YELLOW}[4/10] Checking Vulkan support...${NC}"
VULKAN_VERSION=$(adb shell getprop ro.hardware.vulkan 2>/dev/null | tr -d '\r')

if [ -z "$VULKAN_VERSION" ]; then
  echo -e "${RED}✗ Could not detect Vulkan version${NC}"
  echo "  Device may not support Vulkan"
  ERRORS=$((ERRORS + 1))
else
  # Try to parse version, but handle non-numeric responses (like "mali")
  if echo "$VULKAN_VERSION" | grep -qE '^[0-9]+\.[0-9]+'; then
    MAJOR=$(echo "$VULKAN_VERSION" | cut -d. -f1)
    MINOR=$(echo "$VULKAN_VERSION" | cut -d. -f2)

    if [ "$MAJOR" -ge 1 ] && [ "$MINOR" -ge 1 ]; then
      echo -e "${GREEN}✓ Vulkan $VULKAN_VERSION (compatible)${NC}"
    else
      echo -e "${RED}✗ Vulkan $VULKAN_VERSION (need 1.1+)${NC}"
      ERRORS=$((ERRORS + 1))
    fi
  else
    # Non-numeric response (e.g., "mali")
    echo -e "${YELLOW}⚠ Vulkan property: $VULKAN_VERSION${NC}"
    echo "  Cannot parse version, checking via alternate method..."

    # Try to get actual Vulkan version from vulkaninfo or feature check
    VK_INFO=$(adb shell dumpsys SurfaceFlinger 2>/dev/null | grep -i vulkan | head -1)
    if [ -n "$VK_INFO" ]; then
      echo -e "${GREEN}✓ Vulkan supported: $VK_INFO${NC}"
    else
      echo -e "${YELLOW}⚠ Cannot verify Vulkan version, proceeding with caution${NC}"
      WARNINGS=$((WARNINGS + 1))
    fi
  fi
fi
echo ""

# Check 5: Storage Space
echo -e "${YELLOW}[5/10] Checking storage space...${NC}"
AVAILABLE_KB=$(adb shell df /data/local/tmp 2>/dev/null | tail -1 | awk '{print $4}')

if [ -n "$AVAILABLE_KB" ]; then
  AVAILABLE_MB=$((AVAILABLE_KB / 1024))
  if [ "$AVAILABLE_MB" -lt 100 ]; then
    echo -e "${YELLOW}⚠ Low storage: ${AVAILABLE_MB}MB${NC}"
    WARNINGS=$((WARNINGS + 1))
  else
    echo -e "${GREEN}✓ Available: ${AVAILABLE_MB}MB${NC}"
  fi
else
  echo -e "${YELLOW}⚠ Could not check storage${NC}"
  WARNINGS=$((WARNINGS + 1))
fi
echo ""

# Check 6: Rust Toolchain
echo -e "${YELLOW}[6/10] Checking Rust toolchain...${NC}"
if command -v cargo &> /dev/null; then
  RUST_VERSION=$(rustc --version | cut -d' ' -f2)
  echo -e "${GREEN}✓ Rust: $RUST_VERSION${NC}"

  # Check Android target
  if rustup target list | grep -q "aarch64-linux-android (installed)"; then
    echo -e "${GREEN}✓ Android target (aarch64-linux-android) installed${NC}"
  else
    echo -e "${RED}✗ Android target not installed${NC}"
    echo "  Run: rustup target add aarch64-linux-android"
    ERRORS=$((ERRORS + 1))
  fi
else
  echo -e "${RED}✗ Rust not found${NC}"
  echo "  Install from: https://rustup.rs/"
  ERRORS=$((ERRORS + 1))
fi
echo ""

# Check 7: cargo-ndk
echo -e "${YELLOW}[7/10] Checking cargo-ndk...${NC}"
if command -v cargo-ndk &> /dev/null; then
  echo -e "${GREEN}✓ cargo-ndk installed${NC}"
else
  echo -e "${RED}✗ cargo-ndk not found${NC}"
  echo "  Run: cargo install cargo-ndk"
  ERRORS=$((ERRORS + 1))
fi
echo ""

# Check 8: Android NDK
echo -e "${YELLOW}[8/10] Checking Android NDK...${NC}"
if [ -n "$ANDROID_NDK_ROOT" ]; then
  echo -e "${GREEN}✓ NDK path: $ANDROID_NDK_ROOT${NC}"

  # Check NDK version (r25c+ recommended)
  if [ -f "$ANDROID_NDK_ROOT/source.properties" ]; then
    NDK_VERSION=$(grep "Pkg.Revision" "$ANDROID_NDK_ROOT/source.properties" | cut -d= -f2 | tr -d ' ')
    echo -e "${GREEN}✓ NDK version: $NDK_VERSION${NC}"
  fi
else
  echo -e "${YELLOW}⚠ ANDROID_NDK_ROOT not set${NC}"
  echo "  Set in ~/.bashrc or ~/.zshrc"
  WARNINGS=$((WARNINGS + 1))
fi
echo ""

# Check 9: Gradle
echo -e "${YELLOW}[9/10] Checking Gradle...${NC}"
if [ -f "./android/gradlew" ]; then
  GRADLE_VERSION=$(cd android && ./gradlew --version 2>/dev/null | grep "Gradle" | head -1 || echo "Unknown")
  echo -e "${GREEN}✓ Gradle wrapper found${NC}"
  if [ "$GRADLE_VERSION" != "Unknown" ]; then
    echo "  $GRADLE_VERSION"
  fi
else
  echo -e "${RED}✗ Gradle wrapper not found${NC}"
  ERRORS=$((ERRORS + 1))
fi
echo ""

# Check 10: Git Submodules
echo -e "${YELLOW}[10/10] Checking git submodules...${NC}"
if [ -e "vendor/zed/.git" ]; then
  echo -e "${GREEN}✓ Zed submodule initialized${NC}"

  # Check if submodule is dirty
  cd vendor/zed
  if git diff-index --quiet HEAD -- 2>/dev/null; then
    echo -e "${GREEN}✓ Submodule clean${NC}"
  else
    echo -e "${YELLOW}⚠ Submodule has uncommitted changes${NC}"
    echo "  This is expected if you've modified GPUI platform code"
    WARNINGS=$((WARNINGS + 1))
  fi
  cd ../..
else
  echo -e "${RED}✗ Zed submodule not initialized${NC}"
  echo "  Run: git submodule update --init --recursive"
  ERRORS=$((ERRORS + 1))
fi
echo ""

# Summary
echo -e "${BLUE}════════════════════════════════════════${NC}"
echo ""

if [ $ERRORS -eq 0 ] && [ $WARNINGS -eq 0 ]; then
  echo -e "${GREEN}✓✓✓ ALL CHECKS PASSED ✓✓✓${NC}"
  echo ""
  echo "Ready to build! Run:"
  echo "  ./scripts/dev-cycle.sh"
  exit 0
elif [ $ERRORS -eq 0 ]; then
  echo -e "${YELLOW}⚠ WARNINGS: $WARNINGS${NC}"
  echo ""
  echo "You can proceed, but review warnings above"
  exit 0
else
  echo -e "${RED}✗ ERRORS: $ERRORS, WARNINGS: $WARNINGS${NC}"
  echo ""
  echo "Fix errors before building"
  exit 1
fi
