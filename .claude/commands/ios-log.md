Stream and analyze logs from the connected iPad running Zedra.

## Live log streaming

Stream all Zedra-related logs (Rust oslog + UIKit):
```
/opt/homebrew/bin/idevicesyslog | grep -E 'Zedra|zedra|panic|PANIC|crash|CRASH|fault|NSException|Terminating' --line-buffered
```

Stream everything (unfiltered, verbose):
```
/opt/homebrew/bin/idevicesyslog
```

## Crash analysis

After a crash, fetch the crash report from the device:
```
xcrun devicectl device copy from --device 9F0ACED3-EE4F-593D-B15E-93954D93FD94 \
  "/var/mobile/Library/Logs/CrashReporter/" /tmp/ios-crashes/
ls -lt /tmp/ios-crashes/ | head -5
```

Or use `idevicecrashreport` if available:
```
idevicecrashreport -e /tmp/ios-crashes/
```

## Launch with stderr capture (good for early startup crashes)

Captures stderr directly from the process — useful when the app crashes before oslog is set up:
```
xcrun devicectl device process launch --console --device 9F0ACED3-EE4F-593D-B15E-93954D93FD94 dev.zedra.app
```

## Screenshot (visual verification)

Take a screenshot from the device and pull to local machine:
```
xcrun devicectl device copy from --device 9F0ACED3-EE4F-593D-B15E-93954D93FD94 \
  /tmp/zedra-screen.png /tmp/zedra-screen.png
# Alternative via idevicescreenshot:
idevicescreenshot /tmp/zedra-screen.png
```
