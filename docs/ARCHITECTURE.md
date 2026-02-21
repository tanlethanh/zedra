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
  zedra-transport/     Identity, iroh transport, CF Worker discovery, QR pairing
  zedra-rpc/           Transport trait, JSON-RPC 2.0 framing, RpcClient/RpcServer
  zedra-host/          Desktop daemon: iroh listener, RPC dispatch, session registry
  zedra-session/       Mobile client: iroh connection, RPC session, terminal buffers, auto-reconnect
  zedra-terminal/      Terminal emulation (alacritty) + GPUI rendering
  zedra-editor/        Code editor with tree-sitter syntax highlighting
  zedra-nav/           Mobile navigation primitives (tabs, stacks, drawer)
  zedra/               Android cdylib: JNI bridge, GPUI app, touch handling

packages/
  relay-worker/        Cloudflare Worker: endpoint discovery + WebSocket relay
```

### Dependency Graph

```
zedra-rpc          (Transport trait, RpcClient, framing)
    ^
zedra-transport    (IrohTransport, identity, pairing)
    ^
zedra-session      (RemoteSession, iroh connection, terminal output buffers)
    ^
zedra-terminal     (TerminalState, TerminalElement, TerminalView)
    ^
zedra              (Android cdylib, GPUI app, JNI bridge)

zedra-host         (iroh listener, RPC daemon, session registry)
    ^
zedra-rpc
zedra-transport
```

---

## Transport Layer (`zedra-transport`)

All connectivity goes through a single iroh Endpoint per device. iroh handles path selection (LAN, hole-punched, relay) internally.

### Identity (`identity/`)

Every device has a persistent Ed25519 keypair that serves as both identity and iroh Endpoint secret key.

| Component               | Purpose                                                                        |
| ----------------------- | ------------------------------------------------------------------------------ |
| `Keypair`               | Ed25519 keypair stored at `~/.config/zedra-host/identity.key` (32-byte secret) |
| `DeviceId`              | Human-readable 56-char base32 identifier derived from public key               |
| `PublicKey`/`SecretKey` | Re-exported from iroh                                                          |

```
DeviceId format: RLKQ4WE-GLLHZT5-7QFG3G2-VFI3HTG-XFQTPNL-BNVHJ6Q-WDHYQFP-XWIQTAH
                 (8 groups of 7 chars, SHA-256 of public key, base32-encoded)
```

### IrohTransport (`iroh_transport.rs`)

Adapter wrapping iroh's QUIC bidirectional streams into the `Transport` trait:

- Length-delimited framing: `[4-byte big-endian length][JSON payload]`
- `into_rpc_channels()` spawns reader/writer tasks, returns mpsc pairs for `RpcClient`

### Pairing (`pairing.rs`)

QR code pairing protocol using `zedra://pair?d=<base64url-json>` URIs:

```json
{
  "v": 1,
  "endpoint_id": "<z-base-32 Ed25519 public key>",
  "name": "my-laptop",
  "relay_url": "https://relay.zedra.dev",
  "addrs": ["192.168.1.100:12345"]
}
```

`PairingPayload.to_endpoint_addr()` converts the payload into an iroh `EndpointAddr` for connecting.

---

## RPC Layer (`zedra-rpc`)

Transport-agnostic JSON-RPC 2.0 protocol with multiplexed request/response.

### Transport Trait (`transport.rs`)

```rust
#[async_trait]
pub trait Transport: Send + 'static {
    async fn send(&mut self, payload: &[u8]) -> Result<()>;
    async fn recv(&mut self) -> Result<Vec<u8>>;
    fn name(&self) -> &str;
}
```

Each `send`/`recv` handles one complete length-delimited message.

### RpcClient

Multiplexed client with pending-request map:

- `call(method, params) -> Response` -- sends request, waits for matching response by ID
- `notify(method, params)` -- fire-and-forget notification
- `spawn(reader, writer)` -- creates client from `AsyncRead`/`AsyncWrite` halves
- `spawn_from_channels(rx, tx)` -- creates client from mpsc channels (used by session layer)

### Protocol (`protocol.rs`)

Standard JSON-RPC 2.0 messages with domain-specific methods:

| Category   | Methods                                                                           |
| ---------- | --------------------------------------------------------------------------------- |
| Filesystem | `fs/list`, `fs/read`, `fs/write`, `fs/stat`, `fs/remove`, `fs/mkdir`              |
| Terminal   | `terminal/create`, `terminal/data`, `terminal/resize`, `terminal/close`, `terminal/list` |
| Git        | `git/status`, `git/diff`, `git/log`, `git/commit`, `git/branches`, `git/checkout` |
| Session    | `session/resume_or_create`, `session/attach`, `session/info`, `session/list`      |
| LSP        | `lsp/hover`                                                                       |
| AI         | `ai/prompt`                                                                       |

Notifications: `terminal/output` (streamed from PTY reader).

---

## Host Daemon (`zedra-host`)

Desktop process that listens for incoming iroh connections and dispatches RPC operations.

### Startup Flow (`main.rs`)

```
1. Load/generate persistent host identity (Ed25519 keypair)
2. Create named session for working directory
3. Bind iroh Endpoint
4. Generate pairing QR code (endpoint ID + relay URL + direct addrs)
5. Spawn session cleanup loop (every 60s, 5-min grace period)
6. Run iroh accept loop (blocks main thread)
```

### Iroh Listener (`iroh_listener.rs`)

- **ALPN**: `zedra/rpc/1`
- **Relay**: `RelayMode::custom([DEFAULT_RELAY_URL])` -- uses `relay.zedra.dev` for NAT traversal
- `create_endpoint()` -- builds iroh Endpoint with host's SecretKey, waits for relay connection (10s timeout)
- `run_accept_loop()` -- accepts connections, spawns handler per connection
- `handle_incoming()` -- accepts bidi stream, wraps in `IrohTransport`, passes to RPC dispatch

### RPC Dispatch (`rpc_daemon.rs`)

`handle_transport_connection(transport, registry, state)` is the transport-agnostic entry point:

1. First message triggers session binding (`session/resume_or_create` or `session/attach`)
2. Main loop reads requests, dispatches to handlers, sends responses
3. Spawns notification forwarder (session backlog -> transport)
4. Spawns PTY reader tasks for terminal output streaming
5. On disconnect: `clear_notif_senders()` so PTY readers don't send on dead channel
6. `terminal/list` handler returns active terminal IDs for reconnect reconciliation

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
zedra-host start [--workdir .] [--relay-url URL] [--no-relay]
zedra-host devices
zedra-host revoke <device_id>
zedra-host session create --name NAME --workdir PATH
zedra-host session list
zedra-host session qr
zedra-host session remove NAME
```

---

## Session Client (`zedra-session`)

Mobile client library for connecting to zedra-host via iroh and issuing RPC calls.

### Connection (`connect_with_iroh`)

```
1. Store PairingPayload in PAIRING_PAYLOAD (for reconnect)
2. Reset USER_DISCONNECT flag
3. Create ephemeral iroh Endpoint (client side, relay: relay.zedra.dev)
4. Parse host's EndpointAddr from PairingPayload (includes relay URL)
5. endpoint.connect(addr, b"zedra/rpc/1")
   iroh internally: tries direct addrs -> hole-punches -> relay fallback (relay.zedra.dev)
6. Open bidi stream, wrap in IrohTransport
7. Convert to RpcClient via into_rpc_channels()
8. Use persistent terminal output buffers (PERSISTENT_TERMINAL_OUTPUTS)
   so UI views survive reconnect via shared Arc references
9. session/resume_or_create with SESSION_CREDENTIALS + LAST_NOTIF_SEQ
10. Process notification backlog (decode base64, route terminal output to buffers)
11. Spawn notification listener (tracks seq, triggers reconnect on exit)
12. Spawn ping loop (breaks after 2 consecutive failures)
```

### Global State (OnceLock singletons)

- `ACTIVE_SESSION` -- current `RemoteSession`
- `SESSION_RUNTIME` -- dedicated tokio runtime (2 worker threads)
- `MAIN_THREAD_CALLBACKS` -- deferred work queue for GPUI main thread
- `TERMINAL_DATA_PENDING` -- atomic flag, signaled by notification listener, polled by GPUI frame loop

Reconnect state (persists across `RemoteSession` rebuilds):

- `RECONNECT_ATTEMPT` (AtomicU32) -- current attempt (0 = not reconnecting)
- `USER_DISCONNECT` (AtomicBool) -- set on user disconnect, prevents auto-reconnect
- `LAST_NOTIF_SEQ` (AtomicU64) -- highest notification seq processed
- `PAIRING_PAYLOAD` -- stored for reconnect attempts
- `SESSION_CREDENTIALS` -- (session_id, auth_token) for session resumption
- `PERSISTENT_TERMINAL_OUTPUTS` -- shared output buffers that survive reconnect
- `PERSISTENT_TERMINAL_IDS` -- terminal ID list that survives reconnect
- `PERSISTENT_ACTIVE_TERMINAL` -- active terminal ID that survives reconnect

### Terminal Output Flow

```
Host PTY reader -> terminal/output notification -> IrohTransport
    -> notification listener -> per-terminal OutputBuffer (persistent Arc)
    -> LAST_NOTIF_SEQ incremented
    -> TERMINAL_DATA_PENDING atomic flag
    -> GPUI frame loop polls flag -> drains buffer
    -> TerminalState.advance_bytes() -> re-render
```

### Auto-Reconnect Flow

```
Connection drops -> notification listener rx.recv() returns None
    -> spawn_reconnect()
       Guard: USER_DISCONNECT? abort
       Guard: CAS RECONNECT_ATTEMPT 0->1 (prevent double-reconnect)
       Loop (max 20 attempts, ~5 min matching server grace period):
         1. Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s cap
         2. Signal TERMINAL_DATA_PENDING -> UI shows "Reconnecting... (N)"
         3. connect_with_iroh(stored PairingPayload)
            -> reuses persistent output buffers (UI views keep working)
            -> establish_rpc_session with stored credentials
            -> server resumes same session, replays backlog
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
                        -> GPUI (single-threaded) -> Blade -> Vulkan
```

### Key Components

| Component     | File                       | Purpose                                                 |
| ------------- | -------------------------- | ------------------------------------------------------- |
| JNI Bridge    | `android_jni.rs`           | Java-Rust interface, thread-safe command queue          |
| Android App   | `android_app.rs`           | Main-thread GPUI state, touch/scroll/key handling       |
| Command Queue | `android_command_queue.rs` | Crossbeam mpsc channel for JNI decoupling               |
| Zedra App     | `zedra_app.rs`             | Root view: DrawerHost + TabNavigator + connection state |
| QR Scanner    | `QRScannerActivity.java`   | Camera-based QR code scanning for pairing               |

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
1. Generate Ed25519 identity
2. Start iroh Endpoint (connects to relay.zedra.dev)
3. Wait for relay connection (endpoint.online())
4. Display QR code:
   zedra://pair?d=<base64url-json>
                                        5. Scan QR code
                                        6. Parse PairingPayload
                                           (endpoint_id, relay_url, addrs)
```

### Connection (automatic path selection)

```
Mobile                                  Host
1. Create ephemeral iroh Endpoint (relay: relay.zedra.dev)
2. Parse EndpointAddr from payload (includes relay URL)
3. endpoint.connect(addr, "zedra/rpc/1")
   iroh: direct -> hole-punch -> relay
                                        4. endpoint.accept()
                                        5. accept_bi() -> IrohTransport
4. open_bi() -> IrohTransport
5. RpcClient multiplexes requests
6. session/resume_or_create
                                        7. Create/resume ServerSession
                                        8. Send backlog (if resuming)
9. Replay missed terminal output
10. Connected
```

### Terminal Session Persistence

```
Client connects -> session/resume_or_create -> new session
    terminal/create -> PTY spawned on host
    terminal/output notifications stream to client
    LAST_NOTIF_SEQ incremented per notification

Client disconnects (network drop, app backgrounded)
    Server: clear_notif_senders() on disconnect
    PTY keeps running on host
    Output buffered in notification backlog (seq + payload)
    Client: notification listener detects dead channel
    Client: spawn_reconnect() with exponential backoff

Client reconnects -> session/resume_or_create with stored credentials
    Passes LAST_NOTIF_SEQ to get only missed notifications
    Host sends backlog entries (seq, base64 payload)
    Client decodes backlog, routes terminal output to persistent buffers
    terminal/list verifies server-side terminals still alive
    UI views resume seamlessly (same Arc<OutputBuffer> references)
```

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
