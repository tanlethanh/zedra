#!/bin/bash

set -e

FEATURES="--features android-platform"
PROFILE="--release"
TARGETS=""
OUTPUT_DIR="./android/app/libs"
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/zedra-android-libs.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT

publish_zedra_libs() {
    rm -rf "$OUTPUT_DIR"
    mkdir -p "$OUTPUT_DIR"

    local copied=0
    while IFS= read -r lib_path; do
        local abi
        abi="$(basename "$(dirname "$lib_path")")"
        mkdir -p "$OUTPUT_DIR/$abi"
        cp "$lib_path" "$OUTPUT_DIR/$abi/libzedra.so"
        copied=$((copied + 1))
    done < <(find "$STAGING_DIR" -type f -name 'libzedra.so' | sort)

    if [ "$copied" -eq 0 ]; then
        echo "Error: cargo-ndk did not produce libzedra.so." >&2
        exit 1
    fi
}

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

# Release builds ship arm64 only; debug builds keep all development ABIs unless
# a target is specified explicitly.
if [ -z "$TARGETS" ]; then
    if [ "$PROFILE" = "--release" ]; then
        TARGETS="-t arm64-v8a"
    else
        TARGETS="-t arm64-v8a -t armeabi-v7a -t x86_64"
    fi
fi

echo "Building Android libraries (targets:$TARGETS)..."

# Build for specified architectures
# Note: cargo-ndk will automatically find the NDK
if [ "$PROFILE" = "--release" ]; then
    CARGO_PROFILE_RELEASE_STRIP=false cargo ndk $TARGETS -o "$STAGING_DIR" build -p zedra --lib $PROFILE $FEATURES
    # Gradle packages every .so under app/libs, so publish only the app entry
    # library and keep Cargo dependency artifacts out of release bundles.
    publish_zedra_libs
    rm -rf ./android/build/zedra-unstripped-libs/release
    mkdir -p ./android/build/zedra-unstripped-libs/release
    cp -R ./android/app/libs/. ./android/build/zedra-unstripped-libs/release/
else
    cargo ndk $TARGETS -o "$STAGING_DIR" build -p zedra --lib $PROFILE $FEATURES
    publish_zedra_libs
fi

echo "Android libraries built successfully!"
echo "Libraries copied to ./android/app/libs/"
