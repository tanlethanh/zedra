# Architecture

Zedra connects an Android client to a desktop host daemon for remote terminal, filesystem, git, and development tool access. All networking runs through [iroh](https://iroh.computer/) (QUIC/TLS 1.3) with automatic NAT traversal and relay fallback.

## System Overview

```
Android (zedra)                              Desktop (zedra-host)
+-----------------+                          +-------------------+
| GPUI + Vulkan   |                          | RPC Daemon        |
| TerminalView    |   iroh (QUIC/TLS 1.3)   | SessionRegistry   |
| FileExplorer    | <======================> | PTY Management    |
| QR Scanner      |   NAT traversal, relay   | Git / FS / LSP    |
+-----------------+                          +-------------------+
        |                                            |
        |          CF Worker (relay.zedra.dev)        |
        +----------> Endpoint Discovery <------------+
                     WebSocket Relay
```

## Crate Structure

```
crates/
  zedra-rpc/           irpc protocol types, QR pairing codec (postcard + base64-url)
  zedra-host/          Desktop daemon: iroh listener, irpc dispatch, session registry, host identity
  zedra-session/       Mobile client: iroh connection, RPC session, terminal buffers, auto-reconnect
  zedra-terminal/      Terminal emulation (alacritty) + GPUI rendering
  zedra/               Android cdylib: JNI bridge, GPUI app, touch handling, editor, navigation

packages/
  relay-worker/        Cloudflare Worker: endpoint discovery + WebSocket relay
```

### Dependency Graph

```
zedra-rpc          (irpc protocol types, QR pairing codec)
    ^
zedra-session      (RemoteSession, iroh connection, terminal output buffers)
    ^
zedra-terminal     (TerminalState, TerminalElement, TerminalView)
    ^
zedra              (Android cdylib, GPUI app, JNI bridge)

zedra-host         (iroh listener, irpc RPC daemon, session registry, host identity)
    ^
zedra-rpc
```

---

## Transport Layer

All connectivity goes through a single iroh Endpoint per device. iroh handles path selection (LAN, hole-punched, relay) internally.

### Identity (`zedra-host/src/identity.rs`)

The host has a persistent Ed25519 keypair stored at `~/.config/zedra/workspaces/<hash>/identity.key` (32-byte secret, per-workspace) or `~/.config/zedra/identity.key` (base identity for `zedra qr` without `--workdir`). Each `zedra-host start --workdir` gets its own identity so multiple instances on the same machine have distinct iroh NodeIds. The key is used directly as the iroh Endpoint secret key. Identity is host-only — the mobile client uses an ephemeral iroh Endpoint.

### Pairing (`zedra-rpc/src/pairing.rs`)

QR code contains a compact binary encoding of the host's `iroh::EndpointAddr`:

```
postcard::to_allocvec(&addr) -> base64_url::encode() -> QR code string (~50 bytes)
```

The `EndpointAddr` contains the host's endpoint ID (Ed25519 public key), relay URL, and direct socket addresses. No metadata (hostname, device name) is included — this is discovered post-connection via `GetSessionInfo` RPC.

- `encode_endpoint_addr(&EndpointAddr) -> String` — for QR generation
- `decode_endpoint_addr(&str) -> EndpointAddr` — for QR scanning

---

## RPC Layer (`zedra-rpc`)

Type-safe RPC using [irpc](https://docs.rs/irpc) with postcard binary serialization over QUIC.

### Protocol (`proto.rs`)

Defines `ZedraProto` service with typed request/response pairs:

| Category   | RPC Methods                                                                        |
| ---------- | ---------------------------------------------------------------------------------- |
| Session    | `ResumeOrCreate`, `SessionInfo`                                                    |
| Filesystem | `FsList`, `FsRead`, `FsWrite`, `FsStat`, `FsRemove`, `FsMkdir`                    |
| Terminal   | `TermCreate`, `TermAttach` (bidi stream), `TermResize`, `TermClose`, `TermList`    |
| Git        | `GitStatus`, `GitDiff`, `GitLog`, `GitCommit`, `GitBranches`, `GitCheckout`        |

Terminal I/O uses bidi streaming (`TermAttach`) — input and output flow concurrently over a single QUIC stream.

---

## Host Daemon (`zedra-host`)

Desktop process that listens for incoming iroh connections and dispatches RPC operations.

### Startup Flow (`main.rs`)

```
1. Load/generate per-workspace host identity (~/.config/zedra/workspaces/<hash>/identity.key)
2. Create named session for working directory
3. Bind iroh Endpoint with host's SecretKey
4. Generate pairing QR code (compact EndpointAddr: postcard + base64-url)
5. Spawn session cleanup loop (every 60s, 5-min grace period)
6. Run iroh accept loop (blocks main thread)
```

### Iroh Listener (`iroh_listener.rs`)

- **ALPN**: `zedra/rpc/2`
- **Relay**: `RelayMode::custom([DEFAULT_RELAY_URL])` — uses `relay.zedra.dev` for NAT traversal
- `create_endpoint()` — builds iroh Endpoint with host's SecretKey, waits for relay connection (10s timeout)
- `run_accept_loop()` — accepts connections, spawns handler per connection
- No ping loop — QUIC native keepalive (5s interval) handles liveness

### RPC Dispatch (`rpc_daemon.rs`)

`handle_connection(conn, registry, state)` is the entry point for each iroh connection:

1. First message must be `ResumeOrCreate` for session binding
2. Dispatch loop reads typed irpc messages and spawns handlers
3. `TermAttach` uses bidi streaming — PTY output flows to client via `tx`, input flows from client via `rx`
4. On disconnect: terminal streams are dropped, session persists for reconnect
5. `TermList` returns active terminal IDs for reconnect reconciliation

### Session Registry (`session_registry.rs`)

Sessions persist independently of transport connections:

```
Client connects     -> session/resume_or_create -> ServerSession created
Client disconnects  -> session stays alive (5-min grace period)
                       PTY processes keep running
                       Terminal output buffered in notification backlog
Client reconnects   -> session/resume_or_create with session_id + auth_token
                       Reattach to same PTY, replay missed notifications
```

**ServerSession** holds:

- `id` (UUID), `auth_token`
- `terminals: HashMap<String, TermSession>` (active PTYs)
- `notification_backlog: VecDeque<(seq, payload)>` (capped at 1000 entries)
- `next_notif_seq: u64` (monotonic counter)

Named sessions map human-readable names to session IDs for persistent workdir access.

### CLI

```
zedra-host start [--workdir .] [--json]   # Start daemon, show QR
zedra-host qr                              # Show pairing QR code
```

---

## Session Client (`zedra-session`)

Mobile client library for connecting to zedra-host via iroh and issuing RPC calls.

### Connection (`connect_with_iroh`)

```
1. Store EndpointAddr in ENDPOINT_ADDR (for reconnect)
2. Reset USER_DISCONNECT flag
3. Create ephemeral iroh Endpoint (client side, relay: relay.zedra.dev)
4. endpoint.connect(addr, b"zedra/rpc/2")
   iroh: direct -> hole-punch -> relay
5. Wrap connection as irpc Client<ZedraProto>
6. Use persistent terminal output buffers (PERSISTENT_TERMINAL_OUTPUTS)
   so UI views survive reconnect via shared Arc references
7. ResumeOrCreate RPC with SESSION_CREDENTIALS
8. For each terminal: TermAttach bidi stream (replay missed output, then live I/O)
9. Spawn path watcher (tracks direct vs relay, RTT)
```

No ping loop — QUIC native keepalive handles liveness.

### Global State (OnceLock singletons)

- `ACTIVE_SESSION` — current `RemoteSession`
- `SESSION_RUNTIME` — dedicated tokio runtime (2 worker threads)
- `MAIN_THREAD_CALLBACKS` — deferred work queue for GPUI main thread
- `TERMINAL_DATA_PENDING` — atomic flag, signaled by terminal stream, polled by GPUI frame loop

Reconnect state (persists across `RemoteSession` rebuilds):

- `RECONNECT_ATTEMPT` (AtomicU32) — current attempt (0 = not reconnecting)
- `USER_DISCONNECT` (AtomicBool) — set on user disconnect, prevents auto-reconnect
- `ENDPOINT_ADDR` — stored EndpointAddr for reconnect attempts
- `SESSION_CREDENTIALS` — (session_id, auth_token) for session resumption
- `PERSISTENT_TERMINAL_OUTPUTS` — shared output buffers that survive reconnect
- `PERSISTENT_TERMINAL_IDS` — terminal ID list that survives reconnect
- `PERSISTENT_ACTIVE_TERMINAL` — active terminal ID that survives reconnect

### Terminal Output Flow

```
Host PTY reader -> TermAttach bidi stream tx (raw bytes + seq)
    -> per-terminal OutputBuffer (persistent Arc)
    -> TERMINAL_DATA_PENDING atomic flag
    -> GPUI frame loop polls flag -> drains buffer
    -> TerminalState.advance_bytes() -> re-render
```

### Auto-Reconnect Flow

```
Connection drops -> TermAttach stream ends / connection closed
    -> spawn_reconnect()
       Guard: USER_DISCONNECT? abort
       Guard: CAS RECONNECT_ATTEMPT 0->1 (prevent double-reconnect)
       Loop (max 20 attempts, ~5 min matching server grace period):
         1. Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s cap
         2. Signal TERMINAL_DATA_PENDING -> UI shows "Reconnecting... (N)"
         3. connect_with_iroh(stored EndpointAddr)
            -> new irpc Client<ZedraProto> per reconnect
            -> ResumeOrCreate with stored credentials
            -> TermAttach for each terminal (replays missed output via last_seq)
         4. On success: set_active_session(), terminal_list() reconcile
         5. On failure: increment attempt, continue
```

---

## Relay Worker (`packages/relay-worker`)

Cloudflare Worker implementing an iroh-compatible relay server at `relay.zedra.dev`. Standard iroh clients connect directly — no fork needed.

### Bindings

- `ZEDRA_RELAY_KV` -- KV namespace for endpoint routing table
- `ZEDRA_RELAY_ENDPOINT` -- Durable Object for per-endpoint WebSocket relay

### HTTP Endpoints

| Method | Path             | Description                                          |
| ------ | ---------------- | ---------------------------------------------------- |
| `GET`  | `/`              | Health check (`{"ok": true}`)                        |
| `GET`  | `/ping`          | HTTPS probe for iroh `net_report` (latency measurement) |
| `GET`  | `/generate_204`  | Captive portal detection (returns 204)               |
| `GET`  | `/relay`         | WebSocket upgrade to `RelayEndpoint` Durable Object  |

### WebSocket Relay (Durable Object: `RelayEndpoint`)

Each iroh `Endpoint` gets its own DO instance. After Ed25519 challenge-response handshake, the DO registers the endpoint in KV (`relay:ep:{pubkey_hex}`, 90s TTL). Datagrams are forwarded between DOs via internal `POST /forward` calls.

See `docs/RELAY.md` for full wire protocol details.

---

## Android App (`zedra`)

GPUI on Android via Vulkan 1.1 with a command queue threading architecture.

### Threading Model

```
JNI Thread (any) -> AndroidCommandQueue (crossbeam channel)
                        -> Main Thread drains @ 60 FPS via Choreographer
                        -> GPUI (single-threaded) -> wgpu -> Vulkan
```

### Key Components

| Component      | File                        | Purpose                                                    |
| -------------- | --------------------------- | ---------------------------------------------------------- |
| JNI Bridge     | `android/jni.rs`            | Java-Rust interface, thread-safe command queue             |
| Android App    | `android/app.rs`            | Main-thread GPUI state, surface/window lifecycle           |
| Command Queue  | `android/command_queue.rs`  | Crossbeam mpsc channel for JNI decoupling                  |
| Touch Handler  | `android/touch.rs`          | Tap, scroll, drawer pan, fling momentum                    |
| Platform Bridge| `platform_bridge.rs`        | PlatformBridge trait (density, keyboard show/hide)         |
| Zedra App      | `app.rs`                    | Root view: DrawerHost + screens + transport/reconnect badge|
| App Drawer     | `app_drawer.rs`             | Tabbed sidebar (files, git, terminal, session)             |
| Gesture Arena  | `mgpui/gesture.rs`          | Pan vs scroll gesture disambiguation                      |
| Drawer Host    | `mgpui/drawer_host.rs`      | Slide-from-left overlay with snap animation                |
| Pending Slot   | `pending.rs`                | Generic async→main-thread one-shot channel                 |
| QR Scanner     | `QRScannerActivity.java`    | Camera-based QR code scanning for pairing                  |

### Pixel Handling

```
Android DP -> GPUI Logical Pixels -> Vulkan Physical Pixels
Physical = Logical x Scale Factor (e.g., 3.0 for high-DPI)
Conversion at: window.rs:handle_surface_created()
```

### Surface Lifecycle

Window (logical) persists across surface recreation. Renderer (physical) is created/destroyed with the Android surface lifecycle.

---

## Key Flows

### Pairing (scan once)

```
Host                                    Mobile
1. Load/generate Ed25519 identity
2. Start iroh Endpoint (connects to relay.zedra.dev)
3. Wait for relay connection (endpoint.online())
4. Display QR code:
   base64url(postcard(EndpointAddr))
   (~50 bytes, contains endpoint ID + relay URL + direct addrs)
                                        5. Scan QR code
                                        6. decode_endpoint_addr() -> EndpointAddr
```

### Connection (automatic path selection)

```
Mobile                                  Host
1. Create ephemeral iroh Endpoint (relay: relay.zedra.dev)
2. endpoint.connect(addr, "zedra/rpc/2")
   iroh: direct -> hole-punch -> relay
                                        3. endpoint.accept()
3. Wrap as irpc Client<ZedraProto>
4. ResumeOrCreate RPC
                                        5. Create/resume ServerSession
6. TermAttach bidi stream per terminal
                                        7. Replay missed output via tx, then live I/O
8. Connected
```

### Terminal Session Persistence

> See `docs/TERMINAL_PERSISTENCE.md` for the full design including server-side
> `vt100` screen capture, fresh client terminal discovery, and credential persistence.

**Current flow (within same app session):**

```
Client connects -> ResumeOrCreate -> new session
    TermCreate -> PTY spawned on host
    TermAttach bidi stream: raw PTY output via tx, input via rx

Client disconnects (network drop, app backgrounded)
    Server: TermAttach streams dropped
    PTY keeps running on host
    Output buffered in per-terminal backlog (seq + raw bytes)
    Client: connection/stream ends
    Client: spawn_reconnect() with exponential backoff

Client reconnects -> ResumeOrCreate with stored credentials
    TermAttach { id, last_seq } for each terminal
    Host replays entries with seq > last_seq through tx
    Then switches to live PTY output
    terminal/list verifies server-side terminals still alive
    UI views resume seamlessly (same Arc<OutputBuffer> references)
```

**Known gaps:** Fresh client can't discover existing terminals. No screen state
restoration after long disconnect (blank/garbled terminal). No on-disk credential
storage for cross-restart resume.

---

## Security Model

| Layer                | Mechanism                                                         |
| -------------------- | ----------------------------------------------------------------- |
| Transport encryption | iroh uses QUIC with TLS 1.3 (Ed25519 keys)                        |
| Identity             | Ed25519 keypair = device identity = iroh Endpoint key             |
| Pairing              | QR code establishes host public key out-of-band                   |
| Session auth         | Auth token from initial session creation, required for resume     |
| Relay                | iroh relay forwards encrypted QUIC packets (cannot read contents) |
| Session expiry       | 5-minute grace period, then cleanup                               |

iroh encrypts all traffic end-to-end. The relay server and CF Worker coordination server only see ciphertext.

---

## Performance

| Metric          | Value                         |
| --------------- | ----------------------------- |
| Platform init   | ~51ms                         |
| Frame time      | <5ms CPU, <4ms GPU            |
| Frame rate      | 60 FPS (Choreographer-driven) |
| Memory          | ~40-50 MB                     |
| iroh connection | <100ms LAN, <500ms relay      |
