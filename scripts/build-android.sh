#!/bin/bash

set -e

FEATURES=""
for arg in "$@"; do
    case "$arg" in
        --preview)
            FEATURES="--features preview"
            echo "Preview mode enabled"
            ;;
    esac
done

echo "Building Android libraries..."

# Build for different architectures
# Note: cargo-ndk will automatically find the NDK
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ./android/app/libs build -p zedra --lib --release $FEATURES

echo "Android libraries built successfully!"
echo "Libraries copied to ./android/app/libs/"
