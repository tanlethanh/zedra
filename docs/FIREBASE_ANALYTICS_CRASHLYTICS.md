# Firebase Analytics + Crashlytics Integration

This document covers the full integration plan for Firebase Analytics and Firebase Crashlytics in Zedra — a Rust-based mobile app with a JNI bridge (Android) and an `extern "C"` FFI bridge (iOS).

## Architecture Overview

Firebase SDKs are Java/Kotlin (Android) and Obj-C/Swift (iOS). Rust cannot call them directly. The integration follows the same bridge pattern already used for keyboard control, QR scanning, and display density:

```
Rust analytics API  (crates/zedra/src/analytics.rs)
    |
    +-- Android: JNI --> ZedraFirebase.java --> Firebase Android SDK
    |
    +-- iOS:  extern "C" --> ZedraFirebase.m --> Firebase iOS SDK

Crashlytics NDK plugin
    --> uploads unstripped .so debug symbols to Firebase
    --> native signal handler catches SIGSEGV / abort from Rust .so
```

### Why this approach

- No Rust Firebase crate exists for Android/iOS at the level needed.
- The JNI + FFI patterns are already proven in the codebase (`jni.rs`, `ios/app.rs`).
- Crashlytics NDK handles the hardest case (native SIGSEGV / OOM abort) automatically from the Gradle plugin — zero Rust code needed for native crashes in release.
- `panic = "abort"` in the release profile means Rust panics also produce native abort signals captured by Crashlytics NDK.

---

## Implementation Plan

### Phase 1 — Firebase Project Setup (manual)

1. Go to the Firebase console and create a new project (or use an existing one).
2. Add an **Android app**:
   - Package name: `dev.zedra.app`
   - Register the SHA-1 certificate fingerprint (see below — required for Analytics and Crashlytics to activate)
   - Download `google-services.json`
   - Place it at `android/google-services.json` (same directory as `build.gradle`)
3. Add an **iOS app**:
   - Bundle ID: `dev.zedra.app`
   - Download `GoogleService-Info.plist`
   - Place it at `ios/Zedra/GoogleService-Info.plist`

#### Android SHA-1 Fingerprint

Firebase requires the SHA-1 fingerprint of your signing certificate to verify app identity and enable Analytics and Crashlytics. Without it the Firebase project will not associate events or crashes with the correct app.

**Debug builds** use a shared debug keystore committed to the repo at `android/debug.keystore`.
This is a standard self-signed certificate with well-known credentials — it is intentionally
public and safe to commit. The `.gitignore` allows it via `!android/debug.keystore`.

Keystore details:
- Path: `android/debug.keystore`
- Alias: `androiddebugkey`
- Store password: `android`
- Key password: `android`

Fingerprints for this keystore:
```
SHA1:   7E:ED:96:E1:53:20:6C:9C:21:13:51:2C:82:69:22:D8:EB:6D:AF:B5
SHA256: 85:BF:D7:E6:1D:E0:5B:4F:C3:C2:BD:F5:22:9F:95:00:4D:15:18:A0:1E:E6:29:CC:CD:EF:37:62:65:B5:7F:89
```

To verify locally at any time:

```bash
keytool -list -v \
  -keystore android/debug.keystore \
  -alias androiddebugkey \
  -storepass android \
  -keypass android \
  | grep -E "SHA1:|SHA256:"
```

Or via Gradle:

```bash
cd android && ./gradlew signingReport
```

**Release builds** use a production keystore that is never committed to the repo.
Run the same `keytool` command pointing at your release keystore and alias to get
its SHA-1 for Firebase.

**Registering in Firebase:**

1. Open the Firebase console → Project settings → Your Android app
2. Click "Add fingerprint"
3. Paste the SHA-1 value (register both the debug SHA-1 above and your release SHA-1)
4. Re-download `google-services.json` after adding fingerprints — the file
   embeds the certificate hashes and must be updated whenever you add one

---

### Phase 2 — Android: Gradle Changes

File: `android/build.gradle`

**a) Add plugins to `buildscript.dependencies`:**

```groovy
buildscript {
    dependencies {
        classpath 'com.android.tools.build:gradle:8.13.2'
        classpath 'com.google.gms:google-services:4.4.2'
        classpath 'com.google.firebase:firebase-crashlytics-gradle:3.0.2'
    }
}
```

**b) Apply the plugins after `apply plugin: 'com.android.application'`:**

```groovy
apply plugin: 'com.android.application'
apply plugin: 'com.google.gms.google-services'
apply plugin: 'com.google.firebase.crashlytics'
```

**c) Add Firebase dependencies:**

```groovy
dependencies {
    implementation platform('com.google.firebase:firebase-bom:33.7.0')
    implementation 'com.google.firebase:firebase-analytics'
    implementation 'com.google.firebase:firebase-crashlytics'
    implementation 'com.google.firebase:firebase-crashlytics-ndk'
}
```

**d) Configure Crashlytics NDK symbol upload inside `android { buildTypes { release { ... } } }`:**

```groovy
buildTypes {
    release {
        minifyEnabled true
        firebaseCrashlytics {
            nativeSymbolUploadEnabled true
            unstrippedNativeLibsDir 'libs'
        }
    }
}
```

The `unstrippedNativeLibsDir` should point to where `build-android.sh` deposits
the `.so` files before stripping. See Phase 10 for the script change.

---

### Phase 3 — Android: Java Firebase Wrapper

New file: `android/app/src/main/java/dev/zedra/app/ZedraFirebase.java`

This class initializes Firebase once and exposes static methods called from Rust via JNI.

```java
package dev.zedra.app;

import android.content.Context;
import android.os.Bundle;
import com.google.firebase.FirebaseApp;
import com.google.firebase.analytics.FirebaseAnalytics;
import com.google.firebase.crashlytics.FirebaseCrashlytics;

public class ZedraFirebase {
    private static FirebaseAnalytics analytics;

    /** Called once from MainActivity.onCreate() before gpuiInit(). */
    public static void initialize(Context context) {
        FirebaseApp.initializeApp(context);
        analytics = FirebaseAnalytics.getInstance(context);
    }

    /** Log a screen view or custom event. Keys/values are paired by index. */
    public static void logEvent(String name, String[] keys, String[] values) {
        if (analytics == null) return;
        Bundle params = new Bundle();
        if (keys != null) {
            for (int i = 0; i < keys.length && i < values.length; i++) {
                params.putString(keys[i], values[i]);
            }
        }
        analytics.logEvent(name, params);
    }

    /** Record a non-fatal error (visible in Crashlytics > Non-fatals). */
    public static void recordError(String message, String file, int line) {
        FirebaseCrashlytics.getInstance().recordException(
            new RuntimeException("[" + file + ":" + line + "] " + message)
        );
    }

    /** Record a Rust panic as a non-fatal (debug builds; release uses native abort). */
    public static void recordPanic(String message, String location) {
        FirebaseCrashlytics.getInstance().log("PANIC at " + location + ": " + message);
        FirebaseCrashlytics.getInstance().recordException(
            new RuntimeException("Rust panic at " + location + ": " + message)
        );
    }

    /** Associate subsequent events/crashes with a session or user identity. */
    public static void setUserId(String id) {
        if (analytics != null) analytics.setUserId(id);
        FirebaseCrashlytics.getInstance().setUserId(id);
    }

    /** Add a key-value breadcrumb to crash reports. */
    public static void setCustomKey(String key, String value) {
        FirebaseCrashlytics.getInstance().setCustomKey(key, value);
    }
}
```

**Wire into `MainActivity.onCreate()`** — call `ZedraFirebase.initialize(this)` before `gpuiInit(this)`:

```java
@Override
protected void onCreate(Bundle savedInstanceState) {
    SplashScreen.installSplashScreen(this);
    super.onCreate(savedInstanceState);

    ZedraFirebase.initialize(this);   // <-- add this line

    gpuiInitMainThread();
    gpuiHandle = gpuiInit(this);
    // ...
}
```

---

### Phase 4 — Android: Rust JNI Bridge

New file: `crates/zedra/src/android/analytics.rs`

Follows the established pattern from `show_keyboard_inner()` in `jni.rs` — grab the stored `JVM`, attach the thread, call a static method on `ZedraFirebase`.

```rust
use jni::objects::{JString, JValue};
use std::ffi::CString;

use super::jni::JVM;

fn with_env<F: FnOnce(&mut jni::JNIEnv)>(f: F) {
    let jvm = match JVM.lock() {
        Ok(g) => match g.as_ref() {
            Some(jvm) => jvm.clone(),
            None => return,
        },
        Err(_) => return,
    };
    let mut env = match jvm.get_env()
        .or_else(|_| jvm.attach_current_thread_as_daemon())
    {
        Ok(e) => e,
        Err(e) => { log::error!("analytics: attach failed: {:?}", e); return; }
    };
    f(&mut env);
}

pub fn log_event(name: &str, params: &[(&str, &str)]) {
    with_env(|env| {
        let class = match env.find_class("dev/zedra/app/ZedraFirebase") {
            Ok(c) => c,
            Err(_) => return,
        };
        let jname = match env.new_string(name) {
            Ok(s) => s,
            Err(_) => return,
        };
        let keys: Vec<_> = params.iter()
            .filter_map(|(k, _)| env.new_string(*k).ok())
            .collect();
        let values: Vec<_> = params.iter()
            .filter_map(|(_, v)| env.new_string(*v).ok())
            .collect();

        let string_class = env.find_class("java/lang/String").unwrap();
        let jkeys = env.new_object_array(keys.len() as i32, &string_class, JObject::null()).unwrap();
        let jvalues = env.new_object_array(values.len() as i32, &string_class, JObject::null()).unwrap();
        for (i, (k, v)) in keys.iter().zip(values.iter()).enumerate() {
            env.set_object_array_element(&jkeys, i as i32, k).ok();
            env.set_object_array_element(&jvalues, i as i32, v).ok();
        }
        env.call_static_method(
            &class,
            "logEvent",
            "(Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V",
            &[(&jname).into(), (&jkeys).into(), (&jvalues).into()],
        ).ok();
    });
}

pub fn record_error(message: &str, file: &str, line: u32) {
    with_env(|env| {
        let class = env.find_class("dev/zedra/app/ZedraFirebase").ok()?;
        let jmsg = env.new_string(message).ok()?;
        let jfile = env.new_string(file).ok()?;
        env.call_static_method(
            &class,
            "recordError",
            "(Ljava/lang/String;Ljava/lang/String;I)V",
            &[(&jmsg).into(), (&jfile).into(), JValue::Int(line as i32)],
        ).ok();
        Some(())
    });
}

pub fn record_panic(message: &str, location: &str) {
    with_env(|env| {
        let class = env.find_class("dev/zedra/app/ZedraFirebase").ok()?;
        let jmsg = env.new_string(message).ok()?;
        let jloc = env.new_string(location).ok()?;
        env.call_static_method(
            &class,
            "recordPanic",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[(&jmsg).into(), (&jloc).into()],
        ).ok();
        Some(())
    });
}

pub fn set_user_id(id: &str) {
    with_env(|env| {
        let class = env.find_class("dev/zedra/app/ZedraFirebase").ok()?;
        let jid = env.new_string(id).ok()?;
        env.call_static_method(
            &class, "setUserId",
            "(Ljava/lang/String;)V",
            &[(&jid).into()],
        ).ok();
        Some(())
    });
}
```

Register the module in `crates/zedra/src/android/mod.rs`:

```rust
pub mod analytics;
```

---

### Phase 5 — iOS: CocoaPods Setup

The iOS project uses XcodeGen (`project.yml`) and has no current package manager. CocoaPods is the recommended path for Firebase on iOS.

**New file: `ios/Podfile`**

```ruby
platform :ios, '16.0'
use_frameworks! :linkage => :static

target 'Zedra' do
  pod 'FirebaseAnalytics'
  pod 'FirebaseCrashlytics'
end
```

**Install:**

```bash
cd ios && pod install
```

This creates `ios/Zedra.xcworkspace`. After this, always open and build via the workspace, not the `.xcodeproj`.

**`ios/project.yml` additions** — add `CLANG_ENABLE_MODULES: YES` under `settings.base` and
add `FirebaseCore`, `FirebaseAnalytics`, and `FirebaseCrashlytics` to the `dependencies` list
once CocoaPods has generated the Pods project:

```yaml
dependencies:
  - framework: ZedraFFI.xcframework
    embed: false
  - framework: Pods/FirebaseAnalytics/Frameworks/FirebaseAnalytics.xcframework
    embed: true
  # ... etc — managed by pod install, edit Podfile not project.yml for Firebase
```

In practice, CocoaPods manages the Xcode target linkage automatically after `pod install`. The `project.yml` only needs to declare the `Podfile` path so XcodeGen regenerates a workspace-aware project:

```yaml
options:
  podfile: Podfile
```

**Update `scripts/run-ios.sh`** to build via the workspace when it exists:

```bash
if [ -f "ios/Zedra.xcworkspace" ]; then
    xcodebuild -workspace ios/Zedra.xcworkspace -scheme Zedra ...
else
    xcodebuild -project ios/Zedra.xcodeproj -scheme Zedra ...
fi
```

---

### Phase 6 — iOS: Obj-C Firebase Bridge

New file: `ios/Zedra/ZedraFirebase.m`

Provides `extern "C"` C functions callable from Rust via the existing FFI pattern.

```objc
#import <Foundation/Foundation.h>
#import <FirebaseCore/FirebaseCore.h>
#import <FirebaseAnalytics/FirebaseAnalytics.h>
#import <FirebaseCrashlytics/FirebaseCrashlytics.h>

void zedra_firebase_initialize(void) {
    [FIRApp configure];
}

void zedra_log_event(const char* name,
                     const char* const* keys,
                     const char* const* values,
                     int count)
{
    NSString* eventName = [NSString stringWithUTF8String:name ?: "unknown"];
    NSMutableDictionary* params = [NSMutableDictionary dictionary];
    for (int i = 0; i < count; i++) {
        NSString* k = [NSString stringWithUTF8String:keys[i]];
        NSString* v = [NSString stringWithUTF8String:values[i]];
        params[k] = v;
    }
    [FIRAnalytics logEventWithName:eventName parameters:params];
}

void zedra_record_error(const char* message, const char* file, int line) {
    NSString* msg = [NSString stringWithFormat:@"[%s:%d] %s", file, line, message];
    NSError* error = [NSError errorWithDomain:@"dev.zedra.rust"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey: msg}];
    [[FIRCrashlytics crashlytics] recordError:error];
}

void zedra_record_panic(const char* message, const char* location) {
    NSString* msg = [NSString stringWithFormat:@"Rust panic at %s: %s", location, message];
    [[FIRCrashlytics crashlytics] log:msg];
    NSError* error = [NSError errorWithDomain:@"dev.zedra.rust.panic"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey: msg}];
    [[FIRCrashlytics crashlytics] recordError:error];
}

void zedra_set_user_id(const char* user_id) {
    NSString* uid = [NSString stringWithUTF8String:user_id ?: ""];
    [FIRAnalytics setUserID:uid];
    [[FIRCrashlytics crashlytics] setUserID:uid];
}

void zedra_set_custom_key(const char* key, const char* value) {
    NSString* k = [NSString stringWithUTF8String:key ?: ""];
    NSString* v = [NSString stringWithUTF8String:value ?: ""];
    [[FIRCrashlytics crashlytics] setCustomValue:v forKey:k];
}
```

**Wire Firebase init into `main.m`** — call `zedra_firebase_initialize()` before `zedra_launch_gpui()`:

```objc
// In application:didFinishLaunchingWithOptions:
zedra_firebase_initialize();
zedra_launch_gpui();
```

**Expose the declarations in the FFI header** `ios/ZedraFFI.xcframework/ios-arm64/Headers/zedra_ios.h` (and the simulator variant):

```c
void zedra_firebase_initialize(void);
void zedra_log_event(const char* name,
                     const char* const* keys,
                     const char* const* values,
                     int count);
void zedra_record_error(const char* message, const char* file, int line);
void zedra_record_panic(const char* message, const char* location);
void zedra_set_user_id(const char* user_id);
void zedra_set_custom_key(const char* key, const char* value);
```

Note: `ZedraFirebase.m` lives in the `ios/Zedra/` Xcode target, not inside the static
library. The Rust static lib declares `extern "C"` symbols that the linker resolves at link
time from the Obj-C file compiled into the app target.

---

### Phase 7 — iOS: Rust FFI Declarations

New file: `crates/zedra/src/ios/analytics.rs`

```rust
use std::ffi::{CString, c_char, c_int};

extern "C" {
    fn zedra_log_event(
        name: *const c_char,
        keys: *const *const c_char,
        values: *const *const c_char,
        count: c_int,
    );
    fn zedra_record_error(message: *const c_char, file: *const c_char, line: c_int);
    fn zedra_record_panic(message: *const c_char, location: *const c_char);
    fn zedra_set_user_id(user_id: *const c_char);
    fn zedra_set_custom_key(key: *const c_char, value: *const c_char);
}

pub fn log_event(name: &str, params: &[(&str, &str)]) {
    let cname = CString::new(name).unwrap_or_default();
    let ckeys: Vec<CString> = params.iter().map(|(k, _)| CString::new(*k).unwrap_or_default()).collect();
    let cvals: Vec<CString> = params.iter().map(|(_, v)| CString::new(*v).unwrap_or_default()).collect();
    let key_ptrs: Vec<*const c_char> = ckeys.iter().map(|s| s.as_ptr()).collect();
    let val_ptrs: Vec<*const c_char> = cvals.iter().map(|s| s.as_ptr()).collect();
    unsafe {
        zedra_log_event(cname.as_ptr(), key_ptrs.as_ptr(), val_ptrs.as_ptr(), params.len() as c_int);
    }
}

pub fn record_error(message: &str, file: &str, line: u32) {
    let m = CString::new(message).unwrap_or_default();
    let f = CString::new(file).unwrap_or_default();
    unsafe { zedra_record_error(m.as_ptr(), f.as_ptr(), line as c_int); }
}

pub fn record_panic(message: &str, location: &str) {
    let m = CString::new(message).unwrap_or_default();
    let l = CString::new(location).unwrap_or_default();
    unsafe { zedra_record_panic(m.as_ptr(), l.as_ptr()); }
}

pub fn set_user_id(id: &str) {
    let s = CString::new(id).unwrap_or_default();
    unsafe { zedra_set_user_id(s.as_ptr()); }
}
```

Register in `crates/zedra/src/ios/mod.rs`:

```rust
pub mod analytics;
```

---

### Phase 8 — Shared Rust Analytics Module

New file: `crates/zedra/src/analytics.rs`

Single call site for all Rust code. Platform implementations are gated by `cfg`.

```rust
/// Log a named event with optional key-value parameters.
///
/// On Android: calls ZedraFirebase.logEvent() via JNI.
/// On iOS: calls zedra_log_event() via FFI.
/// On other platforms (host, tests): no-op.
pub fn log_event(name: &str, params: &[(&str, &str)]) {
    #[cfg(target_os = "android")]
    crate::android::analytics::log_event(name, params);
    #[cfg(target_os = "ios")]
    crate::ios::analytics::log_event(name, params);
    let _ = (name, params);
}

/// Record a non-fatal error. Use for recoverable errors worth tracking.
pub fn record_error(message: &str) {
    #[cfg(target_os = "android")]
    crate::android::analytics::record_error(message, "", 0);
    #[cfg(target_os = "ios")]
    crate::ios::analytics::record_error(message, "", 0);
    let _ = message;
}

/// Record a Rust panic as a non-fatal crash.
/// Called from install_panic_hook() in lib.rs.
pub fn record_panic(message: &str, location: &str) {
    #[cfg(target_os = "android")]
    crate::android::analytics::record_panic(message, location);
    #[cfg(target_os = "ios")]
    crate::ios::analytics::record_panic(message, location);
    let _ = (message, location);
}

/// Associate this device/session with an identifier in Analytics + Crashlytics.
pub fn set_user_id(id: &str) {
    #[cfg(target_os = "android")]
    crate::android::analytics::set_user_id(id);
    #[cfg(target_os = "ios")]
    crate::ios::analytics::set_user_id(id);
    let _ = id;
}
```

Register in `crates/zedra/src/lib.rs`:

```rust
pub mod analytics;
```

**Update `install_panic_hook()` in `lib.rs`** to forward panics to Crashlytics:

```rust
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload()
            .downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "Unknown panic".to_string());

        let location = info.location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());

        log::error!("PANIC at {}: {}", location, payload);

        // Forward to Crashlytics (no-op in release where panic = "abort" is caught natively)
        crate::analytics::record_panic(&payload, &location);
    }));
}
```

---

### Phase 9 — Event Instrumentation Points

Suggested events to log using `crate::analytics::log_event()`:

| Location | Event name | Key params |
|----------|-----------|------------|
| `home_view.rs` — connect button | `connect_attempt` | `transport` |
| `zedra-session/src/lib.rs` — connected | `session_connected` | `relay`, `rtt_ms` |
| `zedra-session/src/lib.rs` — reconnect | `session_reconnected` | `attempt`, `backoff_s` |
| `zedra-session/src/lib.rs` — disconnect | `session_disconnected` | `reason` |
| `app.rs` — screen navigation | `screen_view` | `screen_name` |
| `terminal_panel.rs` — terminal opened | `terminal_opened` | — |
| `editor/code_editor.rs` — file opened | `file_opened` | `extension` |
| `android/jni.rs` — QR scanned | `qr_scanned` | — |

Keep event names under 40 characters (Firebase Analytics limit).

---

### Phase 10 — Crashlytics NDK Symbol Upload for Release

For Crashlytics to symbolicate Rust `.so` stack traces in release builds, it needs the
**unstripped** `.so` before `strip = true` (Cargo.toml) removes symbols.

Update `scripts/build-android.sh` for release builds:

```bash
# After cargo ndk builds the .so into target/aarch64-linux-android/release/
# and before Gradle strips them, copy unstripped libs to a staging area.
if [[ "$RUST_DEBUG" != "1" ]]; then
    mkdir -p android/app/unstripped-libs/arm64-v8a
    cp target/aarch64-linux-android/release/libzedra.so \
       android/app/unstripped-libs/arm64-v8a/libzedra.so
fi
```

Then update `firebaseCrashlytics` in `build.gradle`:

```groovy
firebaseCrashlytics {
    nativeSymbolUploadEnabled true
    unstrippedNativeLibsDir 'app/unstripped-libs'
}
```

The Gradle plugin uploads the unstripped `.so` to Firebase during `./gradlew assembleRelease`.
Debug builds already use unstripped libs so no staging step is needed.

---

---

## Host Daemon Analytics (`zedra-host`)

The desktop daemon uses the **GA4 Measurement Protocol** directly over HTTPS — no Firebase SDK required. All events go to the same GA4 property as the mobile app.

### Enabling

Set two environment variables at **build time**:

```bash
ZEDRA_GA_MEASUREMENT_ID=G-XXXXXXXXXX \
ZEDRA_GA_API_SECRET=your_secret \
cargo build -p zedra-host --release
```

Get the API secret from Firebase console → your GA4 property → Data Streams → Web stream → **Measurement Protocol API secrets**.

If either variable is absent or empty the analytics module is silently disabled — no HTTP calls, no overhead.

### Analytics ID

A random UUID is generated on first run and stored at `~/.config/zedra/analytics_id`. It is machine-level (shared across workspaces), never tied to cryptographic identity, and never transmitted alongside it.

### Events

Every event automatically includes `host_version`, `os`, and `arch` fields.

| Event | Params | Fired when |
|-------|--------|-----------|
| `daemon_start` | `relay_type` (cf_worker/custom/default) | `zedra start` |
| `net_report` | `has_ipv4`, `has_ipv6`, `symmetric_nat` | STUN completes (~1s after bind) |
| `client_paired` | — | New device paired via QR (Register flow) |
| `auth_success` | `is_new_client`, `duration_ms`, `path_type` | Auth phase completes |
| `auth_failed` | `reason` | Auth rejected for any reason |
| `session_end` | `duration_ms`, `terminal_count`, `path_type` | Client disconnects |
| `terminal_open` | `has_launch_cmd` | `TermCreate` RPC succeeds |
| `bandwidth_sample` | `bytes_sent`, `bytes_recv`, `interval_secs=60` | Every 60s while connected |

`path_type` is `"direct"` (P2P), `"relay"`, or `"unknown"` (path not yet determined at auth time).

### Privacy

No personal data is collected: no paths, hostnames, usernames, IP addresses, or file content. The host's Ed25519 cryptographic identity is never sent. Only behavioral counters and timing are tracked.

---

## Summary of Files Changed/Created

| File | Action | Notes |
|------|--------|-------|
| `android/google-services.json` | Add (from Firebase console) | Required for Android SDK |
| `ios/Zedra/GoogleService-Info.plist` | Add (from Firebase console) | Required for iOS SDK |
| `android/build.gradle` | Modify | Add plugins + dependencies + NDK config |
| `android/app/src/main/java/dev/zedra/app/ZedraFirebase.java` | Create | Java Firebase wrapper |
| `android/app/src/main/java/dev/zedra/app/MainActivity.java` | Modify | Add `ZedraFirebase.initialize(this)` |
| `ios/Podfile` | Create | CocoaPods Firebase deps |
| `ios/project.yml` | Modify | Add `podfile:` option |
| `ios/Zedra/ZedraFirebase.m` | Create | Obj-C Firebase bridge |
| `ios/Zedra/main.m` | Modify | Add `zedra_firebase_initialize()` call |
| `ios/ZedraFFI.xcframework/*/Headers/zedra_ios.h` | Modify | Add analytics FFI declarations |
| `crates/zedra/src/android/analytics.rs` | Create | Rust→JNI bridge |
| `crates/zedra/src/android/mod.rs` | Modify | Add `pub mod analytics` |
| `crates/zedra/src/ios/analytics.rs` | Create | Rust→FFI bridge |
| `crates/zedra/src/ios/mod.rs` | Modify | Add `pub mod analytics` |
| `crates/zedra/src/analytics.rs` | Create | Shared platform-agnostic API |
| `crates/zedra/src/lib.rs` | Modify | Add `pub mod analytics` + update panic hook |
| `scripts/build-android.sh` | Modify | Preserve unstripped `.so` for release |

---

## Crash Coverage

| Crash type | Captured by | Notes |
|------------|-------------|-------|
| Native SIGSEGV / SIGABRT in `.so` | Crashlytics NDK signal handler | Automatic, no code needed |
| Rust `panic!` in release (`panic = "abort"`) | Crashlytics NDK signal handler | `abort()` is a native signal |
| Rust `panic!` in debug | `install_panic_hook()` → `record_panic()` | Reported as non-fatal |
| Rust `anyhow::Error` propagated to top level | Manual call to `analytics::record_error()` | Add at session/transport boundaries |
| Java exception in Firebase wrapper | Crashlytics Java handler | Default Firebase behavior |
| ObjC exception in iOS bridge | Crashlytics default handler | Default Firebase behavior |

---

## Notes

- **`panic = "abort"` in release**: This is already set in the workspace `Cargo.toml`. In release, all Rust panics directly call `abort()`, which the Crashlytics NDK signal handler captures as a native crash with a full stack trace. The panic hook forwarding to the Java/ObjC API is therefore only needed in debug builds.
- **Thread safety**: The `with_env()` helper in `android/analytics.rs` attaches the calling thread to the JVM if needed, matching the same pattern used by `show_keyboard_inner()`. Firebase Android SDK methods are thread-safe.
- **Symbol upload timing**: `nativeSymbolUploadEnabled true` uploads symbols during the `assembleRelease` Gradle task. CI pipelines should run `./gradlew assembleRelease` to trigger uploads. Debug builds do not upload.
- **Google Services plugin placement**: The `google-services.json` must be at the same level as the `build.gradle` that applies `com.google.gms.google-services`. In this project that is `android/google-services.json`.
