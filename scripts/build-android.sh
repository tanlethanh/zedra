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
cargo ndk $TARGETS -o ./android/app/libs build -p zedra --lib $PROFILE $FEATURES

echo "Android libraries built successfully!"
echo "Libraries copied to ./android/app/libs/"
