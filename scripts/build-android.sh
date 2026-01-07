#!/bin/bash

# Set environment variables
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/27.1.12297006  # Adjust path
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH

# Build for different architectures
cargo ndk -t armeabi-v7a -t arm64-v8a -t x86 -t x86_64 -o ./android/app/libs build --release

echo "Android libraries built successfully!"
