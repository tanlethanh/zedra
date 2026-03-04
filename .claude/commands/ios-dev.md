Build, install, launch, and monitor the iOS app on the connected iPad.

## Flags

- `--preview` — enable the `preview` cargo feature (opens `PreviewApp` instead of `ZedraApp`)
- `--debug` — use debug Rust profile (faster compile, no optimizations)

Flags can be combined: `./scripts/run-ios.sh device --preview --debug`

## Steps

1. **Build** Rust libraries + XCFramework + Xcode app + install on device:
   ```
   ./scripts/run-ios.sh device [--preview] [--debug]
   ```
   This script does everything in order:
   - Builds Rust for `aarch64-apple-ios` (staticlib → ZedraFFI.xcframework)
   - Runs `xcodegen generate` to regenerate `ios/Zedra.xcodeproj`
   - Runs `xcodebuild build` targeting the connected iPad
   - Installs with `xcrun devicectl device install app`
   - Launches with `xcrun devicectl device process launch`

2. **Monitor logs** via idevicesyslog (filters to Zedra + errors):
   ```
   /opt/homebrew/bin/idevicesyslog | grep -E 'Zedra|zedra|panic|PANIC|crash|CRASH|fault|error' --line-buffered
   ```

If any step fails, stop and report the error. Do not proceed to the next step.

When monitoring logs, keep watching for ~15 seconds and report any errors, crashes, or notable output.

## Incremental build (skip Rust rebuild)

If you only changed Obj-C or Swift files (no Rust changes), you can skip the Rust build:
```
cd ios && xcodegen generate && xcodebuild build \
  -project Zedra.xcodeproj \
  -scheme Zedra \
  -destination "id=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\([0-9]+\.' | head -1 | grep -oE '[0-9A-F]{8}-[0-9A-F]{16}')" \
  -allowProvisioningUpdates -quiet && cd ..

DEVICE_ID=$(xcrun xctrace list devices 2>&1 | grep -E '^\w.+\([0-9]+\.' | head -1 | grep -oE '[0-9A-F]{8}-[0-9A-F]{16}')
APP_PATH=$(find ~/Library/Developer/Xcode/DerivedData/Zedra-*/Build/Products/Debug-iphoneos -name "Zedra.app" -type d 2>/dev/null | head -1)
xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH"
xcrun devicectl device process launch --device "$DEVICE_ID" dev.zedra.app
```
