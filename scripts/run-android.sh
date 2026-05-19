#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

APP_ID_DEBUG="dev.zedra.app.debug"
APP_ID_RELEASE="dev.zedra.app"
ACTIVITY_CLASS="dev.zedra.app.MainActivity"
PREF_FILE="/tmp/zedra-android-device-$PPID"

usage() {
    echo "Usage: $0 [device|emulator] [--no-build] [--release] [--preview] [--debug] [--debug-telemetry] [--device-id <SERIAL>] [--select-device] [--launch-url <URL>] [--target <ABI>]"
    echo ""
    echo "  device    Build, install, and run on a connected Android target (default)"
    echo "  emulator  Restrict target selection to Android emulators"
    echo ""
    echo "  --no-build              Skip build/install and launch the currently installed app"
    echo "  --release               Build the Android release variant (default is debug)"
    echo "  --preview               Enable preview feature flag for Rust"
    echo "  --debug                 Use Rust debug profile (default unless --release is passed)"
    echo "  --debug-telemetry       Enable debug telemetry feature flag"
    echo "  --devtool               Enable in-app HTTP devtool (127.0.0.1:9777)"
    echo "  --device-id <SERIAL>    Target a specific adb serial (skips selection)"
    echo "  --select-device         Ignore saved device preference and re-prompt"
    echo "  --launch-url <URL>      Open the app with a deep link URL (e.g. zedra://...)"
    echo "  --target <ABI>          Rust/NDK ABI to build (default: detected from target)"
    echo ""
    echo "Examples:"
    echo "  $0"
    echo "  $0 device --preview"
    echo "  $0 emulator --select-device"
    echo "  $0 --device-id emulator-5554 --no-build"
    echo "  $0 --target arm64-v8a --launch-url 'zedra://connect?ticket=...'"
    exit "${1:-1}"
}

resolve_adb() {
    if [ -n "${ANDROID_HOME:-}" ] && [ -x "$ANDROID_HOME/platform-tools/adb" ]; then
        echo "$ANDROID_HOME/platform-tools/adb"
        return
    fi

    if [ -n "${ANDROID_SDK_ROOT:-}" ] && [ -x "$ANDROID_SDK_ROOT/platform-tools/adb" ]; then
        echo "$ANDROID_SDK_ROOT/platform-tools/adb"
        return
    fi

    if command -v adb >/dev/null 2>&1; then
        command -v adb
        return
    fi

    echo "Error: adb not found. Install Android Platform Tools or set ANDROID_HOME." >&2
    exit 1
}

MODE="device"
if [ $# -gt 0 ] && [[ "$1" != --* ]]; then
    MODE="$1"
    shift
fi

REQUESTED_RELEASE=false
REQUESTED_DEBUG=false
REQUESTED_DEBUG_TELEMETRY=false
NO_BUILD=false
SELECT_DEVICE=false
FORCED_DEVICE_ID=""
LAUNCH_URL=""
TARGET_ABI=""
RUST_FLAGS=()
GRADLE_FLAGS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)
            usage 0
            ;;
        --preview)
            RUST_FLAGS+=(--preview)
            ;;
        --debug)
            REQUESTED_DEBUG=true
            ;;
        --release)
            REQUESTED_RELEASE=true
            ;;
        --debug-telemetry)
            REQUESTED_DEBUG_TELEMETRY=true
            RUST_FLAGS+=(--debug-telemetry)
            ;;
        --devtool)
            RUST_FLAGS+=(--devtool)
            ;;
        --device-id)
            shift
            if [ $# -eq 0 ]; then
                echo "Error: --device-id requires a serial." >&2
                exit 1
            fi
            FORCED_DEVICE_ID="$1"
            ;;
        --select-device)
            SELECT_DEVICE=true
            ;;
        --launch-url)
            shift
            if [ $# -eq 0 ]; then
                echo "Error: --launch-url requires a URL." >&2
                exit 1
            fi
            LAUNCH_URL="$1"
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
        --no-build)
            NO_BUILD=true
            ;;
        *)
            echo "Error: unknown argument '$1'." >&2
            usage
            ;;
    esac
    shift
done

case "$MODE" in
    device|emulator) ;;
    *)
        echo "Error: unknown mode '$MODE'." >&2
        usage
        ;;
esac

if [ "$REQUESTED_RELEASE" = true ] && { [ "$REQUESTED_DEBUG" = true ] || [ "$REQUESTED_DEBUG_TELEMETRY" = true ]; }; then
    echo "Error: --release cannot be combined with --debug or --debug-telemetry for Android builds." >&2
    exit 1
fi

BUILD_TYPE="debug"
GRADLE_TASK="assembleDebug"
RUST_TASK="buildRustLibDebug"
APP_ID="$APP_ID_DEBUG"
if [ "$REQUESTED_RELEASE" = true ]; then
    BUILD_TYPE="release"
    GRADLE_TASK="assembleRelease"
    RUST_TASK="buildRustLibRelease"
    APP_ID="$APP_ID_RELEASE"
    # Local release installs need a signature, but plain assembleRelease stays unsigned.
    GRADLE_FLAGS+=(-Pzedra.localReleaseSigning=true)
else
    RUST_FLAGS+=(--debug)
fi
MAIN_ACTIVITY="$APP_ID/$ACTIVITY_CLASS"

ADB="$(resolve_adb)"

device_lines() {
    "$ADB" devices -l | awk 'NR > 1 && $2 == "device" { print }'
}

filtered_device_lines() {
    local lines
    lines="$(device_lines)"
    if [ "$MODE" = "emulator" ]; then
        echo "$lines" | awk '$1 ~ /^emulator-/ { print }'
    else
        echo "$lines"
    fi
}

device_name_from_line() {
    local line="$1"
    local model
    model="$(echo "$line" | tr ' ' '\n' | awk -F: '$1 == "model" { print $2; exit }' | tr '_' ' ')"
    if [ -n "$model" ]; then
        echo "$model"
    else
        echo "$line" | awk '{ print $1 }'
    fi
}

select_device() {
    local lines line count choice selected_id selected_name

    if [ -n "$FORCED_DEVICE_ID" ]; then
        line="$(device_lines | awk -v id="$FORCED_DEVICE_ID" '$1 == id { print; exit }')"
        if [ -z "$line" ]; then
            echo "Error: Android device '$FORCED_DEVICE_ID' is not connected or not authorized." >&2
            echo ""
            echo "Available devices:"
            filtered_device_lines
            exit 1
        fi
        DEVICE_ID="$FORCED_DEVICE_ID"
        DEVICE_NAME="$(device_name_from_line "$line")"
        return
    fi

    if [ "$SELECT_DEVICE" = false ] && [ -f "$PREF_FILE" ]; then
        IFS='|' read -r selected_id selected_name < "$PREF_FILE"
        line="$(filtered_device_lines | awk -v id="$selected_id" '$1 == id { print; exit }')"
        if [ -n "$line" ]; then
            DEVICE_ID="$selected_id"
            DEVICE_NAME="$selected_name"
            echo "==> Using saved Android target: $DEVICE_NAME ($DEVICE_ID)"
            return
        fi
    fi

    lines="$(filtered_device_lines)"
    if [ -z "$lines" ]; then
        if [ "$MODE" = "emulator" ]; then
            echo "Error: No running Android emulator found." >&2
        else
            echo "Error: No connected Android device or emulator found." >&2
        fi
        echo "Run '$ADB devices -l' to inspect connected targets." >&2
        exit 1
    fi

    echo ""
    echo "Connected Android targets:"
    local i=1
    while IFS= read -r line; do
        echo "  $i. $(device_name_from_line "$line") ($(echo "$line" | awk '{ print $1 }'))"
        i=$((i + 1))
    done <<< "$lines"
    echo ""

    count="$(echo "$lines" | wc -l | tr -d ' ')"
    if [ "$count" -eq 1 ]; then
        choice=1
        echo "==> Auto-selecting only Android target."
    else
        read -rp "Select Android target [1-$count]: " choice
    fi

    line="$(echo "$lines" | sed -n "${choice}p")"
    if [ -z "$line" ]; then
        echo "Error: Invalid selection." >&2
        exit 1
    fi

    DEVICE_ID="$(echo "$line" | awk '{ print $1 }')"
    DEVICE_NAME="$(device_name_from_line "$line")"
    echo "$DEVICE_ID|$DEVICE_NAME" > "$PREF_FILE"
}

adb_device() {
    "$ADB" -s "$DEVICE_ID" "$@"
}

detect_target_abi() {
    local abi
    abi="$(adb_device shell getprop ro.product.cpu.abi | tr -d '\r' | tr -d '\n')"
    if [ -z "$abi" ]; then
        echo "Error: Could not detect Android target ABI. Pass --target <ABI>." >&2
        exit 1
    fi
    echo "$abi"
}

find_apk() {
    local apk_dir="android/build/outputs/apk/$BUILD_TYPE"
    find "$apk_dir" -name "*.apk" -type f 2>/dev/null | sort | head -1
}

select_device
echo "==> Target: $DEVICE_NAME ($DEVICE_ID)"

if [ "$NO_BUILD" = false ]; then
    if [ -z "$TARGET_ABI" ]; then
        TARGET_ABI="$(detect_target_abi)"
    fi
    echo "==> Android ABI: $TARGET_ABI"

    echo "==> Building Rust for Android..."
    if [ "${#RUST_FLAGS[@]}" -gt 0 ]; then
        ./scripts/build-android.sh "${RUST_FLAGS[@]}" "--target=$TARGET_ABI"
    else
        ./scripts/build-android.sh "--target=$TARGET_ABI"
    fi

    echo "==> Building Android app..."
    if [ "${#GRADLE_FLAGS[@]}" -gt 0 ]; then
        (cd android && ./gradlew "$GRADLE_TASK" "${GRADLE_FLAGS[@]}" -x "$RUST_TASK")
    else
        (cd android && ./gradlew "$GRADLE_TASK" -x "$RUST_TASK")
    fi

    APK_PATH="$(find_apk)"
    if [ -z "$APK_PATH" ]; then
        echo "Error: Could not find built Android APK for $BUILD_TYPE." >&2
        exit 1
    fi

    echo "==> Installing $APK_PATH..."
    adb_device install -r "$APK_PATH"
fi

echo "==> Launching..."
adb_device shell am force-stop "$APP_ID" >/dev/null 2>&1 || true
if [ -n "$LAUNCH_URL" ]; then
    echo "==> Opening URL: $LAUNCH_URL"
    adb_device shell am start -W -a android.intent.action.VIEW -d "$LAUNCH_URL" -p "$APP_ID" >/dev/null
else
    adb_device shell am start -W -n "$MAIN_ACTIVITY" -a android.intent.action.MAIN -c android.intent.category.LAUNCHER >/dev/null
fi

echo "==> Running on $DEVICE_NAME"
