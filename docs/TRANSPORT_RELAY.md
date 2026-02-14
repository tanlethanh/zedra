# Transport Discovery + Relay Server

## Overview

Zedra uses a **transport-agnostic session layer** where the RPC session persists independently of how bytes move between the mobile client and desktop host. A discovery chain automatically finds the best transport, and the system can hot-switch between transports without the session knowing.

This replaces the original single-transport architecture (direct TCP on the same LAN) with a multi-transport system that works across networks.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  RemoteSession (persistent, transport-unaware)           │
│  - session_id (UUID)                                     │
│  - RPC call/notify via message channels                  │
│  - Terminal state, pending requests                      │
├──────────────────────────────────────────────────────────┤
│  TransportManager (discovery, selection, switching)      │
│  - Ordered discovery chain                               │
│  - Health monitoring (stale connection detection)        │
│  - Hot-switch: swap transport, replay buffered messages  │
│  - Bridges active transport ↔ session message channels   │
├──────────────────────────────────────────────────────────┤
│  Transport (trait)                                       │
│  ┌──────────┐ ┌───────────┐ ┌─────────────────────────┐ │
│  │ LAN TCP  │ │ Tailscale │ │ Relay (CF Worker + KV)  │ │
│  │ (direct) │ │ (100.x)   │ │ (HTTP polling)          │ │
│  └──────────┘ └───────────┘ └─────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

## Components

### Crate Map

| Crate | Path | Purpose |
|-------|------|---------|
| `zedra-rpc` | `crates/zedra-rpc/` | Transport trait, TcpTransport, RPC client/server |
| `zedra-relay` | `crates/zedra-relay/` | HTTP relay client + RelayTransport impl |
| `zedra-transport` | `crates/zedra-transport/` | TransportManager, discovery chain, providers |
| `zedra-session` | `crates/zedra-session/` | RemoteSession (client-side, transport-agnostic) |
| `zedra-host` | `crates/zedra-host/` | SessionRegistry, relay bridge, RPC daemon |
| `relay-worker` | `relay-worker/` | Cloudflare Worker relay server (TypeScript) |

### Dependency Graph

```
zedra-rpc (Transport trait, RpcClient, TcpTransport)
    ↑
zedra-relay (RelayTransport, RelayClient)
    ↑
zedra-transport (TransportManager, discovery, LAN/Tailscale/Relay providers)
    ↑
zedra-session (RemoteSession, uses TransportManager)
    ↑
zedra (Android app, UI)

zedra-host (SessionRegistry, relay bridge, uses Transport trait)
    ↑
zedra-rpc
zedra-relay
```

---

## Transport Trait

Defined in `crates/zedra-rpc/src/transport.rs`:

```rust
#[async_trait]
pub trait Transport: Send + 'static {
    async fn send(&mut self, payload: &[u8]) -> Result<()>;
    async fn recv(&mut self) -> Result<Vec<u8>>;
    fn name(&self) -> &str;
}
```

Each `send`/`recv` handles one complete framed message. Implementations:

| Impl | Crate | Framing | Description |
|------|-------|---------|-------------|
| `TcpTransport` | `zedra-rpc` | 4-byte big-endian length prefix | Wraps `TcpStream::into_split()` |
| `RelayTransport` | `zedra-relay` | Base64 over HTTP | Adaptive polling (50ms–1s) |

### RpcClient Channel Mode

`RpcClient::spawn_from_channels(incoming_rx, outgoing_tx)` creates an RPC client backed by mpsc channels instead of raw streams. The TransportManager bridges between the active transport and these channels, enabling transport swaps without recreating the RPC client.

---

## Discovery Chain

Defined in `crates/zedra-transport/src/discovery.rs`.

**Priority order** (lower = preferred):

| Priority | Provider | Latency | When Used |
|----------|----------|---------|-----------|
| 0 | LAN TCP | ~1ms | Same local network |
| 1 | Tailscale | 5–20ms | WireGuard tunnel (100.x.x.x) |
| 2 | Relay | 50–250ms | Always works (HTTP via Cloudflare) |

**Strategy:**

1. Spawn all providers concurrently via `JoinSet`.
2. **Fast phase (500ms):** If LAN or Tailscale connects, use it immediately. Priority 0 (LAN) short-circuits without waiting.
3. **Slow phase (10s):** If no fast transport connected, wait for any provider. Relay always succeeds if the relay server is up.
4. Pick the transport with the lowest priority value among all successes.

```
PeerInfo (from QR code)
    ↓
TransportManager::connect()
    ↓
discover([LanProvider, TailscaleProvider, RelayProvider])
    ↓ (concurrent, priority-weighted)
Box<dyn Transport>
```

---

## TransportManager

Defined in `crates/zedra-transport/src/manager.rs`.

### Lifecycle

```rust
// 1. Create manager + session-facing channels
let (mut mgr, recv_rx, send_tx) = TransportManager::new(peer_info);

// 2. Pass channels to RpcClient (session never sees the transport)
let (rpc_client, notif_rx) = RpcClient::spawn_from_channels(recv_rx, send_tx);

// 3. Run discovery
mgr.connect().await?;

// 4. Start background bridge loop (handles health, switching, buffering)
tokio::spawn(mgr.run());
```

### Bridge Loop (`run()`)

The main loop uses `tokio::select!` across four concurrent operations:

1. **Transport → Session:** `transport.recv()` → `session_recv_tx.send()`
2. **Session → Transport:** `session_send_rx.recv()` → `transport.send()`
3. **Health check (30s interval):** If no data received for 45s, attempt reconnection. After 3 consecutive failures, give up.
4. **Upgrade probe (30s interval):** If currently on relay, probe LAN addresses (500ms TCP connect). If reachable, switch to LAN.

### Transport States

```rust
pub enum TransportState {
    Discovering,                        // Running discovery chain
    Connected { transport_name: String }, // Active transport
    Switching { from: String, to: String }, // Hot-switching
    Disconnected,                       // All transports failed
}
```

### Message Buffering

During a transport switch:
1. Failed outgoing messages are pushed to `pending_outgoing: Vec<Vec<u8>>`.
2. Additional messages from the session channel are drained into the buffer.
3. After reconnection, all buffered messages are replayed on the new transport.

### Hot Transport Switching

The manager can **upgrade** from relay to LAN automatically:

```
relay (connected) → LAN probe every 30s
                     ↓ (probe succeeds)
                  Switching { from: "relay", to: "lan-tcp" }
                     ↓ (full connect)
                  Connected { transport_name: "lan-tcp" }
                     ↓ (replay buffered messages)
                  Session sees no interruption
```

This handles the case where a user connects via relay while away from home, then returns to the same LAN as the host.

---

## Cloudflare Worker Relay

Located in `relay-worker/`. Stateless Workers + KV storage.

### Setup

```bash
cd relay-worker
npm install
# Edit wrangler.toml: replace KV namespace ID
npx wrangler kv:namespace create "RELAY_KV"
npx wrangler dev          # Local development
npx wrangler deploy       # Production deployment
```

### API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/rooms` | None | Host creates a room (returns code + secret) |
| `POST` | `/rooms/:code/join` | Bearer secret | Mobile joins (rate-limited: 5/min/IP) |
| `POST` | `/rooms/:code/send` | Bearer secret | Send batched messages (max 10, max 1MB each) |
| `GET` | `/rooms/:code/recv?role=X&after=N` | Bearer secret | Poll for messages (max 50) |
| `POST` | `/rooms/:code/signal` | Bearer secret | Exchange connection info (IPs, capabilities) |
| `GET` | `/rooms/:code/signal?role=X` | Bearer secret | Get peer's signaling data |
| `POST` | `/rooms/:code/heartbeat` | Bearer secret | Keep room alive |
| `DELETE` | `/rooms/:code` | Bearer secret | Tear down room |

### KV Schema

| Key Pattern | TTL | Content |
|-------------|-----|---------|
| `room:{code}` | 300s (unjoined) / 3600s (joined) | Room metadata (code, secret, state) |
| `msg:{code}:{role}:{seq}` | 60s | Base64-encoded RPC frame |
| `seq:{code}:{role}` | 3600s | Sequence counter (atomic via KV) |
| `signal:{code}:{role}` | 300s | Signaling data (JSON) |
| `rl:{ip}` | 60s | Rate limit counter |

### Security

- Room secret is a 64-character hex string embedded in the QR code, not typed by the user.
- All operations (except room creation) require the secret via `Authorization: Bearer` header.
- Unjoined rooms expire in 5 minutes. Single-peer: once mobile joins, no additional peers.
- CORS headers on all responses.
- Worker does not inspect message contents (opaque base64 blobs).

---

## Relay Client (`zedra-relay`)

Located in `crates/zedra-relay/`.

### RelayClient

Typed HTTP client for all relay API endpoints:

```rust
let room = RelayClient::create_room("https://relay.zedra.dev").await?;
let client = RelayClient::new(url, room.code, room.secret, "mobile");
client.join_room().await?;
client.send_messages(&[base64_msg]).await?;
let resp = client.recv_messages(last_seq).await?;
```

- Uses `reqwest` with `rustls-tls-webpki-roots` (no OpenSSL, works on Android).
- 10-second HTTP timeout per request.

### RelayTransport

Implements the `Transport` trait over HTTP polling:

- **send():** Base64-encode payload, POST to `/rooms/:code/send`.
- **recv():** Poll `/rooms/:code/recv` with **adaptive intervals**:
  - Active (message received < 1s ago): **50ms**
  - Idle (< 30s since last message): **250ms**
  - Background (> 30s): **1s**

---

## Session Registry (Server-Side)

Defined in `crates/zedra-host/src/session_registry.rs`.

### Design

Sessions persist independently of transport connections:

```
Client connects     → session/resume_or_create → ServerSession created
  ↓
Client disconnects  → session stays alive (5-min grace period)
  ↓                   PTY processes keep running
  ↓                   Terminal output buffered in notification backlog
Client reconnects   → session/resume_or_create with session_id
  ↓                   → Reattach to same PTY, replay missed notifications
```

### ServerSession

```rust
pub struct ServerSession {
    pub id: String,                    // UUID
    pub auth_token: String,            // From QR pairing
    pub terminals: Mutex<HashMap<String, TermSession>>,  // Owned terminals
    pub notification_backlog: Mutex<VecDeque<(u64, Vec<u8>)>>,  // Capped at 1000
    pub next_notif_seq: Mutex<u64>,    // Monotonic sequence counter
}
```

### Session Cleanup

A background task runs every 60 seconds, removing sessions idle for more than 5 minutes (`GRACE_PERIOD`). Terminal PTY processes are cleaned up with their sessions.

### New RPC Methods

| Method | Description |
|--------|-------------|
| `session/resume_or_create` | Client sends optional `session_id` + `auth_token`. Returns session_id + notification backlog. |
| `session/heartbeat` | Keeps session alive, returns OK. |

---

## QR Pairing Payload (v2)

Extended from v1 with multi-transport fields:

```json
{
  "v": 2,
  "host": "192.168.1.100",
  "port": 2123,
  "token": "abc123...",
  "fingerprint": "SHA256:...",
  "name": "my-laptop",
  "host_addrs": ["192.168.1.100", "192.168.1.101"],
  "tailscale_addr": "100.64.0.1",
  "relay_url": "https://relay.zedra.dev",
  "relay_room": "a1b2c3",
  "relay_secret": "64-char-hex..."
}
```

- v1 payloads (just `host`/`port`) still parse correctly — v2 fields have `#[serde(default)]`.
- `PairingPayload::to_peer_info()` converts to `PeerInfo` for the TransportManager.
- URI format: `zedra://pair?d=<base64url-encoded-json>`

---

## Host Relay Mode

Start the host in relay mode:

```bash
zedra-host relay --workdir /path/to/project --relay-url https://relay.zedra.dev
```

Flow:
1. Creates a relay room via `RelayClient::create_room()`.
2. Displays a QR code with room code, secret, and LAN addresses.
3. Waits for the mobile client to join (polls signal endpoint).
4. Creates `RelayTransport` as "host" role.
5. Bridges to `handle_transport_connection()` for RPC dispatch.
6. Spawns a heartbeat task (30s interval) to keep the relay room alive.

---

## Client Integration

### RemoteSession

Two connection methods:

```rust
// Direct TCP (backward compatible, v1 pairing)
let session = RemoteSession::connect("192.168.1.100", 2123).await?;

// Multi-transport via TransportManager (v2 pairing)
let peer_info = pairing_payload.to_peer_info();
let session = RemoteSession::connect_with_peer_info(peer_info).await?;
```

Both return `Arc<RemoteSession>` with the same API. The session is unaware of which transport is active.

### Transport State in UI

The `RemoteSession` exposes `transport_state()` which returns the current `TransportState`. The Android app can display this as a status indicator (e.g., "Connected via lan-tcp" or "Connected via relay").

---

## Connection Flow (End-to-End)

### Scenario: User scans QR at home (same LAN)

```
Host:   zedra-host start --port 2123
Mobile: Scan QR → PairingPayload v2 → PeerInfo
        TransportManager::connect()
          → LanProvider connects in <100ms (priority 0)
          → RelayProvider still trying (priority 2)
          → LAN wins, relay aborted
        RpcClient::spawn_from_channels()
        session/resume_or_create → new session
        terminal/create → PTY spawned
        Connected via lan-tcp
```

### Scenario: User is away from home

```
Host:   zedra-host relay --workdir .
        → Creates relay room, displays QR
Mobile: Scan QR → PeerInfo (has relay info, LAN IPs unreachable)
        TransportManager::connect()
          → LanProvider times out (2s per addr)
          → RelayProvider connects via HTTP
          → Relay wins
        Connected via relay (~100ms per message)

        [User returns home]
        TransportManager upgrade probe (every 30s):
          → LanProvider.probe() succeeds (500ms TCP connect)
          → Full LAN connect succeeds
          → Hot-switch: relay → lan-tcp
          → Session continues uninterrupted
```

### Scenario: Transport dies mid-session

```
Connected via lan-tcp
WiFi drops → transport.recv() returns Err
TransportManager:
  1. Buffers pending outgoing messages
  2. try_reconnect() → discovery chain
     → LAN fails (no WiFi)
     → Relay connects
  3. Replays buffered messages on relay
  4. State: Connected { "relay" }
Session: never sees the switch, PTY still running on host
```

---

## Latency Characteristics

| Transport | Latency | Bandwidth | Reliability | When to Use |
|-----------|---------|-----------|-------------|-------------|
| LAN TCP | ~1ms | Full LAN speed | High | Same network |
| Tailscale | 5–20ms | WireGuard tunnel | High | Across NATs, VPN |
| Relay | 50–250ms | HTTP polling overhead | High (CF edge) | Always available |

---

## Security Model

### v1 (Current)

- Room secret (64-char hex) embedded in QR, not typed.
- Rate limiting on join (5 attempts per IP per minute).
- Relay traffic over HTTPS (Cloudflare TLS termination).
- Session resume requires matching auth token from QR pairing.
- Sessions auto-expire after 5-minute grace period.
- **Tradeoff:** No E2E encryption. The relay can read message contents.

### v2 (Future)

- E2E encryption via X25519 key exchange during pairing.
- ChaCha20-Poly1305 for message encryption.
- Relay becomes a fully opaque pipe.

---

## File Reference

### New Crates

```
crates/zedra-relay/
  Cargo.toml
  src/
    lib.rs              -- Re-exports, DEFAULT_RELAY_URL
    client.rs           -- RelayClient (typed HTTP methods for all endpoints)
    transport.rs        -- RelayTransport (Transport trait impl, adaptive polling)
    types.rs            -- CreateRoomResponse, RecvResponse, RelayMessage, etc.

crates/zedra-transport/
  Cargo.toml
  src/
    lib.rs              -- PeerInfo, re-exports
    discovery.rs        -- Concurrent discovery chain with priority selection
    manager.rs          -- TransportManager (bridge, health, upgrade, buffering)
    providers/
      mod.rs            -- TransportProvider trait
      lan.rs            -- LAN TCP provider (2s timeout, probe method)
      tailscale.rs      -- Tailscale TCP provider
      relay.rs          -- Relay provider (wraps zedra-relay)

relay-worker/
  package.json
  wrangler.toml         -- KV namespace binding
  tsconfig.json
  src/
    index.ts            -- Router, CORS, error handling
    rooms.ts            -- Room CRUD (create, join, heartbeat, delete)
    messaging.ts        -- Send/recv with KV message store
    signaling.ts        -- Connection info exchange
    types.ts            -- TypeScript interfaces
    utils.ts            -- Code gen, rate limiting, validation
```

### New Files in Existing Crates

```
crates/zedra-host/src/
  session_registry.rs   -- SessionRegistry, ServerSession, TermSession
  relay_bridge.rs       -- run_relay_mode(), QR display, relay bridging
```

### Modified Files

| File | Changes |
|------|---------|
| `crates/zedra-rpc/src/transport.rs` | Added `Transport` trait, `TcpTransport`, `RpcClient::spawn_from_channels()` |
| `crates/zedra-rpc/src/protocol.rs` | Added `session/resume_or_create`, `session/heartbeat` methods + types |
| `crates/zedra-host/src/rpc_daemon.rs` | Added `handle_transport_connection()` (generic over Transport), session registry integration, cleanup task |
| `crates/zedra-host/src/main.rs` | Added `relay` subcommand |
| `crates/zedra-host/src/qr.rs` | Added `generate_relay_pairing_qr()` with v2 payload and multi-IP support |
| `crates/zedra-host/src/lib.rs` | Added `session_registry`, `relay_bridge` module declarations |
| `crates/zedra-session/src/lib.rs` | Added `connect_with_peer_info()`, `session_id`, `transport_state` fields, extracted notification listener |
| `crates/zedra-ssh/src/pairing.rs` | Extended `PairingPayload` to v2, added `to_peer_info()` |
| `crates/zedra/src/zedra_app.rs` | Transport state indicator in connection UI |
