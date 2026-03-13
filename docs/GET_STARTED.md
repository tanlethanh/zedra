# Getting Started with Zedra

Zedra is a port of Zed's GPUI UI framework to Android and iOS. This guide covers everything you need to go from zero to a running build.

## Prerequisites

### 1. Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Add targets for Android and iOS:

```bash
rustup target add aarch64-linux-android        # Android device
rustup target add aarch64-apple-ios            # iOS device
rustup target add aarch64-apple-ios-sim        # iOS simulator (Apple Silicon)
```

### 2. Android

**Android Studio / SDK**
- Install [Android Studio](https://developer.android.com/studio) or the command-line tools
- Install SDK API 31+ via SDK Manager

**Android NDK r25c+**
```bash
# Via Android Studio: SDK Manager → SDK Tools → NDK (Side by side)
# Then set in your shell profile (~/.zshrc or ~/.bashrc):
export ANDROID_NDK_ROOT=$HOME/Library/Android/sdk/ndk/<version>
export ANDROID_HOME=$HOME/Library/Android/sdk
export PATH=$PATH:$ANDROID_HOME/platform-tools
```

**cargo-ndk**
```bash
cargo install cargo-ndk
```

### 3. iOS (macOS only)

**Xcode** — install from the App Store, then accept the license:
```bash
sudo xcodebuild -license accept
```

**Metal Toolchain** — required for shader compilation:
```bash
xcodebuild -downloadComponent MetalToolchain
```

**xcodegen and CocoaPods**
```bash
brew install xcodegen
sudo gem install cocoapods
```

### 4. Git Submodules

The GPUI vendor fork must be initialized before any build:

```bash
git submodule update --init --recursive
```

---

## Android Development

### Quick start

```bash
# Verify your environment before the first build
./scripts/preflight-check.sh

# Build, install, and launch in one step (recommended)
./scripts/dev-cycle.sh
```

### Manual steps

```bash
./scripts/build-android.sh                     # Build Rust .so libraries
cd android && ./gradlew installDebug && cd ..  # Install APK
adb logcat | grep zedra                        # View logs
```

### Background log monitoring

```bash
./scripts/logcat-daemon.sh start   # Start background log monitor
./scripts/logcat-daemon.sh tail    # View live filtered logs
./scripts/logcat-daemon.sh stop    # Stop monitor
```

### Crash analysis

```bash
./scripts/analyze-crash.sh
```

---

## iOS Development

### Quick start

```bash
# Build, install, and launch on simulator (debug)
./scripts/run-ios.sh sim

# Build and install on a connected device
./scripts/run-ios.sh device
```

### Common flags

| Flag | Description |
|---|---|
| `--no-build` | Skip build, just relaunch (uses last installed app) |
| `--release` | Release Rust build (slower to build, faster to run) |
| `--preview` | Enable preview feature flag |
| `--debug` | Xcode Debug configuration (default) |
| `--device-id <UDID>` | Target a specific device |
| `--select-device` | Re-prompt for device selection |
| `--launch-url <URL>` | Open a `zedra://` deep link on launch |

### Examples

```bash
./scripts/run-ios.sh sim                        # Run on simulator (debug)
./scripts/run-ios.sh sim --release              # Run on simulator (release)
./scripts/run-ios.sh sim --no-build             # Relaunch without rebuilding
./scripts/run-ios.sh device --select-device     # Pick which device to use
./scripts/run-ios.sh sim --no-build --launch-url 'zedra://connect?ticket=...'
```

### View logs

```bash
./scripts/ios-log.sh                            # Stream logs via USB
./scripts/ios-log.sh --filter zedra             # Filter to zedra messages
./scripts/ios-log.sh --select-device            # Choose device
```

---

## Host Daemon (`zedra-host`)

The host daemon runs on your desktop and exposes a terminal, filesystem, and git over iroh P2P transport. The mobile client connects by scanning a QR code.

```bash
# Start the daemon (serves current directory)
cargo run -p zedra-host -- start

# Start serving a specific directory
cargo run -p zedra-host -- start --workdir ~/myproject

# Measure round-trip latency to the running daemon
cargo run -p zedra-host -- client

# Stop the daemon
cargo run -p zedra-host -- stop
```

When started, `zedra start` prints a QR code and URL:

```
zedra://connect?ticket=<base64url_encoded_ticket>
```

Scan the QR from the app, or pass the URL directly via `--launch-url` during development:

```bash
./scripts/run-ios.sh sim --no-build --launch-url 'zedra://connect?ticket=...'
```

---

## Project Structure

```
crates/
  zedra/           # Android/iOS app binary (cdylib + staticlib)
  zedra-host/      # Desktop daemon (terminal, FS, git over iroh)
  zedra-rpc/       # Shared protocol types + QR pairing codec
  zedra-session/   # Mobile client: iroh connection + auto-reconnect
  zedra-terminal/  # Terminal emulation (alacritty + GPUI rendering)
vendor/
  zed/             # GPUI fork with Android/iOS platform support
android/           # Android Studio project (Java + JNI)
ios/               # Xcode project (ObjC + Swift shims)
scripts/           # Build, deploy, and debug scripts
packages/
  relay-worker/    # Cloudflare Worker relay (relay.zedra.dev)
```

---

## Running Tests

```bash
# All workspace tests (host, rpc, session)
cargo test

# Specific crate
cargo test -p zedra-rpc
cargo test -p zedra-host

# Integration tests (requires no running daemon on same port)
cargo test -p zedra-host --test integration
```

---

## Troubleshooting

**Black screen on Android**
- Check surface dimensions are physical pixels: `adb shell screencap -p /sdcard/s.png && adb pull /sdcard/s.png /tmp/s.png`
- Verify Vulkan 1.1+: `adb shell getprop ro.hardware.vulkan`

**Metal shader compilation fails**
```bash
xcodebuild -downloadComponent MetalToolchain
```

**xcodegen: `Decoding failed at "path": Nothing found`**
- Ensure `info:` block in `ios/project.yml` has an explicit `path: Zedra/Info.plist` key.

**Submodule missing / build errors**
```bash
git submodule update --init --recursive
```

**iOS build: provisioning errors**
- Xcode requires a valid team. Check `DEVELOPMENT_TEAM` in `ios/project.yml` matches your Apple Developer account.
