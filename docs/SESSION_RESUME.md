# iOS Session Resume (Foreground ↔ Background)

## Problem

When the iOS app moves to the background, iOS suspends the process after ~10 seconds.
This kills the UDP sockets underlying the iroh QUIC connection. The tokio runtime
freezes, so the connection watcher (`conn.closed().await`) doesn't fire until the app
returns to the foreground and tokio resumes. Combined with the reconnect loop's
exponential backoff (starting at 1s), the user experiences a multi-second delay before
the session recovers.

## What Survives Backgrounding

The following state persists in memory (process is suspended, not killed):

| State | Location | Survives? |
|---|---|---|
| Terminal output buffers | `SessionHandle` (Arc/Mutex statics) | Yes |
| Session credentials (id + token) | `SessionHandle.credentials` | Yes |
| Endpoint address | `SessionHandle.endpoint_addr` | Yes |
| Terminal IDs + active terminal | `SessionHandle` | Yes |
| Server-side PTYs | `SessionRegistry` (host) | Yes (grace period) |
| Server notification backlog | Per-terminal, up to 1000 entries | Yes |
| iroh QUIC connection | UDP socket | **No** — killed by iOS |
| tokio runtime state | In-memory | Yes (frozen, resumes) |

## Implementation: Phase 1 — Immediate Reconnect on Foreground

### Architecture

```
UIKit lifecycle → GPUI on_active_status_change → ZedraApp callback
  → zedra_session::notify_foreground_resume(handle) per workspace
    → sets skip_next_backoff flag
    → calls spawn_reconnect (no-op if already running)
```

### Flow

1. **App backgrounds** → `applicationDidEnterBackground` → CADisplayLink stops →
   iOS suspends process after ~10s → UDP sockets die

2. **App foregrounds** → `applicationWillEnterForeground` → CADisplayLink resumes →
   GPUI fires `on_active_status_change(true)` → `activation_observers` run

3. **ZedraApp receives activation** → iterates all workspaces → calls
   `notify_foreground_resume(&handle)` for each

4. **Per-handle logic**:
   - If `user_disconnect` flag is set → skip (user intentionally disconnected)
   - Set `skip_next_backoff = true` on the handle
   - If a reconnect loop is already running (CAS fails) → return (the loop will
     skip its next backoff sleep thanks to the flag)
   - If no reconnect loop is running → `spawn_reconnect(handle)` starts one, which
     immediately skips the first backoff delay

5. **Reconnect loop** checks `skip_next_backoff` before each sleep. If true, clears
   the flag and attempts immediately (0ms delay instead of 1s+).

6. **On success** → `connect_with_iroh` creates a fresh iroh Endpoint + QUIC connection,
   resumes the RPC session with stored credentials, reattaches all terminals with
   backlog replay via `last_seq`.

### Key Files

| File | Change |
|---|---|
| `crates/zedra-session/src/lib.rs` | `skip_next_backoff` field on `SessionHandleInner`, `notify_foreground_resume()` public fn, backoff skip logic in `spawn_reconnect` |
| `crates/zedra/src/app.rs` | `observe_window_activation` in `ZedraApp::new()`, calls `notify_foreground_resume` for each workspace |

### Timing

- **Before**: resume → tokio detects dead conn (~0-2s) → backoff 1s → connect attempt = **1-3s**
- **After**: resume → activation callback (~0ms) → skip backoff → connect attempt = **<1s**

## Future Phases

### Phase 2: Background Task Extension

Use `UIApplication.beginBackgroundTask` to get ~30s of execution when entering background.
Gracefully close the iroh connection so the server knows immediately (instead of waiting
for its own timeout). End the background task after the close handshake completes.

### Phase 3: Fresh Endpoint Recovery

If the iroh Endpoint can't recover its UDP socket after suspension, detect this and
create a new Endpoint. The server authenticates via `session_id + auth_token`, not
transport identity, so a new Endpoint works transparently.

### Phase 4: Network Path Monitoring

Register `NWPathMonitor` to detect WiFi ↔ cellular transitions during background.
Proactively trigger reconnect on network path changes.
