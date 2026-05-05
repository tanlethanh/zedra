# Getting Started

## Prerequisites

### Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim  # iOS
rustup target add aarch64-linux-android                     # Android
```

### Git Submodules

```bash
git submodule update --init --recursive
```

### iOS

- Xcode 26+ (App Store) — `sudo xcodebuild -license accept`
- xcodegen: `brew install xcodegen`
- libimobiledevice: `brew install libimobiledevice` (for device logs)
- CocoaPods: `sudo gem install cocoapods`

### Android

- Android Studio / SDK (API 31+)
- Android NDK r25c+
- Set env vars:
  ```bash
  export ANDROID_NDK_ROOT=$HOME/Library/Android/sdk/ndk/<version>
  export ANDROID_HOME=$HOME/Library/Android/sdk
  ```
- `cargo install cargo-ndk`

## iOS Development

```bash
./scripts/run-ios.sh device          # full build + install + launch
./scripts/run-ios.sh sim             # simulator
./scripts/run-ios.sh device --release  # release build
./scripts/log-ios.sh                 # stream device logs
./scripts/log-ios.sh --filter zedra  # filtered logs
```

See `docs/IOS_WORKFLOW.md` for full pipeline details.

## Android Development

```bash
./scripts/build-android.sh                     # build Rust .so
cd android && ./gradlew installDebug && cd ..  # install APK
./scripts/log-android.sh start                 # background log monitor
./scripts/log-android.sh tail                  # view logs
```

## Host Daemon

```bash
cargo run -p zedra-host -- start                    # start, show QR
cargo run -p zedra-host -- start --workdir ~/project  # specific directory
cargo run -p zedra-host -- start --detach           # keep running after SSH logout
cargo run -p zedra-host -- start --static-qr        # static startup QR for review/testing
cargo run -p zedra-host -- qr --workdir .           # refresh one-time QR
cargo run -p zedra-host -- qr --workdir . --static  # static QR for testing/store review
cargo run -p zedra-host -- logs --workdir .         # show recent daemon logs
cargo run -p zedra-host -- client                   # measure RTT
cargo run -p zedra-host -- stop                     # stop daemon
```

Scan the printed QR from the app, or pass the URL during development:

```bash
./scripts/run-ios.sh sim --no-build --launch-url 'zedra://connect?ticket=...'
```

## Pre-Commit Checks

```bash
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host
```

## Troubleshooting

- **Black screen (Android)**: surface dimensions must be physical pixels. Check Vulkan 1.1+: `adb shell getprop ro.hardware.vulkan`
- **Submodule missing**: `git submodule update --init --recursive`
- **iOS provisioning**: check `DEVELOPMENT_TEAM` in `ios/project.yml`
- **Metal shader fail**: `xcodebuild -downloadComponent MetalToolchain`
