Clear logcat and stream filtered logs from the running Android app.

Run this command and monitor the output:
```
adb logcat -c && adb logcat -s zedra:V RustStdoutStderr:V AndroidRuntime:E *:S
```

Watch for ~15 seconds, then report any errors, warnings, crashes, or notable log output. Use timeout to stop if needed:
```
timeout 15 adb logcat -s zedra:V RustStdoutStderr:V AndroidRuntime:E *:S
```
