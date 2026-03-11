# Network Transport

This document describes how Zedra connects a mobile client to a host daemon:
the QR pairing format, PKI authentication, address discovery via pkarr, the
connection state machine, and the authorization model.

---

## 1. QR Pairing Ticket

The QR payload is a `ZedraPairingTicket` encoded as a `zedra://` URL:

```
zedra://zedra<BASE32LOWER(postcard(ZedraPairingTicket))>
```

```rust
// zedra-rpc/src/pairing.rs

pub struct ZedraPairingTicket {
    /// Host's 32-byte Ed25519 public key.
    /// Used as pkarr lookup key and to verify AuthChallenge signatures.
    pub endpoint_id: iroh::PublicKey,
    /// 32-byte random key for this pairing slot.
    /// One-use: consumed by the first valid Register.
    /// Client uses this as the HMAC key to prove QR possession.
    pub handshake_key: [u8; 32],
    /// The session this QR was generated for.
    /// New client's pubkey is auto-added to this session's ACL on Register.
    pub session_id: String,
}
```

Routing info (relay URL, direct IPs) is **not** embedded — it is resolved
dynamically via pkarr at connect time, so the QR stays valid after IP changes.

### QR code size

Error correction level L (7% recovery).

| Payload | Raw bytes | Base32 chars | QR version |
|---------|-----------|--------------|------------|
| EndpointId (32) + handshake_key (32) + session_id (~8) | ~72 | ~116 | v4 |
| With embedded EndpointAddr (relay URL + 2 LAN IPs, future) | ~112 | ~180 | v6 |

### Handshake slot lifecycle

- **Created**: one slot per session at `zedra start` time (`rand::random::<[u8;32]>()`).
- **Stored**: in `SessionRegistry` (in-memory + `sessions.json`). A host crash
  before pairing invalidates the QR — restart the daemon to get a new one.
- **Consumed**: atomically on the first valid `Register`. Subsequent attempts
  return `HandshakeConsumed`.
- **Expires**: 10 minutes after creation regardless of use.

---

## 2. Connection Lifecycle

ALPN: `zedra/rpc/3`. All messages are postcard-serialized irpc requests.

### First pairing (after QR scan)

```
iroh QUIC + TLS 1.3 established
Client → host: Register { client_pubkey, timestamp, hmac }
  hmac = HMAC-SHA256(handshake_key, client_pubkey || timestamp_le_bytes)
  Proves sender physically scanned the QR (has handshake_key).
  Host verifies: |now - timestamp| ≤ 60s, HMAC valid, slot not consumed.
  On success: slot consumed, client_pubkey added to authorized_clients + session ACL.
Host → client: RegisterResult::Ok

Client → host: Authenticate { client_pubkey }
Host: generates 32-byte nonce, signs it with iroh SecretKey
Host → client: AuthChallengeResult { nonce, host_signature }
  Client MUST verify host_signature using the stored endpoint_id.

Client: signs nonce with application Ed25519 key
Client → host: AuthProve { nonce, client_signature, session_id }
  Host: verifies client_signature against stored pubkey, attaches session.
Host → client: AuthProveResult::Ok

→ RPC calls may proceed
```

### Reconnect (subsequent connections)

```
iroh QUIC + TLS 1.3 established

Client → host: Authenticate { client_pubkey }
Host → client: AuthChallengeResult { nonce, host_signature }
  Client verifies host_signature.

Client → host: AuthProve { nonce, client_signature, session_id }
Host → client: AuthProveResult::Ok

→ RPC calls may proceed
```

### Register result codes

| Code | Meaning |
|------|---------|
| `Ok` | Registered. Proceed to Authenticate. |
| `HandshakeConsumed` | Slot already used. Ask host to restart to get a new QR. |
| `InvalidHandshake` | HMAC failed. Wrong key or tampered packet. |
| `StaleTimestamp` | Clock skew > 60s or replay attempt. |
| `SlotNotFound` | No slot for this session. QR expired (> 10 min) or wrong session. |

### AuthProve result codes

| Code | Meaning |
|------|---------|
| `Ok` | Authenticated and attached. |
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
│ Reconnecting (attempt N / 10)                                   │
│  • Fresh pkarr resolve on each attempt (no cached addr)        │
│  • On connect: full Authenticate → AuthProve → Connected        │
│  • Backoff: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s, 30s, 30s     │
│  • After 10 failed attempts → Host Unreachable                 │
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
| Reconnect auth | Challenge-response (Ed25519, pubkey verified) |
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

## 9. Future Improvements

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
