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
  |     Android: JNI --> ZedraFirebase.java --> Firebase Android SDK
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

### Runtime opt-out

- **App**: `zedra_telemetry::set_enabled(false)` — flips an `AtomicBool`, all
  subsequent `send()` calls become no-ops. Also disables Firebase SDK collection.
- **Host**: `--no-telemetry` flag or `ZEDRA_TELEMETRY=0` env var.

---

## Event Catalog

### App events (mobile + shared crates)

Emitted by `zedra` (app crate) and `zedra-session`.

| Event | Context fields | Emitted from |
|-------|---------------|-------------|
| `app_open` | `saved_workspaces`, `app_version`, `platform`, `arch` | `app.rs` — cold start |
| `screen_view` | `screen` | `app.rs` — navigation |
| `qr_scan_initiated` | — | `app.rs` — Scan QR tapped |
| `connect_success` | `total_ms`, `binding_ms`, `hole_punch_ms`, `auth_ms`, `fetch_ms`, `path`, `network`, `rtt_ms`, `relay`, `relay_latency_ms`, `alpn`, `has_ipv4`, `has_ipv6`, `symmetric_nat`, `is_first_pairing` | `handle.rs` — connect() |
| `connect_failed` | `phase`, `error`, `elapsed_ms`, `relay`, `alpn`, `has_ipv4`, `has_ipv6`, `relay_connected` | `handle.rs` — set_failed() |
| `session_resumed` | `terminal_count`, `resume_ms` | `app.rs` — reconnect to existing session |
| `disconnect` | — | `app.rs` — user disconnect |
| `workspace_selected` | `source` (`"active"` or `"saved"`) | `app.rs` — workspace tapped |
| `reconnect_started` | `reason` | `handle.rs` — reconnect loop |
| `reconnect_success` | `attempt`, `elapsed_ms`, `reason`, `binding_ms`, `hole_punch_ms`, `auth_ms`, `fetch_ms`, `path`, `network`, `rtt_ms`, `relay`, `alpn`, `has_ipv4`, `has_ipv6` | `handle.rs` |
| `reconnect_exhausted` | `attempts`, `elapsed_ms`, `reason`, `fatal_error` (optional) | `handle.rs` |
| `path_upgraded` | `network`, `rtt_ms`, `from_relay` | `handle.rs` — path watcher |
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

Host events also carry `host_version`, `os`, and `arch` — injected automatically
by `Ga4::build_payload()` before the GA4 HTTP POST.

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
| `ios/Zedra/GoogleService-Info.plist` | Firebase project config (from console) |
| `ios/Zedra/NativeBridge.swift` | Swift C-export bridge: `zedra_firebase_initialize`, `zedra_log_event`, `zedra_record_error`, `zedra_record_panic` |
| `crates/zedra/src/ios/telemetry.rs` | Rust FFI declarations calling into `NativeBridge.swift` |
| `crates/zedra/src/telemetry.rs` | `FirebaseBackend` — registers with `zedra_telemetry` |

**Build**: `pod install` in `ios/`, then build via `.xcworkspace`.

### Android (planned)

| File | Purpose |
|------|---------|
| `android/google-services.json` | Firebase project config |
| `android/build.gradle` | `firebase-analytics`, `firebase-crashlytics`, `firebase-crashlytics-ndk` |
| `android/.../ZedraFirebase.java` | Java wrapper: `logEvent`, `recordError`, `recordPanic` |
| `crates/zedra/src/android/telemetry.rs` | Rust JNI bridge |

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
