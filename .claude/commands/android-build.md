Build the Rust libraries and install the APK to the connected Android device. Does NOT launch the app or monitor logs.

If the user says "preview" or "preview mode", add `--features preview` to the cargo build command.

1. **Build** (arm64 only, fast incremental):
   ```
   cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib
   ```
   For preview mode:
   ```
   cargo ndk -t arm64-v8a -o ./android/app/libs build -p zedra --lib --features preview
   ```

2. **Install** (skip redundant Rust build):
   ```
   cd android && ./gradlew installDebug -x buildRustLib && cd ..
   ```

Report success or failure. If the build fails, show the relevant compiler errors.

## Release build

For release builds (all architectures, optimized), use the full build script instead of step 1:
```
./scripts/build-android.sh         # normal mode
./scripts/build-android.sh --preview  # preview mode
```
And remove `-x buildRustLib` from step 2.
