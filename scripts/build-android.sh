#!/bin/bash

set -e

FEATURES="--features android-platform"
PROFILE="--release"
TARGETS=""
for arg in "$@"; do
    case "$arg" in
        --preview)
            FEATURES="$FEATURES,preview"
            echo "Preview mode enabled"
            ;;
        --debug)
            PROFILE=""
            echo "Debug mode enabled"
            ;;
        --debug-telemetry)
            FEATURES="$FEATURES,debug-telemetry"
            echo "Debug telemetry enabled (events logged to logcat)"
            ;;
        --devtool)
            FEATURES="$FEATURES,devtool"
            echo "Devtool enabled: in-app HTTP server on 127.0.0.1:9777"
            ;;
        --target=*)
            TARGETS="$TARGETS -t ${arg#--target=}"
            ;;
    esac
done

# Default to all architectures if no target specified
if [ -z "$TARGETS" ]; then
    TARGETS="-t arm64-v8a -t armeabi-v7a -t x86_64"
fi

echo "Building Android libraries (targets:$TARGETS)..."

# Build for specified architectures
# Note: cargo-ndk will automatically find the NDK
if [ "$PROFILE" = "--release" ]; then
    CARGO_PROFILE_RELEASE_STRIP=false cargo ndk $TARGETS -o ./android/app/libs build -p zedra --lib $PROFILE $FEATURES
    rm -rf ./android/build/zedra-unstripped-libs/release
    mkdir -p ./android/build/zedra-unstripped-libs/release
    cp -R ./android/app/libs/. ./android/build/zedra-unstripped-libs/release/
else
    cargo ndk $TARGETS -o ./android/app/libs build -p zedra --lib $PROFILE $FEATURES
fi

echo "Android libraries built successfully!"
echo "Libraries copied to ./android/app/libs/"
