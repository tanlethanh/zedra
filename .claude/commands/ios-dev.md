Build, install, launch, and monitor the iOS app on a connected device.

## Flags

- `--preview` — enable the `preview` cargo feature (opens `PreviewApp` instead of `ZedraApp`)
- `--debug` — use debug Rust profile (faster compile, no optimizations)
- `--devtool` — enable the in-app HTTP devtool (127.0.0.1:9777, debug only); see `docs/DEVTOOL.md`
- `--select-device` — ignore saved preference and prompt for device selection

Flags can be combined: `./scripts/run-ios.sh device --preview --debug`

## Device selection

The device preference is one machine-global target — the fixed path below is
shared by every checkout and worktree, not scoped to a session, shell, or
checkout. Earlier this used `$PPID`, but a child script sees its
invoker's PID, not the invoker's own `$PPID`, so that scoping was fragile
across separate tool calls and let `devtool.sh`/`ios-log.sh daemon` silently
diverge on which device they thought was current. Shared by `run-ios.sh`,
`ios-log.sh`, and `devtool.sh`.

```bash
PREF_FILE="/tmp/zedra-ios-device"
```

**Step 1 — Check for saved preference** (skip if `--select-device` was passed):
```bash
cat "$PREF_FILE" 2>/dev/null
```
The file contains `<UDID>|<Name>` if a device was already chosen.

**Step 2 — If no preference (or `--select-device`)**, enumerate connected devices:
```bash
xcrun xctrace list devices 2>&1 | grep -vE 'Simulator|^=='
```
This lists physical devices with their UDIDs in libimobiledevice format (used by run-ios.sh
and idevicesyslog). Then use `AskUserQuestion` to present the list and ask the user to pick one:
> Connected iOS devices:
> 1. Tan iPad (00008132-0019312A3C83001C)
> 2. Tan iPhone (00008140-0010798E1413801C)
>
> Which device would you like to target?

**Step 3 — Save the chosen device** as the current global target:
```bash
echo "<UDID>|<Name>" > "$PREF_FILE"
```

**Step 4 — Use `--device-id <UDID>`** when invoking the build script.

## Build and install

```
./scripts/run-ios.sh device --device-id <UDID> [--preview] [--debug]
```

The script does everything in order:
- Builds Rust for `aarch64-apple-ios` (staticlib → ZedraFFI.xcframework)
- Runs `xcodegen generate` to regenerate `ios/Zedra.xcodeproj`
- Runs `xcodebuild build` targeting the chosen device
- Installs with `xcrun devicectl device install app`
- Launches with `xcrun devicectl device process launch`

If any step fails, stop and report the error. Do not proceed to the next step.

## Monitor logs

After launch, stream logs via idevicesyslog using the saved UDID:
```
/opt/homebrew/bin/idevicesyslog -u <UDID> | grep -E 'Zedra|zedra|panic|PANIC|crash|CRASH|fault|error' --line-buffered
```

Keep watching for ~15 seconds and report any errors, crashes, or notable output.

## Incremental build (skip Rust rebuild)

If you only changed Obj-C or Swift files (no Rust changes), you can skip the Rust build:
```
DEVICE_ID=<UDID>
cd ios && xcodegen generate && xcodebuild build \
  -project Zedra.xcodeproj \
  -scheme Zedra \
  -destination "id=$DEVICE_ID" \
  -allowProvisioningUpdates -quiet && cd ..

APP_PATH=$(find ~/Library/Developer/Xcode/DerivedData/Zedra-*/Build/Products/Debug-iphoneos -name "Zedra.app" -type d 2>/dev/null | head -1)
xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH"
xcrun devicectl device process launch --device "$DEVICE_ID" dev.zedra.app
```
