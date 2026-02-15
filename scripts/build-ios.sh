#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

LIB_NAME="zedra"
FRAMEWORK_NAME="ZedraFFI"

echo "==> Building for iOS device (aarch64-apple-ios)..."
cargo build --target aarch64-apple-ios --release --features ios-platform -p zedra

echo "==> Building for iOS simulator (aarch64-apple-ios-sim)..."
cargo build --target aarch64-apple-ios-sim --release --features ios-platform -p zedra

echo "==> Creating XCFramework..."
rm -rf "${FRAMEWORK_NAME}.xcframework"
xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/lib${LIB_NAME}.a" \
    -headers include/ \
    -library "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
    -headers include/ \
    -output "${FRAMEWORK_NAME}.xcframework"

echo "==> Done! Created ${FRAMEWORK_NAME}.xcframework"
