Build, install, launch, and monitor the Android app.

## Steps

1. **Build** the Rust libraries for Android:
   ```
   ./scripts/build-android.sh
   ```

2. **Install** the APK to the connected device:
   ```
   cd android && ./gradlew installDebug && cd ..
   ```

3. **Launch** the app on device:
   ```
   adb shell am start -n dev.zedra.app/.MainActivity
   ```

4. **Monitor logs** with filtered output (zedra tags only):
   ```
   adb logcat -c && adb logcat -s zedra:V RustStdoutStderr:V AndroidRuntime:E *:S
   ```

If any step fails, stop and report the error. Do not proceed to the next step.

When monitoring logs, keep watching for ~10 seconds and report any errors, warnings, or notable output. Use Ctrl+C or timeout to stop log monitoring.
