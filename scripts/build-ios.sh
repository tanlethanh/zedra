#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

LIB_NAME="zedra"
FRAMEWORK_NAME="ZedraFFI"

FEATURES="--features ios-platform"
PROFILE=""
PROFILE_DIR="debug"
# By default build both targets so the xcframework works on sim and device.
# Pass --sim or --device to only build one (faster incremental builds).
BUILD_SIM=true
BUILD_DEVICE=true

for arg in "$@"; do
    case "$arg" in
        --preview)
            FEATURES="$FEATURES,preview"
            echo "Preview mode enabled"
            ;;
        --release)
            PROFILE="--release"
            PROFILE_DIR="release"
            echo "Release mode enabled"
            ;;
        --sim)
            BUILD_DEVICE=false
            ;;
        --device)
            BUILD_SIM=false
            ;;
    esac
done

# Use the deployment target passed in from run-ios.sh (which detects the
# connected device's OS version), or fall back to 16.0 when called standalone.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"
echo "==> Deployment target: iOS $IPHONEOS_DEPLOYMENT_TARGET"

# The crate has crate-type = ["cdylib", "staticlib"]. The cdylib is for Android
# (JNI .so) and will fail to link on iOS (OpenGL framework not available).
# The staticlib (.a) is produced before the cdylib linking step, so we allow
# the cargo build to "fail" and just check the .a exists.

if [ "$BUILD_DEVICE" = true ]; then
    echo "==> Building for iOS device (aarch64-apple-ios)..."
    cargo build --target aarch64-apple-ios $PROFILE $FEATURES -p zedra || true
    if [ ! -f "target/aarch64-apple-ios/${PROFILE_DIR}/lib${LIB_NAME}.a" ]; then
        echo "ERROR: staticlib not produced for device"
        exit 1
    fi
fi

if [ "$BUILD_SIM" = true ]; then
    echo "==> Building for iOS simulator (aarch64-apple-ios-sim)..."
    cargo build --target aarch64-apple-ios-sim $PROFILE $FEATURES -p zedra || true
    if [ ! -f "target/aarch64-apple-ios-sim/${PROFILE_DIR}/lib${LIB_NAME}.a" ]; then
        echo "ERROR: staticlib not produced for simulator"
        exit 1
    fi
fi

echo "==> Creating XCFramework..."
rm -rf "ios/${FRAMEWORK_NAME}.xcframework"
XCF_ARGS=()
if [ "$BUILD_DEVICE" = true ]; then
    XCF_ARGS+=(-library "target/aarch64-apple-ios/${PROFILE_DIR}/lib${LIB_NAME}.a" -headers include/)
fi
if [ "$BUILD_SIM" = true ]; then
    XCF_ARGS+=(-library "target/aarch64-apple-ios-sim/${PROFILE_DIR}/lib${LIB_NAME}.a" -headers include/)
fi
xcodebuild -create-xcframework "${XCF_ARGS[@]}" -output "ios/${FRAMEWORK_NAME}.xcframework"

echo "==> Done! Created ios/${FRAMEWORK_NAME}.xcframework"
