#!/bin/bash

set -e

echo "Building Android libraries..."

# Build for different architectures
# Note: cargo-ndk will automatically find the NDK
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ./android/app/libs build -p zedra --lib --release

echo "Android libraries built successfully!"
echo "Libraries copied to ./android/app/libs/"
