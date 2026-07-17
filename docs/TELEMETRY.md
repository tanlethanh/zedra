# Telemetry Architecture

Zedra uses a shared telemetry crate (`zedra-telemetry`) to emit typed, privacy-safe
events from both the mobile app and the desktop host daemon. Events flow through
platform-specific backends registered at startup.

---

## Architecture

```
zedra-telemetry (pure crate, no platform deps)
  |
  |-- Event enum          typed variants with context structs
  |-- TelemetryBackend    trait: send(), record_panic(), ...
  |-- send(Event)         global free function, delegates to backend
  |-- init(backend)       called once at startup
  |
  +-- App backend: FirebaseBackend  (crates/zedra/src/telemetry.rs)
  |     iOS:     extern "C" FFI --> Firebase iOS SDK
  |     Android: JNI --> ZedraFirebase.kt --> Firebase Android SDK
  |
  +-- Host backend: HostBackend  (crates/zedra-host/src/telemetry.rs)
        Wraps Ga4 transport --> tokio::spawn --> HTTPS POST
```

### Non-blocking guarantee

`TelemetryBackend::send()` **must never block** the calling thread.

- **Firebase SDK** (iOS/Android): `logEvent` queues internally — the FFI/JNI call
  returns immediately.
- **GA4 HostBackend**: calls `Ga4::track_raw()` which does `tokio::spawn` —
  the HTTP POST runs in the background.
- **`record_panic`** is the one exception: it MAY block briefly (synchronous HTTP
  with 3s timeout) to flush the event before the process aborts.

Telemetry calls appear inline in app logic and RPC handlers. They add negligible
overhead and never delay rendering, connection setup, or terminal I/O.

### Opt-out

- **App (persisted)**: Settings → Privacy → "Share usage data" toggle. Stored as
  `telemetry_enabled` in `settings.json` (`crate::settings`). Absent/`None` = enabled
  (opt-out model). On launch, `crate::telemetry::apply_persisted_optout()` reads the
  setting and calls `zedra_telemetry::set_enabled(...)` **after** the platform bridge is
  set and **before** the first `AppOpen`, so an opted-out user emits no events at all.
- **App (Firebase default-off)**: the Firebase SDK is configured to **not** auto-collect
  at init — iOS `Info.plist`/`project.yml` (`FIREBASE_ANALYTICS_COLLECTION_ENABLED=NO`,
  `FirebaseCrashlyticsCollectionEnabled=NO`) and Android manifest placeholders
  (`firebase_analytics_collection_enabled=false`, `firebase_crashlytics_collection_enabled=false`,
  even in release). `apply_persisted_optout()` re-enables collection at runtime only when
  the user is opted-in, via `set_enabled(true)` → `set_collection_enabled(true)`.
- **App (runtime)**: `zedra_telemetry::set_enabled(false)` flips an `AtomicBool`, all
  subsequent telemetry operations become no-ops, and disables Firebase SDK collection.
  The Settings toggle updates the single persisted preference through
  `settings::set_telemetry_enabled(...)`.
- **Host (runtime)**: `--no-telemetry` flag or `ZEDRA_TELEMETRY=0` env var.
  `telemetry_disabled()` gates `new_ga4()` before any event fires.

### Exclude telemetry completely when building from source

For the desktop host daemon, use the explicit `no-telemetry` feature:

```sh
# Desktop host daemon: removes GA4 code, credentials, and send paths.
cargo build --release -p zedra-host --features no-telemetry
```

For mobile, use a supported run script with `--no-telemetry`:

```sh
# iOS simulator or connected device: compiles out Firebase and builds, installs, and launches the app.
./scripts/run-ios.sh sim --no-telemetry
./scripts/run-ios.sh device --no-telemetry

# Android connected device: compiles out Firebase and builds, installs, and launches the app.
./scripts/run-android.sh device --no-telemetry
```

The run scripts forward the flag to both the Rust and native app builds. Calling
`build-ios.sh` or `build-android.sh` alone produces only Rust artifacts; using one
before a separate Xcode or Gradle app build is not a supported opt-out workflow.

The mobile build keeps the Privacy row visible as a muted, disabled Off control
with a build-disabled explanation. It does not initialize or call Firebase Analytics
or Crashlytics. Android also omits those dependencies; Firebase Messaging remains
available for Delta push notifications.

---

## Event Catalog

### App events (mobile + shared crates)

Emitted by `zedra` (app crate) and `zedra-session`.

| Event | Context fields | Emitted from |
|-------|---------------|-------------|
| `app_open` | `saved_workspaces`, `app_version`, `platform`, `arch` | `app.rs` — cold start |
| `screen_view` | `screen`, `screen_name`, `screen_class` | `app.rs`, `workspace.rs`, `workspace_drawer.rs`, `workspace_terminal.rs`, `settings_view.rs` — GPUI logical views |
| `qr_scan_initiated` | — | `app.rs` — Scan QR tapped |
| `connect_success` | `total_ms`, `binding_ms`, `hole_punch_ms`, `resolve_ms`, `handshake_ms`, `auth_ms`, `fetch_ms`, `path`, `network`, `rtt_ms`, `relay`, `relay_latency_ms`, `alpn`, `has_ipv4`, `has_ipv6`, `symmetric_nat`, `is_first_pairing` | `handle.rs` — connect() (`hole_punch_ms` = `resolve_ms` discovery + `handshake_ms` QUIC) |
| `connect_failed` | `phase`, `error`, `elapsed_ms`, `relay`, `alpn`, `has_ipv4`, `has_ipv6`, `relay_connected` | `handle.rs` — set_failed() |
| `session_resumed` | `terminal_count`, `resume_ms` | `app.rs` — reconnect to existing session |
| `disconnect` | — | `app.rs` — user disconnect |
| `workspace_selected` | `source` (`"active"` or `"saved"`) | `app.rs` — workspace tapped |
| `reconnect_started` | `reason` | `handle.rs` — reconnect loop |
| `reconnect_success` | `attempt`, `elapsed_ms`, `reason`, `binding_ms`, `hole_punch_ms`, `resolve_ms`, `handshake_ms`, `auth_ms`, `fetch_ms`, `path`, `network`, `rtt_ms`, `relay`, `alpn`, `has_ipv4`, `has_ipv6` | `handle.rs` |
| `reconnect_exhausted` | `attempts`, `elapsed_ms`, `reason`, `fatal_error` (optional) | `handle.rs` |
| `path_upgraded` | `network`, `rtt_ms`, `from_relay`, `upgrade_ms` (relay→direct time) | `handle.rs` — path watcher |
| `direct_upgrade_timeout` | `elapsed_ms`, `relay`, `network`, `symmetric_nat` | `workspace.rs` — no direct path within the upgrade window (relay-only) |
| `connection_latency_sample` | `source`, `connection_type`, `network_type`, `rtt_ms`, `relay`, `relay_region`, `nearest_relay_region`, `path_count`, `interval_secs`, `sample_reason` | `workspace.rs` — selected-path latency sample |
| `terminal_opened` | `source`, `terminal_count` | `workspace_view.rs` |
| `terminal_closed` | `remaining` | `workspace_view.rs` |

### Host events (desktop daemon)

Emitted by `zedra-host` via the same `zedra_telemetry::send()` mechanism.

| Event | Context fields | Emitted from |
|-------|---------------|-------------|
| `daemon_start` | `relay_type`, `is_first_run` | `main.rs` — `zedra start` |
| `net_report` | `has_ipv4`, `has_ipv6`, `symmetric_nat` | `iroh_listener.rs` — STUN complete |
| `client_paired` | — | `rpc_daemon.rs` — QR Register flow |
| `auth_success` | `is_new_client`, `duration_ms`, `path_type` | `rpc_daemon.rs` — auth complete |
| `auth_failed` | `reason` | `rpc_daemon.rs` — auth rejected |
| `session_end` | `duration_ms`, `terminal_count`, `path_type` | `rpc_daemon.rs` — client disconnect |
| `terminal_open` | `has_launch_cmd` | `rpc_daemon.rs` — PTY spawned |
| `bandwidth_sample` | `bytes_sent`, `bytes_recv`, `interval_secs` | `rpc_daemon.rs` — every 60s |
| `connection_latency_sample` | `source`, `connection_type`, `network_type`, `rtt_ms`, `relay`, `relay_region`, `nearest_relay_region`, `path_count`, `interval_secs`, `sample_reason` | `rpc_daemon.rs` — every 60s while connected |

Host events also carry `host_version`, `os`, and `arch` — injected automatically
by `Ga4::build_payload()` before the GA4 HTTP POST.

`connection_latency_sample` is a point-in-time selected-path sample, not a
session average. The app emits an initial sample after connection, another when
the selected path type changes, and then periodic samples on the configured
interval. The host emits the same event every 60 seconds from the active RPC
connection. Relay values are sanitized to known relay IDs (`sg1`, `vn1`, `us1`,
`eu1`) or `custom`; no arbitrary relay hostname, IP address, or geolocation is
sent. `nearest_relay_region` is inferred from the preferred relay reported by
iroh, not from user IP lookup.

---

## Privacy Rules

- **Never** include personal data: usernames, file paths, file contents, IP addresses, hostnames.
- Use opaque IDs only (node ID short forms, session IDs, terminal IDs).
- Durations, counts, enum labels, and boolean flags are always safe.
- `record_panic` strips filesystem paths via `sanitize_panic_message()`.

---

## Adding Events for New Features

Any change that adds a **user-facing feature or significant behavior** MUST
define telemetry events:

1. **Add a typed `Event` variant** in `crates/zedra-telemetry/src/lib.rs` with a
   dedicated context struct. Place it in the correct section (App or Host).
2. **Implement `to_params()`** for the new variant.
3. **Instrument the call site** using `zedra_telemetry::send(Event::Variant(...))`.
4. **Include meaningful context**: connection timing/phase, transport path, relay,
   ALPN, network classification, version info — whatever helps understand the
   feature's behavior in production.

---

## Setup: Firebase (App)

### iOS

Firebase is integrated via CocoaPods. Key files:

| File | Purpose |
|------|---------|
| `ios/Podfile` | `FirebaseAnalytics`, `FirebaseCrashlytics` pods |
| `ios/Zedra/GoogleService-Info.plist` | Firebase project config (from console); optional for Debug, required for Release |
| `ios/project.yml` | XcodeGen spec that bundles Firebase config when present and fails Release builds when it is missing |
| `ios/Zedra/NativeBridge.swift` | Swift C-export bridge: `zedra_firebase_initialize`, `zedra_log_event`, `zedra_record_error`, `zedra_record_panic` |
| `crates/zedra/src/ios/telemetry.rs` | Rust FFI declarations calling into `NativeBridge.swift` |
| `crates/zedra/src/telemetry.rs` | `FirebaseBackend` — registers with `zedra_telemetry` |

Manual GPUI logical screen views are emitted as Firebase `screen_view` events
with `screen_name` and `screen_class`. Native automatic screen reporting is disabled
on both platforms because GPUI, not native view controllers, owns Zedra's screens.

**Build**: `pod install` in `ios/`, then build via `.xcworkspace`.
Debug builds can run without `ios/Zedra/GoogleService-Info.plist`; Firebase
initialization and event calls become no-ops. Release builds require the plist
and fail during the Xcode build when it is absent.

### Android

| File | Purpose |
|------|---------|
| `android/google-services.json` | Release Firebase project config for `dev.zedra.app` (ignored; add locally or in CI secrets) |
| `android/build.gradle` | Firebase BoM, `firebase-analytics`, `firebase-crashlytics`, `firebase-crashlytics-ndk`; release-only Google Services and Crashlytics Gradle plugins |
| `android/app/src/main/kotlin/dev/zedra/app/ZedraFirebase.kt` | Kotlin wrapper: `logEvent`, `recordError`, `recordPanic`, SDK collection toggles |
| `crates/zedra/src/android/telemetry.rs` | Rust JNI bridge |
| `android/app/proguard-rules.pro` | R8 keep rules for the app JNI boundary package |

`android/google-services.json` only needs the release client
`dev.zedra.app`. Firebase Gradle plugins are applied only for release tasks; the
debug build includes the Firebase SDK but disables Analytics, Crashlytics, and
automatic screen reporting through manifest placeholders and the
`BuildConfig.DEBUG` guard in `ZedraFirebase`. Debug does not require a Firebase
client for `dev.zedra.app.debug`. Android uses `minSdk 23` because current
Firebase Crashlytics and Crashlytics NDK require API 23 or newer.

Android release builds are signed from normal Gradle properties. Put
`ZEDRA_KEYSTORE`, `ZEDRA_KEYSTORE_ALIAS`, and `ZEDRA_KEYSTORE_PASSWORD` in
`~/.gradle/gradle.properties` or pass them with `-P`. `ZEDRA_KEY_PASSWORD` is
optional and defaults to `ZEDRA_KEYSTORE_PASSWORD`.

Rust looks up app Kotlin classes such as `dev/zedra/app/MainActivity` and
`dev/zedra/app/ZedraFirebase` by exact JNI class/name/signature.
`android/app/proguard-rules.pro` keeps `dev.zedra.app.**` and
`dev.zed.gpui.**` stable in release so alerts, selections, keyboard toggles,
sheets, notifications, Firebase calls, GPUI lifecycle callbacks, and
Kotlin-to-Rust callbacks do not depend on R8's rewritten names. After
`assembleRelease`, confirm the release mapping still preserves those packages.

### Crashlytics NDK (release builds)

For Rust `.so` crash symbolication, the unstripped `.so` must be preserved
before Gradle strips it. `build-android.sh` copies it to a staging dir, and
the `firebaseCrashlytics` Gradle block uploads it during `assembleRelease`.

---

## Setup: GA4 Measurement Protocol (Host)

The host daemon sends events via GA4 Measurement Protocol over HTTPS. No
Firebase SDK required.

### Credentials

Two **compile-time** environment variables:

```
ZEDRA_GA_MEASUREMENT_ID=G-XXXXXXXXXX
ZEDRA_GA_API_SECRET=your_secret
```

Get the API secret from: Firebase console -> GA4 property -> Data Streams ->
Web stream -> Measurement Protocol API secrets.

If either variable is absent or empty, telemetry is silently disabled (no HTTP
calls, no overhead). Source builds without credentials send no data.

### GitHub Actions release workflow

The release workflow (`.github/workflows/release.yml`) builds `zedra-host`
for distribution. To enable telemetry in release builds, add the credentials
as GitHub repository secrets and pass them as env vars during the build step:

```yaml
- name: Build
  env:
    ZEDRA_GA_MEASUREMENT_ID: ${{ secrets.ZEDRA_GA_MEASUREMENT_ID }}
    ZEDRA_GA_API_SECRET: ${{ secrets.ZEDRA_GA_API_SECRET }}
  run: |
    cargo build -p zedra-host --release --target ${{ matrix.target }}
```

**Required GitHub secrets:**

| Secret | Value |
|--------|-------|
| `ZEDRA_GA_MEASUREMENT_ID` | GA4 Measurement ID (e.g. `G-XXXXXXXXXX`) |
| `ZEDRA_GA_API_SECRET` | Measurement Protocol API secret |

Without these secrets, CI builds produce binaries with telemetry disabled.

### Telemetry ID

A random UUID is generated on first run and stored at `~/.config/zedra/telemetry_id`.
It is machine-level (shared across workspaces), never tied to cryptographic identity,
and never transmitted alongside it.

---

## Key Files

| File | Role |
|------|------|
| `crates/zedra-telemetry/src/lib.rs` | `Event` enum, `TelemetryBackend` trait, `send()`, `init()` |
| `crates/zedra/src/telemetry.rs` | `FirebaseBackend` — registers Firebase with `zedra-telemetry` |
| `crates/zedra/src/ios/telemetry.rs` | iOS FFI: Rust -> `NativeBridge.swift` -> Firebase SDK |
| `crates/zedra/src/android/telemetry.rs` | Android JNI: Rust -> `ZedraFirebase.kt` -> Firebase SDK |
| `ios/project.yml` | iOS Firebase config bundling and Release validation |
| `crates/zedra-host/src/telemetry.rs` | `HostBackend` — bridges GA4 `Ga4` <-> `zedra-telemetry` |
| `crates/zedra-host/src/ga4.rs` | GA4 Measurement Protocol transport (`track_raw`, `host_panic_sync`) |

---

## Crash Coverage

| Crash type | Captured by |
|------------|-------------|
| Native SIGSEGV/SIGABRT in `.so` | Crashlytics NDK signal handler (automatic) |
| Rust `panic!` in release (`panic = "abort"`) | Crashlytics NDK signal handler |
| Rust `panic!` in debug | `install_panic_hook()` -> `record_panic()` |
| Recoverable errors | Manual `zedra_telemetry::record_error()` at boundaries |
