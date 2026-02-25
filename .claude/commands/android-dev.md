Build, install, launch, and monitor the Android app.

If the user says "preview" or "preview mode", add `--features preview` to the cargo build command.

## Steps

1. **Build** the Rust libraries for Android (arm64 only, ~9s incremental):
   ```
   cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib
   ```
   For preview mode:
   ```
   cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib --features preview
   ```

2. **Install** the APK to the connected device (skip redundant Rust build):
   ```
   cd android && ./gradlew installDebug -x buildRustLib && cd ..
   ```

3. **Launch** the app on device:
   ```
   adb shell am force-stop dev.zedra.app && adb shell am start -n dev.zedra.app/.MainActivity
   ```

4. **Monitor logs** with filtered output (zedra tags only):
   ```
   adb logcat -c && adb logcat -s zedra:V RustStdoutStderr:V AndroidRuntime:E
   ```

If any step fails, stop and report the error. Do not proceed to the next step.

When monitoring logs, keep watching for ~10 seconds and report any errors, warnings, or notable output. Use Ctrl+C or timeout to stop log monitoring.

## Release build

For release builds (all architectures, optimized), use the full build script instead of step 1:
```
./scripts/build-android.sh         # normal mode
./scripts/build-android.sh --preview  # preview mode
```
And remove `-x buildRustLib` from step 2.
