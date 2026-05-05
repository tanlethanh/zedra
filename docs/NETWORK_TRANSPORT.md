# Network Transport

This document describes how Zedra connects a mobile client to a host daemon:
the QR pairing format, PKI authentication, address discovery via pkarr, the
connection state machine, and the authorization model.

---

## 1. QR Pairing Ticket

The QR payload is a `ZedraPairingTicket` encoded as a `zedra://` URL:

```
zedra://connect?ticket=<base64url(postcard(ZedraPairingTicket))>
```

```rust
// zedra-rpc/src/pairing.rs

pub struct ZedraPairingTicket {
    /// Host's 32-byte Ed25519 public key.
    /// Used as pkarr lookup key and to verify AuthChallenge signatures.
    pub endpoint_id: iroh::PublicKey,
    /// 16-byte random secret for this pairing slot.
    /// Normal slots are consumed by the first valid Register.
    /// Static QR slots remain reusable while the daemon runs.
    /// Client uses this as the HMAC key to prove QR possession.
    pub handshake_secret: [u8; 16],
    /// The session this QR was generated for.
    /// New client's pubkey is auto-added to this session's ACL on Register.
    pub session_id: String,
}
```

Routing info (relay URL, direct IPs) is **not** embedded — it is resolved
dynamically via pkarr at connect time, so the QR stays valid after IP changes.

### QR code size

Error correction level L (7% recovery).

| Payload | Raw bytes | Encoded chars | QR version |
|---------|-----------|--------------|------------|
| EndpointId (32) + handshake_secret (16) + session_id (~32) | ~80 | ~128 | v4 |
| With embedded EndpointAddr (relay URL + 2 LAN IPs, future) | ~112 | ~180 | v6 |

### Handshake slot lifecycle

- **Created**: one slot per session at `zedra start` time or when `zedra qr`
  is run.
- **Stored**: in `SessionRegistry` memory only. A host crash or restart
  invalidates the QR.
- **Default**: one-time. The slot is consumed atomically on the first valid
  `Register`. Subsequent attempts return `HandshakeConsumed`.
- **Static mode**: `zedra start --static-qr` or `zedra qr --static` creates a
  QR that can be scanned repeatedly until the daemon exits or another QR
  replaces it. Use only for testing or store review.
- **Expiry**: default slots expire 10 minutes after creation. Static slots do
  not expire while the daemon is running.

---

## 2. Connection Lifecycle

ALPN: `zedra/rpc/2`. All messages are postcard-serialized irpc requests.

### First pairing (after QR scan)

```
iroh QUIC + TLS 1.3 established
Client → host: Register { client_pubkey, timestamp, hmac }
  hmac = HMAC-SHA256(handshake_key, client_pubkey || timestamp_le_bytes)
  Proves sender physically scanned the QR (has handshake_key).
  Host verifies: |now - timestamp| ≤ 60s, HMAC valid, slot active.
  On success: one-time slot consumed; static slot remains active;
  client_pubkey added to authorized_clients + session ACL.
Host → client: RegisterResult::Ok

Client → host: Connect { client_pubkey, session_id, session_token: None }
Host: generates 32-byte nonce, signs it with iroh SecretKey
Host → client: ConnectResult::Challenge { nonce, host_signature }
  Client MUST verify host_signature using the stored endpoint_id.

Client: signs nonce with application Ed25519 key
Client → host: AuthProve { nonce, client_signature, session_id }
  Host: verifies client_signature against stored pubkey, attaches session.
Host → client: AuthProveResult::Ok(SyncSessionResult { session_token, ... })

→ RPC calls may proceed
```

### Reconnect (subsequent connections)

Fast path when the client still has the in-memory `session_token` from the last
successful attach:

```
iroh QUIC + TLS 1.3 established

Client → host: Connect { client_pubkey, session_id, session_token: Some(token) }
Host: validates and consumes token, attaches session, issues a fresh token.
Host → client: ConnectResult::Ok(SyncSessionResult { session_token, ... })

→ RPC calls may proceed
```

PKI fallback when no valid session token is available:

```
iroh QUIC + TLS 1.3 established

Client → host: Connect { client_pubkey, session_id, session_token: None }
Host → client: ConnectResult::Challenge { nonce, host_signature }
  Client verifies host_signature.

Client → host: AuthProve { nonce, client_signature, session_id }
Host → client: AuthProveResult::Ok(SyncSessionResult { session_token, ... })

→ RPC calls may proceed
```

### Register result codes

| Code | Meaning |
|------|---------|
| `Ok` | Registered. Proceed to `Connect`. |
| `HandshakeConsumed` | One-time slot already used. Ask host to run `zedra qr --workdir <workdir>` to get a new QR. |
| `InvalidHandshake` | HMAC failed. Wrong key or tampered packet. |
| `StaleTimestamp` | Clock skew > 60s or replay attempt. |
| `SlotNotFound` | No slot for this session. One-time QR expired (> 10 min), QR was replaced, or wrong session. |

### AuthProve result codes

| Code | Meaning |
|------|---------|
| `Ok` | Authenticated and attached; carries `SyncSessionResult` bootstrap data and a fresh `session_token`. |
| `InvalidSignature` | Nonce mismatch or bad client signature. |
| `NotInSessionAcl` | Client not authorized for this session. |
| `SessionOccupied` | Another client is already attached. |
| `SessionNotFound` | Session gone (daemon restarted). Client falls back to any ACL'd session. |

---

## 3. Client Identity

The client uses a **dedicated Ed25519 application keypair**, separate from the
iroh transport keypair. This decouples auth identity from transport so iroh key
rotation or protocol upgrades never invalidate pairings.

```rust
// zedra-session/src/signer.rs

pub trait ClientSigner: Send + Sync {
    fn pubkey(&self) -> [u8; 32];
    fn sign(&self, data: &[u8]) -> [u8; 64];
}

/// File-backed implementation. Key stored at:
///   Desktop/CLI: <workspace_config_dir>/cli-client.key
///   Android/iOS: <app_data_dir>/zedra/client.key
/// Permissions: 0o600. Future: swap for hardware-backed impl.
pub struct FileClientSigner { signing_key: ed25519_dalek::SigningKey }
```

The keypair is generated once on first launch and reused across app restarts,
network changes, and reconnects to any host.

---

## 4. Two-Level Authorization Model

Analogous to SSH `authorized_keys` but with per-session granularity.

### Level 1 — Global authorized list

```
authorized_clients: HashSet<[u8; 32]>
```

A pubkey here can connect to the host at all. Populated by `Register`.
Persisted in `sessions.json`. Equivalent to `~/.ssh/authorized_keys`.

### Level 2 — Per-session ACL

```
ServerSession {
    id:            String,
    workdir:       PathBuf,
    acl:           HashSet<[u8; 32]>,     // who may attach
    active_client: Option<[u8; 32]>,      // currently attached
}
```

A pubkey must appear in `session.acl` to attach. Being globally authorized is
necessary but not sufficient.

The ACL is populated automatically on `Register` — the `session_id` from the
QR ticket determines which session gains the new client. A client may be in
multiple sessions' ACLs (e.g. `~/work` and `~/personal`).

### Exclusive session ownership

Only one client may be actively attached to a session at a time, preventing
split-brain on the PTY. A second client gets `SessionOccupied`.

To transfer ownership:
- Active client disconnects voluntarily, or
- Host operator runs `zedra detach --session-id <id>` *(CLI not yet implemented;
  `SessionRegistry::force_detach()` exists)*

On forced detach, the host sends `SessionCloseReason::SessionTakenOver` as a
QUIC APPLICATION_CLOSE so the client can show a meaningful UI message.

---

## 5. Address Discovery (pkarr)

The host publishes its addresses to `dns.iroh.link` via iroh's `PkarrPublisher`.
The client resolves at connect time via `PkarrResolver`. This allows the QR to
contain only the `endpoint_id` (pubkey) — routing info is always fresh.

**Host** (`iroh_listener.rs`):
```rust
iroh::Endpoint::builder()
    .relay_mode(iroh::RelayMode::Custom(relay_map_from_url(relay_url)?))
    .address_lookup(PkarrPublisher::n0_dns())
    .bind()
    .await?
```

**Client** (`zedra-session/src/lib.rs`):
```rust
// id-only addr: pkarr resolver supplies routing info at connect time
let addr = iroh::EndpointAddr::from(ticket.endpoint_id);
endpoint.connect(addr, ZEDRA_ALPN).await?
```

When a relay URL is set, `PkarrPublisher` publishes the relay URL only (no
direct IPs). When there is no relay, it publishes direct IPs. This means
the host's IP is visible to `dns.iroh.link` in relay-free setups — an accepted
trade-off for simplicity. Future: self-hosted pkarr with AES-GCM-encrypted
records (see Section 8).

---

## 6. Connection States & Reconnect

```
┌─────────────────────────────────────────────────────────────────┐
│ Connected                                                       │
│  • Ping every 2s (foreground only), RTT tracked for badge      │
│  • 5 consecutive missed pongs → Reconnecting                   │
│  • QUIC error / connection closed → Reconnecting               │
│  • HostShutdown received → brief "Host disconnected" message   │
│    → auto-transition to Reconnecting after 2s                  │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ Reconnecting (attempt N / 3)                                    │
│  • Fresh pkarr resolve on each attempt (no cached addr)        │
│  • On connect: Connect(session_token) fast path, or             │
│    Connect → Challenge → AuthProve fallback                     │
│  • Backoff: 1s, 2s, 4s with per-second countdown updates       │
│  • After 3 failed attempts → Host Unreachable                  │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ Host Unreachable                                                │
│  • "Retry" button → reset attempt counter → Reconnecting       │
│  • Long press workspace → "Remove" → delete stored pairing     │
└─────────────────────────────────────────────────────────────────┘
```

### Ping / Pong

```rust
pub struct PingReq   { pub timestamp_ms: u64 }  // sent by client every 2s
pub struct PongResult { pub timestamp_ms: u64 } // host echoes timestamp
```

RTT = `now_ms - pong.timestamp_ms`. Displayed in the transport badge and
session panel. Path watcher also provides RTT from iroh path stats (updated
every 2s or on path change, whichever comes first).

### Background / foreground

The OS suspends network activity when the app is backgrounded. Missed pings
must not count toward the 5-miss threshold:
- Ping counter incremented only while app is in the foreground.
- On return to foreground: if QUIC connection is alive, resume normally;
  if the OS has surfaced a connection error, enter Reconnecting immediately.
- Foreground → background: reset miss counter.

### Host shutdown signal

On clean exit (`zedra stop`, SIGTERM), the host closes the QUIC connection
with `SessionCloseReason::HostShutdown`. Client shows "Host disconnected"
and auto-transitions to Reconnecting after 2s (the daemon may restart).

---

## 7. Security Properties

| Property | Current |
|----------|---------|
| Who can connect? | Only device that scanned QR (HMAC proves possession) |
| Auth secret quality | 256-bit CSPRNG (one-use handshake key) |
| Auth secret origin | Host-generated |
| Client identity | Persistent Ed25519 pubkey per device (`ClientSigner`) |
| Reconnect auth | `session_token` fast path; `Connect` challenge-response fallback (Ed25519, pubkey verified) |
| Host identity verified by client | Yes (challenge signed by host iroh key) |
| Session access control | Per-session ACL (two-level model) |
| Exclusive session ownership | Yes (one active client per session) |
| Session takeover | Explicit detach + `SessionTakenOver` close reason |
| Auth layer coupled to transport | No (`ClientSigner` trait, separate keypair) |
| Routing info in QR | No (pkarr resolves dynamically) |
| QR valid after IP change | Yes |
| Transport encryption | iroh TLS 1.3 |
| Host identity key | Persistent Ed25519 (iroh `SecretKey`) |

**Remaining exposure:** host IP visible to `dns.iroh.link` when relay is not
used. Mitigated by always using a relay (current default). Future: self-hosted
pkarr with encrypted records (see Section 8).

**Not yet addressed:** path traversal on filesystem RPCs (`PathBuf::join` does
not sanitize `..` or absolute paths).

---

## 8. Pending Work

- `zedra detach --session-id <id>` CLI subcommand (`force_detach` API exists)
- `SessionTakenOver` / `HostShutdown` QUIC APPLICATION_CLOSE codes actually sent
- `zedra sessions`, `zedra clients`, `zedra revoke` management subcommands
- Hardware-backed `ClientSigner` (Android Keystore, iOS Secure Enclave)
- Filesystem RPC path sanitization

---

## 9. Terminal I/O Multiplexing and Head-of-Line Blocking

Each `TermAttach` RPC opens a **separate QUIC bidi stream** via `conn.open_bi()`
(irpc-iroh 0.12). The multiplexing properties differ significantly depending on
whether the path is direct P2P or relayed.

### Direct path — true stream multiplexing

Over the direct QUIC/UDP path:

- Each terminal stream has **independent flow control** — backpressure on
  terminal A's stream does not affect terminal B's window.
- **No head-of-line (HoL) blocking** — QUIC packet loss on stream A triggers
  per-stream retransmission without stalling stream B.
- The only shared resource is **connection-level congestion control** (bandwidth
  shaping), which is fair between streams and does not serialize writes.
- On the host, each inbound bidi stream is dispatched with `tokio::spawn` in
  `rpc_daemon.rs`, so I/O handlers run fully concurrently.

**Result:** true independent multiplexing. A full-screen `top` in terminal 1
does not delay keystrokes in terminal 2.

### Relay path — TCP head-of-line blocking

The iroh relay (`iroh-relay 0.96`) uses **WebSocket over TCP** — not QUIC. The
full relay stack per client connection is:

```
QUIC bidi stream A ──┐
QUIC bidi stream B ──┤   iroh net layer
QUIC bidi stream C ──┘   encapsulates QUIC UDP datagrams as ClientToRelayDatagram frames
                              │
                              ▼
                     Single WebSocket (tokio_websockets) over TCP/TLS
                     RelayedStream { inner: WsBytesFramed<RateLimited<TcpStream>> }
                              │
                              ▼
                     Relay server Actor: send_queue (mpsc, depth=512) → single TCP stream
                              │
                              ▼
                     Single WebSocket over TCP/TLS to the mobile client
```

All QUIC streams share **one TCP connection** to the relay. TCP is an ordered
byte stream — a burst of terminal A output that fills the TCP send buffer stalls
all pending writes until the relay ACKs the data. Terminal B's keypress response
is delayed until A's burst drains.

The relay's `PER_CLIENT_SEND_QUEUE_DEPTH = 512` absorbs short bursts, and the
server actor applies a `write_timeout` per frame, but these do not eliminate the
fundamental TCP ordering constraint.

**`RelayQuicConfig` does not change this.** Setting
`quic: Some(iroh_relay::RelayQuicConfig::default())` in the host's endpoint
builder enables **QUIC Address Discovery** (ALPN `/iroh-qad/0`) on the relay
server — it is not a QUIC relay transport. The relay path remains WebSocket/TCP.

### Comparison

| | Direct P2P (QUIC/UDP) | Relay (WebSocket/TCP) |
|---|---|---|
| Transport | QUIC bidi streams over UDP | WebSocket frames over TCP |
| HoL blocking | None (stream-level independence) | Yes — TCP serializes all bytes |
| Per-terminal isolation | Full (flow control + loss recovery per stream) | None — all share one TCP connection |
| Congestion | Connection-level, fair | TCP congestion window, shared |
| Burst impact | Terminal A burst → only A slows | Terminal A burst → all terminals stall |

### Host-side serialization (fixed in this codebase)

Separately from the transport layer, the host had internal serialization points
that caused cross-terminal blocking regardless of path:

1. **Shared session backlog mutex** — a single `Mutex<VecDeque<BacklogEntry>>`
   across all terminals; PTY readers blocked each other writing output.
   Fixed by moving `TermBacklog` into `TermSession` (per-terminal `std::sync::Mutex`).

2. **PTY reader blocking on channel send** — `rt.block_on(tx.send(...))` under
   QUIC congestion could stall the OS read thread.
   Fixed with `try_send` + coalesce buffer — PTY threads never stall.

3. **Keystroke lock contention** — each keystroke acquired `session.terminals.lock()`
   to find the PTY writer.
   Fixed by extracting `Arc<Mutex<writer>>` at `TermAttach` setup time.

4. **Sequential terminal reattach on reconnect** — O(N × relay_RTT) reconnect time.
   Fixed with `tokio::task::JoinSet` for parallel concurrent attach.

These fixes eliminate all host-side bottlenecks. On the direct path this fully
restores independent per-terminal I/O. On the relay path, TCP HoL blocking
remains an inherent transport constraint; a QUIC-based relay transport would be
required to address it at the network layer.

---

## 10. Future Improvements

### Embed routing hints in the ticket for LAN connects

`ZedraPairingTicket` currently stores only `endpoint_id: iroh::PublicKey`.
This has two failure modes:

1. **No internet:** pkarr is unreachable; client cannot connect even on LAN.
2. **Relay mode:** `PkarrPublisher` publishes relay URL only when a relay is
   set, suppressing direct IPs. Even LAN clients go through the relay.

Fix: replace `endpoint_id` with `addr: iroh::EndpointAddr` (holds relay URL +
direct `SocketAddr`s). Filter before embedding: keep LAN ranges
(`192.168.x.x`, `10.x.x.x`, `172.16-31.x.x`) and relay URL; drop loopback
and ephemeral mapped public IPs. QR size increases by ~40 bytes (v4 → v6),
still fast-scanning on any modern device.

### Self-hosted pkarr with encrypted address records

Eliminates IP exposure to `dns.iroh.link`:

1. Self-host `iroh-dns-server` (or compatible pkarr relay)
2. Custom `AddressLookup` wrapper: publish `EndpointAddr` encrypted with
   AES-GCM-256, key = `HKDF(handshake_key, "zedra-addr-key")`. Only clients
   with the handshake key (i.e. QR scanners) can decrypt.
3. Relay operator sees only ciphertext; host IP never exposed.
4. Self-hosting also eliminates `EndpointId` lookup metadata observable by n0.

### LAN mDNS discovery

Deferred. Useful for fully offline / airgapped LAN usage and "nearby host"
auto-discovery in the home screen. Auth flow (QR + challenge-response) is
unchanged — mDNS only replaces the pkarr lookup step.

```toml
iroh = { version = "0.96", features = ["address-lookup-mdns"] }
```

```rust
let mdns = MdnsAddressLookup::builder()
    .service_name("zedra")
    .build(endpoint.id())?;
endpoint.address_lookup().add(mdns);
```

Android requires `android.permission.CHANGE_WIFI_MULTICAST_STATE`.
