#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -f "$HOME/.cargo/env" ]; then
    # Xcode run scripts do not load the user's shell profile.
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if [ "${ZEDRA_SKIP_RUST_XCODE_BUILD:-}" = "1" ]; then
    echo "==> Skipping Rust build (ZEDRA_SKIP_RUST_XCODE_BUILD=1)"
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust with rustup, or set PATH so Xcode can find cargo." >&2
    exit 1
fi

configuration="${CONFIGURATION:-Debug}"
platform="${PLATFORM_NAME:-}"
effective_platform="${EFFECTIVE_PLATFORM_NAME:-}"
sdk_name="${SDK_NAME:-}"

if [ -z "$platform" ]; then
    platform="${effective_platform#-}"
fi

if [ -z "$platform" ]; then
    case "$sdk_name" in
        iphoneos*) platform="iphoneos" ;;
        iphonesimulator*) platform="iphonesimulator" ;;
    esac
fi

target_flag=""
case "$platform" in
    iphoneos)
        target_flag="--device"
        ;;
    iphonesimulator)
        target_flag="--sim"
        ;;
    *)
        echo "error: unsupported Xcode platform '$platform' (SDK_NAME=$sdk_name)" >&2
        exit 1
        ;;
esac

build_flags=()
case "$configuration" in
    Release*) build_flags+=("--release") ;;
esac

if [ -n "${ZEDRA_IOS_BUILD_FLAGS:-}" ]; then
    read -r -a extra_flags <<< "$ZEDRA_IOS_BUILD_FLAGS"
    build_flags+=("${extra_flags[@]}")
fi

echo "==> Building Rust for Xcode ($configuration, $platform, IPHONEOS_DEPLOYMENT_TARGET=${IPHONEOS_DEPLOYMENT_TARGET:-unset})"
cd "$REPO_ROOT"
command=("$REPO_ROOT/scripts/build-ios.sh")
if [ "${#build_flags[@]}" -gt 0 ]; then
    command+=("${build_flags[@]}")
fi
command+=("$target_flag")
exec "${command[@]}"
