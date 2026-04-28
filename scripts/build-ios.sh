#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

LIB_NAME="zedra"
FRAMEWORK_NAME="ZedraFFI"

FEATURES="--features ios-platform"
PROFILE=""
PROFILE_DIR="debug"
RELEASE=false
DEBUG_FEATURES=false
PREVIEW=false
DEBUG_TELEMETRY=false
DEBUG_LOGS=false
# By default build both targets so the xcframework works on sim and device.
# Pass --sim or --device to only build one (faster incremental builds).
BUILD_SIM=true
BUILD_DEVICE=true

for arg in "$@"; do
    case "$arg" in
        --preview)
            FEATURES="$FEATURES,preview"
            PREVIEW=true
            ;;
        --debug-telemetry)
            FEATURES="$FEATURES,debug-telemetry"
            DEBUG_FEATURES=true
            DEBUG_TELEMETRY=true
            ;;
        --debug)
            FEATURES="$FEATURES,debug-logs"
            DEBUG_FEATURES=true
            DEBUG_LOGS=true
            ;;
        --release)
            PROFILE="--release"
            PROFILE_DIR="release"
            RELEASE=true
            ;;
        --sim)
            BUILD_DEVICE=false
            ;;
        --device)
            BUILD_SIM=false
            ;;
    esac
done

if [ "$RELEASE" = true ] && [ "$DEBUG_FEATURES" = true ]; then
    echo "ERROR: iOS release builds cannot enable --debug or --debug-telemetry." >&2
    exit 1
fi

[ "$PREVIEW" = true ] && echo "Preview mode enabled"
[ "$DEBUG_TELEMETRY" = true ] && echo "Debug telemetry enabled (events logged to console)"
[ "$DEBUG_LOGS" = true ] && echo "Debug logs enabled (verbose iroh/quinn output)"
[ "$RELEASE" = true ] && echo "Release mode enabled"

# Use the deployment target passed in from run-ios.sh (which detects the
# connected device's OS version), or fall back to 16.0 when called standalone.
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"
echo "==> Deployment target: iOS $IPHONEOS_DEPLOYMENT_TARGET"

# Build the static library explicitly and fail hard on cargo errors.
# Remove any stale output first so the XCFramework step cannot accidentally
# package an older libzedra.a from a previous successful build.

if [ "$BUILD_DEVICE" = true ]; then
    echo "==> Building for iOS device (aarch64-apple-ios)..."
    rm -f "target/aarch64-apple-ios/${PROFILE_DIR}/lib${LIB_NAME}.a"
    cargo build --target aarch64-apple-ios $PROFILE $FEATURES -p zedra
    if [ ! -f "target/aarch64-apple-ios/${PROFILE_DIR}/lib${LIB_NAME}.a" ]; then
        echo "ERROR: staticlib not produced for device"
        exit 1
    fi
fi

if [ "$BUILD_SIM" = true ]; then
    echo "==> Building for iOS simulator (aarch64-apple-ios-sim)..."
    rm -f "target/aarch64-apple-ios-sim/${PROFILE_DIR}/lib${LIB_NAME}.a"
    cargo build --target aarch64-apple-ios-sim $PROFILE $FEATURES -p zedra
    if [ ! -f "target/aarch64-apple-ios-sim/${PROFILE_DIR}/lib${LIB_NAME}.a" ]; then
        echo "ERROR: staticlib not produced for simulator"
        exit 1
    fi
fi

echo "==> XCFramework..."
OUT="ios/${FRAMEWORK_NAME}.xcframework"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

XCF_ARGS=()
if [ "$BUILD_DEVICE" = true ]; then
    XCF_ARGS+=(-library "target/aarch64-apple-ios/${PROFILE_DIR}/lib${LIB_NAME}.a" -headers include/)
fi
if [ "$BUILD_SIM" = true ]; then
    XCF_ARGS+=(-library "target/aarch64-apple-ios-sim/${PROFILE_DIR}/lib${LIB_NAME}.a" -headers include/)
fi

NEW="$TMP/${FRAMEWORK_NAME}.xcframework"
XCODEBUILD_STDERR="$TMP/xcodebuild.stderr"
if ! xcodebuild -create-xcframework "${XCF_ARGS[@]}" -output "$NEW" 2>"$XCODEBUILD_STDERR"; then
    cat "$XCODEBUILD_STDERR" >&2
    exit 1
fi

# Full replace when missing or both slices built; else swap only built slice(s) into existing bundle.
if [ ! -d "$OUT" ] || { [ "$BUILD_DEVICE" = true ] && [ "$BUILD_SIM" = true ]; }; then
    rm -rf "$OUT"
    mv "$NEW" "$OUT"
else
    [ "$BUILD_DEVICE" = true ] && { rm -rf "$OUT/ios-arm64" && cp -R "$NEW/ios-arm64" "$OUT/"; }
    [ "$BUILD_SIM" = true ] && { rm -rf "$OUT/ios-arm64-simulator" && cp -R "$NEW/ios-arm64-simulator" "$OUT/"; }
    SLICES=()
    [ "$BUILD_DEVICE" = true ] && SLICES+=(ios-arm64)
    [ "$BUILD_SIM" = true ] && SLICES+=(ios-arm64-simulator)
    python3 - "$OUT/Info.plist" "$NEW/Info.plist" "${SLICES[@]}" <<'PY'
import plistlib, sys
from pathlib import Path

o, n = Path(sys.argv[1]), Path(sys.argv[2])
replace = set(sys.argv[3:])
new = plistlib.load(n.open("rb"))["AvailableLibraries"]
p = plistlib.load(o.open("rb"))
kept = [x for x in p.get("AvailableLibraries", []) if x.get("LibraryIdentifier") not in replace]
by_id = {}
for lib in kept + new:
    i = lib.get("LibraryIdentifier")
    if i:
        by_id[i] = lib
p["AvailableLibraries"] = sorted(
    by_id.values(), key=lambda x: x.get("LibraryIdentifier") != "ios-arm64"
)
with o.open("wb") as f:
    plistlib.dump(p, f, fmt=plistlib.FMT_XML)
PY
fi

echo "==> Done: $OUT"
