#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

PROJECT="ios/Zedra.xcodeproj"
SCHEME="Zedra"
BUNDLE_ID="dev.zedra.app"

usage() {
    echo "Usage: $0 [sim|device] [--preview] [--debug] [--device-id <UDID>]"
    echo ""
    echo "  sim      Build and run on iOS Simulator (default)"
    echo "  device   Build and install on connected device"
    echo ""
    echo "  --preview             Enable preview feature flag"
    echo "  --debug               Use debug profile (faster build, no optimizations)"
    echo "  --device-id <UDID>    Target a specific device by UDID (overrides auto-detect)"
    echo ""
    echo "Examples:"
    echo "  $0                                        # run on simulator (release)"
    echo "  $0 sim                                    # run on simulator (release)"
    echo "  $0 device                                 # install on first connected device"
    echo "  $0 device --device-id 00008140-001234     # install on specific device"
    echo "  $0 device --preview                       # install with preview features"
    echo "  $0 device --debug                         # install debug build"
    exit 1
}

# Generate Xcode project from project.yml if needed
generate_project() {
    if ! command -v xcodegen &>/dev/null; then
        echo "Error: xcodegen not found. Install with: brew install xcodegen"
        exit 1
    fi

    echo "==> Generating Xcode project..."
    cd ios && xcodegen generate && cd ..
}

MODE="${1:-sim}"
BUILD_FLAGS=""
XCODE_CONFIGURATION="Debug"
FORCED_DEVICE_ID=""

args=("$@")
i=0
while [ $i -lt ${#args[@]} ]; do
    case "${args[$i]}" in
        --preview)
            BUILD_FLAGS="$BUILD_FLAGS --preview"
            ;;
        --debug)
            BUILD_FLAGS="$BUILD_FLAGS --debug"
            XCODE_CONFIGURATION="Debug"
            ;;
        --device-id)
            i=$((i + 1))
            FORCED_DEVICE_ID="${args[$i]}"
            ;;
    esac
    i=$((i + 1))
done

case "$MODE" in
    sim)
        # Pick first booted simulator, or boot one if none running
        BOOTED_ID=$(xcrun simctl list devices booted -j | python3 -c "
import json, sys
data = json.load(sys.stdin)
for runtime, devices in data['devices'].items():
    for d in devices:
        if d['state'] == 'Booted':
            print(d['udid'])
            sys.exit(0)
" 2>/dev/null || true)

        if [ -z "$BOOTED_ID" ]; then
            SIM_ID=$(xcrun simctl list devices available -j | python3 -c "
import json, sys
data = json.load(sys.stdin)
for runtime, devices in sorted(data['devices'].items(), reverse=True):
    if 'iOS' not in runtime: continue
    for d in devices:
        if 'iPhone' in d['name'] and d['isAvailable']:
            print(d['udid'])
            sys.exit(0)
" 2>/dev/null)
            echo "==> Booting simulator..."
            xcrun simctl boot "$SIM_ID"
            BOOTED_ID="$SIM_ID"
        fi

        SIM_NAME=$(xcrun simctl list devices -j | python3 -c "
import json, sys
data = json.load(sys.stdin)
uid = '$BOOTED_ID'
for runtime, devices in data['devices'].items():
    for d in devices:
        if d['udid'] == uid:
            print(d['name'])
            sys.exit(0)
" 2>/dev/null)
        echo "==> Target: $SIM_NAME ($BOOTED_ID)"

        # Build Rust libraries
        echo "==> Building Rust for iOS..."
        ./scripts/build-ios.sh $BUILD_FLAGS

        # Generate Xcode project
        generate_project

        # Build app
        echo "==> Building app..."
        xcodebuild build \
            -project "$PROJECT" \
            -scheme "$SCHEME" \
            -configuration "$XCODE_CONFIGURATION" \
            -destination "id=$BOOTED_ID" \
            -quiet

        # Find the built .app
        APP_PATH=$(find ~/Library/Developer/Xcode/DerivedData/Zedra-*/Build/Products/${XCODE_CONFIGURATION}-iphonesimulator -name "Zedra.app" -type d 2>/dev/null | head -1)

        if [ -z "$APP_PATH" ]; then
            echo "Error: Could not find built .app"
            exit 1
        fi

        # Install and launch
        echo "==> Installing..."
        xcrun simctl install "$BOOTED_ID" "$APP_PATH"
        echo "==> Launching..."
        open -a Simulator
        xcrun simctl launch "$BOOTED_ID" "$BUNDLE_ID"
        echo "==> Running on $SIM_NAME"
        ;;

    device)
        if [ -n "$FORCED_DEVICE_ID" ]; then
            # Resolve name and OS from the forced UDID
            DEVICE_LINE=$(xcrun xctrace list devices 2>&1 | grep "$FORCED_DEVICE_ID" | head -1)
            if [ -z "$DEVICE_LINE" ]; then
                echo "Error: Device with UDID '$FORCED_DEVICE_ID' not found."
                echo ""
                echo "Available devices:"
                xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+'
                exit 1
            fi
            DEVICE_ID="$FORCED_DEVICE_ID"
        else
            # Auto-detect first connected device
            DEVICE_LINE=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+' | head -1)
            DEVICE_ID=$(echo "$DEVICE_LINE" | grep -oE '[0-9A-F]{8}-[0-9A-F]{16}' || true)

            if [ -z "$DEVICE_ID" ]; then
                echo "Error: No connected iOS device found."
                echo ""
                echo "Available devices:"
                xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+'
                exit 1
            fi
        fi

        # Detect device OS version and use it as the deployment target so the
        # Rust build flags and Xcode build settings both match the actual device.
        DEVICE_OS=$(echo "$DEVICE_LINE" | grep -oE '\([0-9]+\.[0-9]+(\.[0-9]+)?\)' | head -1 | tr -d '()')
        export IPHONEOS_DEPLOYMENT_TARGET="${DEVICE_OS:-16.0}"
        DEVICE_NAME=$(echo "$DEVICE_LINE" | sed 's/ (.*//')
        echo "==> Target: $DEVICE_NAME ($DEVICE_ID) — iOS $IPHONEOS_DEPLOYMENT_TARGET"

        # Build Rust libraries
        echo "==> Building Rust for iOS..."
        ./scripts/build-ios.sh $BUILD_FLAGS

        # Generate Xcode project
        generate_project

        # Build app for device
        echo "==> Building app..."
        xcodebuild build \
            -project "$PROJECT" \
            -scheme "$SCHEME" \
            -configuration "$XCODE_CONFIGURATION" \
            -destination "id=$DEVICE_ID" \
            -allowProvisioningUpdates \
            IPHONEOS_DEPLOYMENT_TARGET="$IPHONEOS_DEPLOYMENT_TARGET" \
            -quiet

        # Find the built .app
        APP_PATH=$(find ~/Library/Developer/Xcode/DerivedData/Zedra-*/Build/Products/${XCODE_CONFIGURATION}-iphoneos -name "Zedra.app" -type d 2>/dev/null | head -1)

        if [ -z "$APP_PATH" ]; then
            echo "Error: Could not find built .app"
            exit 1
        fi

        # Install on device
        echo "==> Installing on $DEVICE_NAME..."
        xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH"

        echo "==> Launching..."
        xcrun devicectl device process launch --device "$DEVICE_ID" "$BUNDLE_ID"

        echo "==> Running on $DEVICE_NAME"
        ;;

    *)
        usage
        ;;
esac
