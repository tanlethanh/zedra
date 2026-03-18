#!/usr/bin/env bash
# Build, install and verify the Zedra APK on a connected device.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
APK_PATH="$ROOT_DIR/android/app/build/outputs/apk/debug/app-debug.apk"
PACKAGE="dev.zedra.app"

# ── colours ────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info()  { echo -e "${GREEN}[info]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
error() { echo -e "${RED}[error]${NC} $*" >&2; }

# ── helpers ────────────────────────────────────────────────────────────────
require() {
    if ! command -v "$1" &>/dev/null; then
        error "Missing required tool: $1"
        exit 1
    fi
}

device_connected() {
    adb devices | awk 'NR>1 && $2=="device"' | grep -qc .
}

wait_for_device() {
    local attempts=0
    while ! device_connected; do
        if (( attempts++ > 10 )); then
            error "No device connected after ${attempts}s"
            exit 1
        fi
        warn "Waiting for device… (${attempts})"
        sleep 1
    done
}

# ── main ───────────────────────────────────────────────────────────────────
main() {
    require adb
    require cargo

    info "Checking device…"
    wait_for_device

    local device
    device=$(adb devices | awk 'NR==2 {print $1}')
    info "Using device: $device"

    info "Building Rust libraries…"
    cd "$ROOT_DIR"
    ./scripts/build-android.sh

    info "Building APK…"
    cd android
    ./gradlew assembleDebug --quiet
    cd "$ROOT_DIR"

    if [[ ! -f "$APK_PATH" ]]; then
        error "APK not found at $APK_PATH"
        exit 1
    fi

    info "Installing APK…"
    adb -s "$device" install -r "$APK_PATH"

    info "Launching app…"
    adb -s "$device" shell am start -n "${PACKAGE}/.MainActivity"

    info "Tailing logcat (Ctrl-C to stop)…"
    adb -s "$device" logcat -s "zedra:*" "*:E"
}

main "$@"
