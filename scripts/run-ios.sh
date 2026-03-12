#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

PROJECT="ios/Zedra.xcodeproj"
WORKSPACE="ios/Zedra.xcworkspace"
SCHEME="Zedra"
BUNDLE_ID="dev.zedra.app"

usage() {
    echo "Usage: $0 [sim|device] [--preview] [--debug] [--device-id <UDID>] [--select-device]"
    echo ""
    echo "  sim      Build and run on iOS Simulator (default)"
    echo "  device   Build and install on connected device"
    echo ""
    echo "  --preview               Enable preview feature flag"
    echo "  --debug                 Use debug profile (faster build, no optimizations)"
    echo "  --device-id <UDID>      Target a specific device by UDID (skips selection)"
    echo "  --select-device         Ignore saved device preference and re-prompt"
    echo ""
    echo "Examples:"
    echo "  $0                                        # run on simulator (release)"
    echo "  $0 sim                                    # run on simulator (release)"
    echo "  $0 device                                 # install on saved/selected device"
    echo "  $0 device --select-device                 # re-prompt for device"
    echo "  $0 device --device-id 00008140-001234     # install on specific device"
    echo "  $0 device --preview                       # install with preview features"
    echo "  $0 device --debug                         # install debug build"
    exit 1
}

# Generate Xcode project from project.yml, then run pod install if a Podfile exists
generate_project() {
    if ! command -v xcodegen &>/dev/null; then
        echo "Error: xcodegen not found. Install with: brew install xcodegen"
        exit 1
    fi

    echo "==> Generating Xcode project..."
    cd ios && xcodegen generate

    if [ -f "Podfile" ]; then
        echo "==> Running pod install..."
        pod install --silent
    fi

    cd ..
}

# Returns the right xcodebuild target flags: workspace if available, else project
xcode_target_flags() {
    if [ -d "$WORKSPACE" ]; then
        echo "-workspace $WORKSPACE"
    else
        echo "-project $PROJECT"
    fi
}

MODE="${1:-sim}"
BUILD_FLAGS=""
XCODE_CONFIGURATION="Debug"
FORCED_DEVICE_ID=""
SELECT_DEVICE=false
PREF_FILE="/tmp/zedra-ios-device-$PPID"

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
        --select-device)
            SELECT_DEVICE=true
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
            $(xcode_target_flags) \
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
        DEVICE_ID=""
        DEVICE_LINE=""

        if [ -n "$FORCED_DEVICE_ID" ]; then
            # Explicit --device-id takes priority, resolve name/OS from it
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
            # Check session-scoped pref file (shared with ios-log.sh)
            if [ "$SELECT_DEVICE" = false ] && [ -f "$PREF_FILE" ]; then
                IFS='|' read -r DEVICE_ID DEVICE_NAME_SAVED < "$PREF_FILE"
                DEVICE_LINE=$(xcrun xctrace list devices 2>&1 | grep "$DEVICE_ID" | head -1)
                if [ -z "$DEVICE_LINE" ]; then
                    echo "Warning: Saved device $DEVICE_ID not found, re-prompting..."
                    DEVICE_ID=""
                else
                    echo "==> Using saved device: $DEVICE_NAME_SAVED ($DEVICE_ID)"
                fi
            fi

            # Interactive selection if no device yet
            if [ -z "$DEVICE_ID" ]; then
                DEVICE_LINES=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\(\d+\.\d+' | grep -v Simulator)
                if [ -z "$DEVICE_LINES" ]; then
                    echo "Error: No connected iOS device found." >&2
                    exit 1
                fi

                echo ""
                echo "Connected iOS devices:"
                i=1
                while IFS= read -r line; do
                    echo "  $i. $line"
                    i=$((i + 1))
                done <<< "$DEVICE_LINES"
                echo ""

                COUNT=$(echo "$DEVICE_LINES" | wc -l | tr -d ' ')
                if [ "$COUNT" -eq 1 ]; then
                    CHOICE=1
                    echo "==> Auto-selecting only device."
                else
                    read -rp "Select device [1-$COUNT]: " CHOICE
                fi

                SELECTED_LINE=$(echo "$DEVICE_LINES" | sed -n "${CHOICE}p")
                if [ -z "$SELECTED_LINE" ]; then
                    echo "Error: Invalid selection." >&2
                    exit 1
                fi

                DEVICE_ID=$(echo "$SELECTED_LINE" | grep -oE '\([A-F0-9a-f-]{25,}\)' | tail -1 | tr -d '()')
                DEVICE_NAME_SAVED=$(echo "$SELECTED_LINE" | sed 's/ ([^)]*) ([^)]*)$//' | sed 's/ ([^)]*)$//')
                DEVICE_LINE="$SELECTED_LINE"
                echo "$DEVICE_ID|$DEVICE_NAME_SAVED" > "$PREF_FILE"
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
            $(xcode_target_flags) \
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
