#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

LIB_NAME="zedra"
FRAMEWORK_NAME="ZedraFFI"

# The crate has crate-type = ["cdylib", "staticlib"]. The cdylib is for Android
# (JNI .so) and will fail to link on iOS (OpenGL framework not available).
# The staticlib (.a) is produced before the cdylib linking step, so we allow
# the cargo build to "fail" and just check the .a exists.

echo "==> Building for iOS device (aarch64-apple-ios)..."
cargo build --target aarch64-apple-ios --release --features ios-platform -p zedra || true
if [ ! -f "target/aarch64-apple-ios/release/lib${LIB_NAME}.a" ]; then
    echo "ERROR: staticlib not produced for device"
    exit 1
fi

echo "==> Building for iOS simulator (aarch64-apple-ios-sim)..."
cargo build --target aarch64-apple-ios-sim --release --features ios-platform -p zedra || true
if [ ! -f "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" ]; then
    echo "ERROR: staticlib not produced for simulator"
    exit 1
fi

echo "==> Creating XCFramework..."
rm -rf "${FRAMEWORK_NAME}.xcframework"
xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/lib${LIB_NAME}.a" \
    -headers include/ \
    -library "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
    -headers include/ \
    -output "${FRAMEWORK_NAME}.xcframework"

echo "==> Done! Created ${FRAMEWORK_NAME}.xcframework"
