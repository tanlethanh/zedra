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
First pairing:   Register → Connect(None) → Challenge → AuthProve → Ok(SyncSessionResult) → RPC
Token resume:    Connect(session_token) → Ok(SyncSessionResult) → RPC
PKI reconnect:   Connect(None) → Challenge → AuthProve → Ok(SyncSessionResult) → RPC
```

- **Register**: HMAC-SHA256(handshake_key, pubkey||timestamp) proves QR possession
- **AuthProve**: client signs challenge nonce with Ed25519 key
- **Connect**: universal connection initiator; optionally includes `session_token` for fast resume
- **Authenticate**: deprecated/reserved append-only enum variant; not used by current clients

### RPC Methods

| Category   | Methods |
|------------|---------|
| Auth/bootstrap | `Register`, `Connect`, `AuthProve`, `SyncSession` |
| Health     | `Ping` |
| Session    | `GetSessionInfo`, `ListSessions`, `SubscribeHostInfo` |
| Filesystem | `FsList`, `FsRead`, `FsWrite`, `FsStat`, `FsDocsTree`, `FsWatch`, `FsUnwatch` |
| Terminal   | `TermCreate`, `TermAttach` (bidi stream), `TermResize`, `TermClose`, `TermList`, `TermReorder` |
| Git        | `GitStatus`, `GitDiff`, `GitLog`, `GitCommit`, `GitStage`, `GitUnstage`, `GitBranches`, `GitCheckout` |
| AI         | `AiPrompt` |
| LSP        | `LspDiagnostics`, `LspHover` |
| Events     | `Subscribe` (server-streaming: `HostEvent`) |
| Reserved   | `Authenticate` (deprecated auth challenge request), `SwitchSession` (does not switch the active dispatch session) |

`SwitchSession` remains in the append-only protocol surface, but it is not a
supported active workspace-switching mechanism. The current host handler returns
an explicit unsupported error because the authenticated dispatch worker remains
bound to the original session.

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
3. `Register` only on first pairing
4. `Connect(session_token)` fast path, or `Connect(None) → Challenge → AuthProve` PKI fallback
5. Read the piggybacked `SyncSessionResult` from `ConnectResult::Ok` or `AuthProveResult::Ok`
6. `TermAttach` bidi stream per terminal (replay missed output via `last_seq`)
7. Spawn path watcher (tracks direct vs relay, RTT)

`SyncSession` remains available as a mid-session state refresh, but current
connect bootstrap does not require a separate `SyncSession` round trip.

### Auto-Reconnect

On connection drop: exponential backoff (1s, 2s, 4s, max 3 attempts). Reuses stored credentials. Terminal output buffers survive reconnect.

### Session → UI Bridge

See repo conventions in `AGENTS.md` and `docs/CONVENTIONS.md`. Summary:

```
Session (Tokio) → ConnectEvent via mpsc → cx.spawn loop → SessionState Entity → WorkspaceState Entity → Views
```

On the first successful sync, `Workspace` keeps the connecting UI in the Sync
phase until drawer bootstrap data is fetched: the file explorer root listing
and git status are refreshed before the initial terminal is opened or created.
On reconnect, the same drawer refresh is triggered in the background so terminal
reattach and user interaction are not blocked by file/git refresh latency.

## Security

| Layer | Mechanism |
|-------|-----------|
| Transport | QUIC/TLS 1.3 (iroh, Ed25519 keys) |
| Identity | Ed25519 keypair per device |
| Pairing | QR out-of-band key exchange + HMAC registration |
| Session auth | PKI challenge-response + session tokens |
| Relay | Forwards encrypted QUIC packets only |
