Build the Rust libraries and install the APK to the connected Android device. Does NOT launch the app or monitor logs.

1. **Build**:
   ```
   ./scripts/build-android.sh
   ```

2. **Install**:
   ```
   cd android && ./gradlew installDebug && cd ..
   ```

Report success or failure. If the build fails, show the relevant compiler errors.
