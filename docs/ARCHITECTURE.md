# Architecture

## System Overview

```
Mobile (zedra)                               Desktop (zedra-host)
+-------------------+                       +-------------------+
| GPUI + Metal/wgpu |                       | RPC Daemon        |
| Workspace views   |  iroh (QUIC/TLS 1.3) | SessionRegistry   |
| Terminal, Editor   | <====================> | PTY, Git, FS      |
| QR Scanner        |  NAT traversal/relay  | AI Prompt relay   |
+-------------------+                       +-------------------+
```

## Crates

| Crate | Role |
|-------|------|
| `zedra-rpc` | Protocol types, QR pairing codec. No deps on other zedra crates |
| `zedra-telemetry` | Typed Event enum + TelemetryBackend trait. Pure, no platform deps |
| `zedra-terminal` | Remote terminal view (alacritty VTE + GPUI rendering). No zedra deps — channels attached by app |
| `zedra-session` | Client session (iroh connection, RPC, auto-reconnect). Deps: `zedra-rpc`, `zedra-telemetry` |
| `zedra` | Mobile editor app (iOS + Android cdylib). Deps: `zedra-session`, `zedra-terminal`, `zedra-rpc`, `zedra-telemetry` |
| `zedra-host` | Desktop daemon + CLI. Deps: `zedra-rpc`, `zedra-telemetry` |

```
zedra-rpc          zedra-telemetry        zedra-terminal
(protocol)         (telemetry)            (terminal view, standalone)
    ↑  ↑               ↑  ↑                      ↑
    │  │               │  │                       │
    │  └───────┐   ┌───┘  │                       │
    │          │   │      │                       │
zedra-session  zedra-host │                       │
(client)       (daemon)   │                       │
    ↑                     │                       │
    └─────────────────────┴───────────────────────┘
                    zedra (mobile app)
```

## Transport Layer

All connectivity via iroh (QUIC/TLS 1.3). Path selection (LAN, hole-punch, relay) is automatic.

- **ALPN**: `zedra/rpc/2`
- **Host identity**: persistent Ed25519 keypair at `~/.config/zedra/identity.key`, used as iroh Endpoint secret key
- **Client identity**: persistent Ed25519 keypair in app data directory, used for PKI auth
- **Relay**: iroh-relay servers for NAT traversal fallback

### QR Pairing (`zedra-rpc/src/pairing.rs`)

QR encodes: `zedra://zedra<BASE32LOWER(postcard(ZedraPairingTicket))>`

`ZedraPairingTicket` contains: endpoint ID, relay URL, direct addrs, handshake key, session ID. Metadata (hostname, etc.) discovered post-connection via `SyncSession` RPC.

## RPC Protocol (`zedra-rpc/src/proto.rs`)

Type-safe RPC via irpc with postcard binary serialization over QUIC.

### Auth Flow

```
First pairing:   Register → Connect → Challenge → AuthProve → SyncSession → RPC
Token resume:    Connect(session_token) → SyncSession → RPC
PKI reconnect:   Connect → Challenge → AuthProve → SyncSession → RPC
```

- **Register**: HMAC-SHA256(handshake_key, pubkey||timestamp) proves QR possession
- **AuthProve**: client signs challenge nonce with Ed25519 key
- **Connect**: optionally includes session_token for fast resume

### RPC Methods

| Category   | Methods |
|------------|---------|
| Auth       | `Register`, `Authenticate`, `AuthProve`, `Connect`, `SyncSession` |
| Session    | `SessionInfo`, `SessionList`, `SessionSwitch`, `Ping` |
| Filesystem | `FsList`, `FsRead`, `FsWrite`, `FsStat`, `FsRemove`, `FsMkdir`, `FsWatch` |
| Terminal   | `TermCreate`, `TermAttach` (bidi stream), `TermResize`, `TermClose`, `TermList` |
| Git        | `GitStatus`, `GitDiff`, `GitLog`, `GitCommit`, `GitStage`, `GitUnstage`, `GitBranches`, `GitCheckout` |
| AI         | `AiPrompt` |
| LSP        | `LspDiagnostics`, `LspHover` |
| Events     | `Subscribe` (server-streaming: `HostEvent`) |

## Host Daemon (`zedra-host`)

### Startup

1. Load/generate host identity
2. Bind iroh Endpoint
3. Print QR code
4. Run accept loop (spawns handler per connection)

### Session Registry

Sessions persist independently of connections. PTYs keep running during disconnect. Terminals buffer output for replay on reconnect.

## Session Client (`zedra-session`)

### Connection Flow

1. Create iroh Endpoint with persistent client identity
2. `endpoint.connect(addr, ZEDRA_ALPN)`
3. PKI auth (Register or AuthProve)
4. `SyncSession` — get workspace info
5. `TermAttach` bidi stream per terminal (replay missed output via `last_seq`)
6. Spawn path watcher (tracks direct vs relay, RTT)

### Auto-Reconnect

On connection drop: exponential backoff (1s→30s, max 10 attempts). Reuses stored credentials. Terminal output buffers survive reconnect.

### Session → UI Bridge

See GPUI Conventions in `CLAUDE.md`. Summary:

```
Session (Tokio) → ConnectEvent via mpsc → cx.spawn loop → SessionState Entity → WorkspaceState Entity → Views
```

## Security

| Layer | Mechanism |
|-------|-----------|
| Transport | QUIC/TLS 1.3 (iroh, Ed25519 keys) |
| Identity | Ed25519 keypair per device |
| Pairing | QR out-of-band key exchange + HMAC registration |
| Session auth | PKI challenge-response + session tokens |
| Relay | Forwards encrypted QUIC packets only |
