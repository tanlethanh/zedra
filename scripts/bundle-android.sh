#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
    echo "Usage: $0 [--release|--debug] [--preview] [--no-telemetry] [--target <ABI>]"
    echo ""
    echo "Builds the Android Rust library, APK, and Android App Bundle."
    echo ""
    echo "  --release       Build the release variant (default)"
    echo "  --debug         Build the debug variant and Rust debug profile"
    echo "  --preview       Enable the Rust preview feature"
    echo "  --no-telemetry  Compile Firebase Analytics and Crashlytics out"
    echo "  --target <ABI>  Build a single Rust/NDK ABI (default: all configured ABIs)"
    echo ""
    echo "Examples:"
    echo "  $0"
    echo "  $0 --debug --target arm64-v8a"
    exit "${1:-1}"
}

REQUESTED_RELEASE=false
REQUESTED_DEBUG=false
TARGET_ABI=""
NO_TELEMETRY=false
RUST_FLAGS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)
            usage 0
            ;;
        --release)
            REQUESTED_RELEASE=true
            ;;
        --debug)
            REQUESTED_DEBUG=true
            ;;
        --preview)
            RUST_FLAGS+=(--preview)
            ;;
        --no-telemetry)
            NO_TELEMETRY=true
            RUST_FLAGS+=(--no-telemetry)
            ;;
        --target)
            shift
            if [ $# -eq 0 ]; then
                echo "Error: --target requires an ABI." >&2
                exit 1
            fi
            TARGET_ABI="$1"
            ;;
        --target=*)
            TARGET_ABI="${1#--target=}"
            ;;
        *)
            echo "Error: unknown argument '$1'." >&2
            usage
            ;;
    esac
    shift
done

if [ "$REQUESTED_RELEASE" = true ] && [ "$REQUESTED_DEBUG" = true ]; then
    echo "Error: --release cannot be combined with --debug." >&2
    exit 1
fi

BUILD_TYPE="release"
VARIANT="Release"
if [ "$REQUESTED_DEBUG" = true ]; then
    BUILD_TYPE="debug"
    VARIANT="Debug"
    RUST_FLAGS+=(--debug)
fi

if [ -n "$TARGET_ABI" ]; then
    RUST_FLAGS+=("--target=$TARGET_ABI")
fi

find_output() {
    local dir="$1"
    local pattern="$2"
    find "$dir" -name "$pattern" -type f 2>/dev/null | sort | head -1
}

format_size() {
    local path="$1"
    local bytes
    bytes="$(wc -c < "$path" | tr -d '[:space:]')"
    awk -v bytes="$bytes" 'BEGIN {
        split("B KiB MiB GiB", units, " ")
        value = bytes
        unit = 1
        while (value >= 1024 && unit < 4) {
            value /= 1024
            unit += 1
        }
        if (unit == 1) {
            printf "%d %s", value, units[unit]
        } else {
            printf "%.1f %s", value, units[unit]
        }
    }'
}

echo "==> Building Android Rust library ($BUILD_TYPE)..."
if [ "${#RUST_FLAGS[@]}" -gt 0 ]; then
    ./scripts/build-android.sh "${RUST_FLAGS[@]}"
else
    ./scripts/build-android.sh
fi

echo "==> Building Android APK and App Bundle ($BUILD_TYPE)..."
GRADLE_FLAGS=()
[ "$NO_TELEMETRY" = true ] && GRADLE_FLAGS+=(-PnoTelemetry)
(
    cd android
    ./gradlew "${GRADLE_FLAGS[@]+"${GRADLE_FLAGS[@]}"}" ":assemble${VARIANT}" ":bundle${VARIANT}" -x "buildRustLib${VARIANT}"
)

APK_PATH="$(find_output "android/build/outputs/apk/$BUILD_TYPE" "*.apk")"
AAB_PATH="$(find_output "android/build/outputs/bundle/$BUILD_TYPE" "*.aab")"

if [ -z "$APK_PATH" ]; then
    echo "Error: could not find built APK under android/build/outputs/apk/$BUILD_TYPE." >&2
    exit 1
fi

if [ -z "$AAB_PATH" ]; then
    echo "Error: could not find built AAB under android/build/outputs/bundle/$BUILD_TYPE." >&2
    exit 1
fi

echo "==> APK: $APK_PATH ($(format_size "$APK_PATH"))"
echo "==> AAB: $AAB_PATH ($(format_size "$AAB_PATH"))"
