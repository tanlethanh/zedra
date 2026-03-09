Stream and analyze logs from a USB-connected iOS device running Zedra.

## Run the script

```bash
./scripts/ios-log.sh [--filter <pattern>] [--select-device]
```

Always uses `idevicesyslog` over USB — no sudo required.

- `--filter <pattern>` — extra grep pattern on top of the default Zedra filter
- `--select-device` — ignore saved device pref and re-prompt

**Device preference** is session-scoped (`/tmp/zedra-ios-device-$PPID`, shared with `/ios-dev`).
The script handles selection automatically — read the pref file, prompt if missing, save choice.

## Log format

Rust logs via `IosLogger` (NSLog):
```
Mar  4 17:42:55 Zedra(Zedra.debug.dylib)[PID] <Notice>: [I zedra::module] message
```
Level prefix: `[I`=Info `[W`=Warn `[E`=Error `[D`=Debug `[T`=Trace

## Crash analysis

Fetch crash report from device:
```bash
xcrun devicectl device copy from --device <UDID> \
  "/var/mobile/Library/Logs/CrashReporter/" /tmp/ios-crashes/
# or:
/opt/homebrew/bin/idevicecrashreport -e /tmp/ios-crashes/
```

Early startup crashes (before NSLog is set up) — capture stderr directly:
```bash
xcrun devicectl device process launch --console --device <UDID> dev.zedra.app
```

## Screenshot

```bash
/opt/homebrew/bin/idevicescreenshot /tmp/zedra-screen.png
```
