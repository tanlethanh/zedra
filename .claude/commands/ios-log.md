Stream and analyze logs from a connected iOS device running Zedra.

## Device selection

The device preference is scoped to this Claude Code session using `$PPID`:

```bash
PREF_FILE="/tmp/zedra-ios-device-$PPID"
cat "$PREF_FILE" 2>/dev/null
```

If no preference is saved, enumerate physical devices and ask the user to pick one
(same flow as `/ios-dev`). Use `xcrun xctrace list devices` (not `devicectl`) — it
returns the libimobiledevice-format UDIDs required by `run-ios.sh` and `idevicesyslog`.
Then save the choice:
```bash
echo "<UDID>|<Name>" > "$PREF_FILE"
```

Pass `--select-device` to ignore the saved preference and re-prompt.

## Log format

Rust logs are routed through NSLog via `IosLogger` (src/ios/logger.rs) and appear as:
```
Mar  4 17:42:55 Zedra(Zedra.debug.dylib)[PID] <Notice>: [I zedra::module] message
```
Level prefix: `[I` = Info, `[W` = Warn, `[E` = Error, `[D` = Debug, `[T` = Trace.

## Live log streaming

Stream all Zedra-related logs (Rust NSLog + UIKit):
```
/opt/homebrew/bin/idevicesyslog -u <UDID> | grep -E 'Zedra\[|zedra\[|\[I |\[W |\[E |\[D |panic|PANIC|crash|CRASH|NSException|Terminating' --line-buffered
```

Rust-only logs (strips UIKit noise):
```
/opt/homebrew/bin/idevicesyslog -u <UDID> | grep -E '\[I |\[W |\[E |\[D |\[T ' --line-buffered
```

Stream everything (unfiltered, verbose):
```
/opt/homebrew/bin/idevicesyslog -u <UDID>
```

## Crash analysis

After a crash, fetch the crash report from the device:
```
xcrun devicectl device copy from --device <UDID> \
  "/var/mobile/Library/Logs/CrashReporter/" /tmp/ios-crashes/
ls -lt /tmp/ios-crashes/ | head -5
```

Or use `idevicecrashreport` if available:
```
idevicecrashreport -e /tmp/ios-crashes/
```

## Launch with stderr capture (good for early startup crashes)

Captures stderr directly from the process — useful when the app crashes before NSLog is set up
(e.g. before `zedra_launch_gpui()` runs). The DIAG() macro in main.m writes to stderr.
```
xcrun devicectl device process launch --console --device <UDID> dev.zedra.app
```

## Screenshot (visual verification)

Take a screenshot from the device and pull to local machine:
```
xcrun devicectl device copy from --device <UDID> \
  /tmp/zedra-screen.png /tmp/zedra-screen.png
# Alternative via idevicescreenshot:
idevicescreenshot /tmp/zedra-screen.png
```
