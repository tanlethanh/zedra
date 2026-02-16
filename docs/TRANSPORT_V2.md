# Zedra Transport Protocol v2 (ZTP/2) - Design Document

## 1. Motivation & Problem Statement

Zedra connects a mobile client (Android) to a host daemon (desktop/server) for remote terminal, file system, and development tool access. The current transport layer (v1) works for demos but has fundamental limitations that prevent production use:

**Current gaps:**
1. **No encryption** - all data travels as plaintext JSON-RPC over TCP, base64 over HTTP relay. The relay server can read all terminal I/O, file contents, and credentials.
2. **No persistent identity** - each QR scan creates an ephemeral relay room. There's no concept of "this is my laptop" that persists across scans.
3. **Fragile reconnection** - 3 consecutive failures = permanent disconnect. Messages buffered during switch are lost on final failure. No sequence numbering or acknowledgments.
4. **HTTP polling relay** - 50ms-1s polling intervals add latency and waste bandwidth vs WebSocket.
5. **Single session per connection** - one QR scan = one working directory. Can't manage multiple projects without re-scanning.
6. **No dynamic discovery** - addresses are static from QR scan. If host IP changes, client has no way to find it again.
7. **No host presence** - host only exists in the relay system when a QR is actively scanned. No way for a previously-paired client to reconnect.

**Design goal:** A protocol where you scan a QR once to pair with a host, and from then on the connection is persistent, encrypted, and self-healing across any network change, transport failure, or device sleep/wake cycle.

---

## 2. Research: Reference Protocols

### 2.1 Mosh (Mobile Shell)
- **Key insight**: State Synchronization Protocol (SSP) over UDP. Instead of streaming bytes (like SSH/TCP), Mosh synchronizes terminal *state objects*. Can skip intermediate frames.
- **Roaming**: Single-packet roaming -- server updates client's address when it receives a valid authenticated packet from a new IP.
- **Reconnection**: Never gives up. Shows "last contact: 5m ago" to user.
- **Crypto**: AES-128-OCB3, custom protocol (not DTLS, because DTLS doesn't support roaming).
- **What we borrow**: Never-give-up reconnection philosophy, single-packet roaming via Connection ID, showing connection staleness to user.

### 2.2 WireGuard
- **Key insight**: Uses Noise_IKpsk2 handshake. Extremely simple (~4000 lines of code).
- **CryptoKey Routing**: Public key = identity = route.
- **Key rotation**: New ephemeral keys every 2 minutes for forward secrecy.
- **What we borrow**: Noise_IK handshake pattern, key rotation schedule, simplicity philosophy.

### 2.3 QUIC
- **Connection Migration**: Uses Connection ID (not IP:port tuple) to identify connections.
- **0-RTT Reconnection**: Previously-connected peers cache session parameters.
- **What we borrow**: Connection ID concept, PATH_CHALLENGE/RESPONSE for validating new transport paths.

### 2.4 Tailscale
- **Coordination Server**: A "shared drop box for public keys." Nodes register their public key + current addresses.
- **DERP Relay**: Globally distributed relays that forward already-encrypted WireGuard packets. Relay sees only encrypted blobs.
- **What we borrow**: Coordination server model, DERP-style encrypted relay, continuous path probing for upgrade.

### 2.5 WebRTC ICE/STUN/TURN
- **ICE**: Framework that gathers "candidates" and races them to find the best connection path.
- **What we borrow**: Candidate gathering pattern, priority-based concurrent racing.

### 2.6 Syncthing
- **Local Discovery**: UDP broadcast on port 21027 with device ID + addresses.
- **What we borrow**: UDP local discovery pattern, device ID format.

### 2.7 Magic Wormhole
- **Dilation Protocol**: 5 nested layers (Mailbox, Raw TCP, Connection selection, Durability, Subchannels).
- **Generation-based reconnect**: Each transport connection = a "generation". L4 replays unACKed messages on new connection.
- **What we borrow**: Layered architecture (the biggest influence), generation-based reconnection, durable message queue with seq/ACK, subchannel multiplexing.

### 2.8 SSH Multiplexing
- **ControlMaster**: Single TCP connection carries multiple SSH sessions.
- **What we borrow**: Channel multiplexing model, session persistence across channel open/close.

### 2.9 Noise Protocol Framework
- **IK pattern** (our choice): Initiator knows responder's static key (from QR), sends its own static key encrypted. 1-RTT handshake.
- **What we borrow**: IK pattern for primary handshake, XX as fallback, the entire framework via `snow` Rust crate.

---

## 3. Protocol Architecture

### 3.1 Layer Diagram

```
+------------------------------------------------------------------+
|  Layer 6: Host Daemon                                            |
|  +------------------+ +-----------------+ +-------------------+  |
|  | Session Registry | | Trust Store     | | Coord Registration|  |
|  | (multi-workdir)  | | (paired devices)| | (heartbeat loop)  |  |
|  +------------------+ +-----------------+ +-------------------+  |
+------------------------------------------------------------------+
|  Layer 5: Session Multiplexing                                   |
|  +------+ +----------+ +----+ +-----+ +-----+                   |
|  | Ctrl | | Terminal  | | FS | | Git | | LSP |  <- subchannels   |
|  | ch:0 | | ch:1,2.. | |ch:N| |ch:N | |ch:N |                   |
|  +--+---+ +----+-----+ +--+-+ +--+--+ +--+--+                   |
|     +----------+----------+------+-------+                       |
|                    Multiplexer                                   |
+------------------------------------------------------------------+
|  Layer 4: Durability & Migration                                 |
|  +----------------+ +--------------+ +---------------------+    |
|  | Durable Queue   | | Reconnection | | Transport Migration |    |
|  | (seq/ACK/replay)| | State Machine| | (probe -> switch)   |    |
|  +----------------+ +--------------+ +---------------------+    |
+------------------------------------------------------------------+
|  Layer 3: Secure Channel                                         |
|  +--------------+ +-------------------+ +------------------+    |
|  | Noise_IK     | | ChaCha20-Poly1305 | | Connection ID    |    |
|  | Handshake    | | Encryption        | | (32-byte token)  |    |
|  +--------------+ +-------------------+ +------------------+    |
+------------------------------------------------------------------+
|  Layer 2: Discovery & Signaling                                  |
|  +------------+ +--------------+ +------------------------+     |
|  | mDNS/UDP   | | Coord Server | | ICE-like Candidate     |     |
|  | Local Disc. | | Signaling    | | Racing                 |     |
|  +------------+ +--------------+ +------------------------+     |
+------------------------------------------------------------------+
|  Layer 1: Pairing & Trust                                        |
|  +----------+ +----------------+ +----------------------+       |
|  | QR Scan  | | Noise_IK + OTP | | Persistent Trust     |       |
|  | (v3 fmt) | | (first contact)| | Store (keypair-based)|       |
|  +----------+ +----------------+ +----------------------+       |
+------------------------------------------------------------------+
|  Layer 0: Identity & Registration                                |
|  +----------------+ +---------------+ +---------------------+   |
|  | Curve25519      | | Device ID     | | Coord Server        |   |
|  | Keypair (local) | | (key fingerp.)| | Registration + HB   |   |
|  +----------------+ +---------------+ +---------------------+   |
+------------------------------------------------------------------+
|  Physical Transports (interchangeable)                           |
|  +---------+ +------------+ +-----------+ +------------------+ |
|  | LAN TCP | | Tailscale  | | WebSocket | | HTTP Relay       | |
|  |         | | TCP        | | (relay)   | | (legacy compat)  | |
|  +---------+ +------------+ +-----------+ +------------------+ |
+------------------------------------------------------------------+
```

### 3.2 Core Design Principles

1. **Public key = identity.** Inspired by WireGuard's cryptokey routing and Syncthing's device IDs. A device is its Curve25519 public key.
2. **Encrypt everything, trust nothing.** Every byte on every transport is encrypted with ChaCha20-Poly1305 via Noise_IK. The relay server sees only ciphertext.
3. **Never give up.** Inspired by Mosh. Connection is logically permanent once paired. Show staleness to user, let them explicitly disconnect.
4. **Pair once, connect forever.** QR scan establishes mutual trust. All future connections use key-based authentication.
5. **Transport is a detail.** Layers 4+ don't know or care whether bytes travel over LAN TCP, Tailscale, WebSocket, or carrier pigeon.
6. **Layered like an onion.** Each layer has a single responsibility. Inspired by Magic Wormhole's L1-L5 dilation architecture.

---

## 4. Layer 0: Identity & Registration

### 4.1 Device Identity

Every Zedra device has a **persistent Curve25519 keypair**.

```
Key Storage (host):   ~/.config/zedra/identity.key    (private)
                      ~/.config/zedra/identity.pub    (public)
Key Storage (mobile): Android Keystore (hardware-backed if available)
```

**Device ID** = truncated SHA256 of public key, formatted as Syncthing-style chunks:

```
RLKQ4WE-GLLHZT5-7QFG3G2-VFI3HTG-XFQTPNL-BNVHJ6Q-WDHYQFP-XWIQTAH
```

### 4.2 Coordination Server

The coordination server (upgrade of current relay-worker) acts as a **presence registry and signaling relay**. It does NOT relay data traffic.

**Responsibilities:**
1. Host registration: "I am device X, reachable at these addresses"
2. Presence: "Is device X online?"
3. Signaling: "Forward these connection candidates from client Y to host X"
4. Relay brokering: "Allocate a relay room for client Y to reach host X"

**Registration API:**
```
POST /v2/hosts/register
Authorization: Bearer <signed-challenge>
{
  "device_id": "RLKQ4WE-...",
  "public_key": "base64url(32 bytes)",
  "hostname": "my-laptop",
  "addresses": [
    { "type": "lan", "addr": "192.168.1.100:2123" },
    { "type": "tailscale", "addr": "100.64.0.5:2123" }
  ],
  "sessions": [
    { "id": "uuid-1", "name": "zedra", "workdir": "/home/user/projects/zedra" }
  ],
  "capabilities": ["terminal", "fs", "git", "lsp"],
  "version": "0.2.0"
}
```

### 4.3 Network Change Handling

When the host's network changes:
1. OS notifies daemon of network interface change
2. Daemon gathers new addresses
3. Immediately sends updated registration to coord server
4. Clients see new addresses on next lookup

---

## 5. Layer 1: Pairing & Trust

### 5.1 QR Pairing (v3 Format)

**QR Payload v3:**
```json
{
  "v": 3,
  "host_pubkey": "base64url(32 bytes, Curve25519 public key)",
  "session_id": "uuid",
  "workdir": "/home/user/projects/zedra",
  "otp": "base64url(32 bytes, one-time pairing token)",
  "coord_url": "https://coord.zedra.dev",
  "hints": {
    "addrs": ["192.168.1.100:2123"],
    "tailscale": "100.64.0.5:2123",
    "relay": "wss://relay.zedra.dev/r/abc123"
  }
}
```

Encoded as: `zedra://pair?d=<base64url(json)>` -> QR code

### 5.2 Pairing Handshake (Noise_IK)

```
Client (Initiator)                          Host (Responder)
------------------                          ----------------
Knows: host_pubkey (from QR)                Knows: own static keypair

1. Noise_IK message 1:
   -> e, es, s, ss
   [encrypted: { otp, client_device_id }]
                                            2. Validate OTP
                                            3. Store client in trust store
                                            4. Noise_IK message 2:
                                            <- e, ee, se
                                               [encrypted: { ok: true }]
5. Store host in trust store
6. Secure channel established
```

### 5.3 Trust Store

After successful pairing, both sides persist trust:

**Host** (`~/.config/zedra/trust.json`):
```json
{
  "trusted_clients": [{
    "device_id": "ABC123-...",
    "public_key": "base64url(...)",
    "name": "Thomas's Pixel",
    "paired_at": "2026-02-16T10:00:00Z"
  }]
}
```

### 5.4 Reconnection (No QR Needed)

Previously-paired clients reconnect with Noise_IK using stored keys. No OTP needed -- trust is key-based.

---

## 6. Layer 2: Discovery & Signaling

### 6.1 Candidate Types

| Priority | Type | Description | Latency |
|----------|------|-------------|---------|
| 0 | `direct-lan` | Same LAN, direct TCP | <1ms |
| 1 | `direct-tailscale` | Tailscale mesh | 1-10ms |
| 2 | `direct-public` | Public IP (future) | 10-50ms |
| 3 | `relay-ws` | WebSocket relay | 50-200ms |
| 4 | `relay-http` | HTTP polling (legacy) | 100-1000ms |

### 6.2 Candidate Racing

```
Phase 1 (0-500ms):   Try all direct-lan + tailscale candidates
Phase 2 (500ms-5s):  Continue direct + start relay-ws
Phase 3 (5s-10s):    Fallback to relay-http if needed
```

---

## 7. Layer 3: Secure Channel

### 7.1 Noise_IK Handshake

Cipher suite: `Noise_IK_25519_ChaChaPoly_BLAKE2s`
- Key exchange: Curve25519
- Cipher: ChaCha20-Poly1305
- Hash: BLAKE2s

### 7.2 Encrypted Frame Format

```
Wire format:
+-------------------+-----------+--------------------------------------------+
| Connection ID     | Length    | Encrypted Payload + Auth Tag                |
| 32 bytes          | 4 bytes  | variable + 16 bytes                         |
| (plaintext)       |(plaintext)| (ChaCha20-Poly1305)                        |
+-------------------+-----------+--------------------------------------------+

Encrypted payload (after decryption):
+----------+----------+------------------------------+
| Counter  | Type     | Inner Payload                 |
| 8 bytes  | 1 byte   | variable                      |
| (u64 LE) |          |                               |
+----------+----------+------------------------------+
```

**Frame types:**
- 0x01 = DATA (Layer 4+ payload)
- 0x02 = PING (keepalive)
- 0x03 = PONG (keepalive response)
- 0x04 = REKEY (key rotation)
- 0x05 = CLOSE (graceful close)
- 0x06 = PATH_CHALLENGE
- 0x07 = PATH_RESPONSE

### 7.3 Key Rotation

Every 120 seconds or every 2^32 messages: exchange new ephemeral DH keys, derive new symmetric keys, zero old keys.

---

## 8. Layer 4: Durability & Migration

### 8.1 Durable Message Queue

Every L5 frame gets a monotonically increasing sequence number. ACKs are piggybacked on data frames.

On reconnect: both sides exchange RESUME frames with last_received_seq, then replay unACKed messages.

### 8.2 Generation Model

Each physical transport connection is a "generation." Generations are non-overlapping. L4 queue bridges across generations.

### 8.3 Reconnection State Machine

```
Connected -> (transport failure) -> Reconnecting
Reconnecting -> Discovering -> Handshaking -> Resuming -> Connected
Backoff: 1s -> 2s -> 4s -> 8s -> 16s -> 30s (cap)
Never gives up.
```

---

## 9. Layer 5: Session Multiplexing

### 9.1 Subchannel Protocol

```
Frame Types:
  OPEN     = 0x01  { channel_id, subprotocol, initial_window }
  OPEN_ACK = 0x02  { channel_id, ok, error? }
  DATA     = 0x03  { channel_id, payload }
  CLOSE    = 0x04  { channel_id, reason? }
  ACK      = 0x05  { channel_id, bytes_consumed }
  FLOW     = 0x06  { channel_id, window_update }
```

### 9.2 Subprotocols

| Subprotocol | Description | Channel lifetime |
|-------------|-------------|-----------------|
| `control` | Session management | Always open (ch:0) |
| `terminal` | PTY I/O | Open per terminal |
| `fs` | File system (JSON-RPC) | On demand |
| `git` | Git operations | On demand |
| `lsp` | Language server | Persistent |

### 9.3 Per-Channel Flow Control

SSH-style sliding window. Default: 65536 bytes. Prevents bulk transfers from starving terminal I/O.

---

## 10. Wire Protocol Summary

### 10.1 L3 Outer Frame

```
Connection ID (32 bytes) | Length (4 bytes BE u32) | Encrypted envelope
```

### 10.2 L4 Durable Frame

```
Sequence (u64 LE) | ACK Seq (u64 LE) | Type (1 byte) | Payload
Types: RESUME=0x01, DATA=0x02, ACK=0x03, RESET=0x04
```

### 10.3 L5 Subchannel Frame

```
Channel ID (u32 LE) | Type (1 byte) | Payload
Types: OPEN=0x01, OPEN_ACK=0x02, DATA=0x03, CLOSE=0x04, ACK=0x05, FLOW=0x06
```

---

## 11. Crate Structure

```
crates/
  zedra-identity/    <- NEW: keypair, device ID, trust store
  zedra-crypto/      <- NEW: Noise_IK, secure channel, connection ID
  zedra-transport/   <- MAJOR REWRITE: generation model, durable queue
  zedra-rpc/         <- ENHANCE: channel-aware framing
  zedra-host/        <- ENHANCE: multi-session, trust management
  zedra-relay/       <- ADD: WebSocket support
```

---

## 12. Implementation Phases

### Phase 1: Identity & Encryption (Foundation)
- Create `zedra-identity` crate
- Create `zedra-crypto` crate
- Integrate `snow` for Noise_IK
- Wrap existing TCP transport with secure channel
- **Milestone**: E2E encrypted terminal session over LAN

### Phase 2: Durable Queue & Reconnection
- Implement `DurableQueue` with seq/ACK
- Rewrite `TransportManager` with generation state machine
- **Milestone**: Kill network, reconnect, no lost output

### Phase 3: Coordination Server
- Add host registry to relay-worker
- Host registration loop, client lookup API
- QR v3 format
- **Milestone**: Paired client reconnects after reboot

### Phase 4: Enhanced Discovery
- mDNS local discovery
- Signaling via coord server
- Android network change detection
- **Milestone**: Switch wifi, auto-reconnect <5s

### Phase 5: WebSocket Relay
- WebSocket transport provider
- Replace HTTP polling
- **Milestone**: Relay latency 50-100ms

### Phase 6: Channel Multiplexing
- Subchannel protocol
- Per-channel flow control
- **Milestone**: File transfer doesn't block terminal

### Phase 7: Multi-Session Host
- Named sessions per working directory
- Multi-client support
- Enhanced CLI
- **Milestone**: Two phones, two project sessions

---

## 13. Security Model

| Threat | Mitigation |
|--------|-----------|
| Eavesdropping | Noise_IK + ChaCha20-Poly1305 |
| Relay reads data | E2E encryption; relay sees only ciphertext |
| MITM | QR establishes host pubkey out-of-band |
| Replay | Monotonic counter; window-based anti-replay |
| Old key compromise | Key rotation every 2 min; forward secrecy |
| Unauthorized client | Trust store with explicit pairing |

---

## References

- [Mosh](https://mosh.org/) -- Mobile Shell
- [WireGuard](https://www.wireguard.com/protocol/)
- [QUIC 0-RTT](https://blog.cloudflare.com/even-faster-connection-establishment-with-quic-0-rtt-resumption/)
- [Tailscale](https://tailscale.com/blog/how-tailscale-works)
- [Syncthing Local Discovery v4](https://docs.syncthing.net/specs/localdisco-v4.html)
- [Magic Wormhole Dilation](https://magic-wormhole.readthedocs.io/en/latest/dilation-protocol.html)
- [Noise Protocol Framework](https://noiseprotocol.org/noise.html)
- [`snow` Rust crate](https://docs.rs/snow/)
