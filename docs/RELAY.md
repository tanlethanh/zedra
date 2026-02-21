# Zedra Relay — iroh-Compatible Relay on Cloudflare Workers

## Overview

Zedra Relay is a Cloudflare Workers implementation that is wire-compatible with
iroh's relay protocol. Each connected iroh `Endpoint` gets its own Durable Object
(DO). KV acts as the distributed routing table. No iroh fork is needed — standard
iroh clients connect directly.

```
                 iroh Endpoint A                    iroh Endpoint B
                      │                                   ▲
                      │ WebSocket                         │ WebSocket
                      ▼                                   │
              ┌───────────────┐                   ┌───────────────┐
              │  CF Worker    │                   │  CF Worker    │
              │  (edge node)  │                   │  (edge node)  │
              └───────┬───────┘                   └───────┬───────┘
                      │                                   │
                      ▼                                   ▼
              ┌───────────────┐   DO-to-DO        ┌───────────────┐
              │ RelayEndpoint │ ──────────────────▶│ RelayEndpoint │
              │   DO (A)      │   fetch("/forward")│   DO (B)      │
              └───────┬───────┘                   └───────┬───────┘
                      │                                   │
                      │  KV lookup:                       │
                      │  relay:ep:{B_hex} → { do_name }   │
                      ▼                                   │
              ┌───────────────┐                           │
              │  ZEDRA_RELAY  │ ◀─────────────────────────┘
              │     KV        │    KV put on handshake
              └───────────────┘
```

## HTTP Endpoints

The Worker serves these HTTP endpoints required by iroh clients:

| Method | Path            | Purpose                                          |
| ------ | --------------- | ------------------------------------------------ |
| `GET`  | `/`             | Health check (`{"ok": true}`)                    |
| `GET`  | `/ping`         | HTTPS probe for iroh `net_report` latency probes |
| `GET`  | `/generate_204` | Captive portal detection (returns 204)           |
| `GET`  | `/relay`        | WebSocket upgrade to `RelayEndpoint` DO          |

## Wire Protocol

### Frame Format

All frames use a QUIC VarInt type prefix. For types 0–12 (< 64), this is a
single byte.

```
┌──────────┬──────────────────┐
│ Type (1B)│    Body (var)    │
└──────────┴──────────────────┘
```

### Frame Types

| Type | Name                       | Direction | Body                                            |
| ---- | -------------------------- | --------- | ----------------------------------------------- |
| 0x00 | ServerChallenge            | S→C       | `[16B challenge]`                               |
| 0x01 | ClientAuth                 | C→S       | `[32B pubkey][0x40][64B signature]` (postcard)  |
| 0x02 | ServerConfirmsAuth         | S→C       | (empty)                                         |
| 0x03 | ServerDeniesAuth           | S→C       | `[varint len][UTF-8 reason]` (postcard)         |
| 0x04 | ClientToRelayDatagram      | C→S       | `[32B dst_id][1B ECN][data...]`                 |
| 0x05 | ClientToRelayDatagramBatch | C→S       | `[32B dst_id][1B ECN][2B BE seg_size][data...]` |
| 0x06 | RelayToClientDatagram      | S→C       | `[32B src_id][1B ECN][data...]`                 |
| 0x07 | RelayToClientDatagramBatch | S→C       | `[32B src_id][1B ECN][2B BE seg_size][data...]` |
| 0x08 | EndpointGone               | S→C       | `[32B endpoint_id]`                             |
| 0x09 | Ping                       | bidir     | `[8B payload]`                                  |
| 0x0a | Pong                       | bidir     | `[8B payload]`                                  |
| 0x0b | Health                     | S→C       | `[UTF-8 problem]` (raw, no length prefix)       |
| 0x0c | Restarting                 | S→C       | `[4B BE reconnect_ms][4B BE try_for_ms]`        |

### Postcard Encoding Details

iroh serializes handshake frames with `postcard` (a no_std Rust serializer):

- **ServerChallenge**: `{ challenge: [u8; 16] }` — 16 raw bytes (fixed-size array, no prefix)
- **ClientAuth**: `{ public_key: [u8; 32], signature: [u8; 64] }`
  - `public_key` is a fixed array → 32 raw bytes
  - `signature` uses `#[serde(with = "serde_bytes")]` → postcard emits varint length prefix
  - varint(64) = `0x40` (1 byte, since 64 < 128)
  - Total body: 32 + 1 + 64 = 97 bytes
- **ServerConfirmsAuth**: `{}` — 0 bytes
- **ServerDeniesAuth**: `{ reason: String }` — postcard varint length + UTF-8

## Handshake Flow

```
Client                                    Server (DO)
  │                                          │
  │──── WebSocket upgrade GET /relay ───────▶│
  │                                          │ generate 16 random bytes
  │◀──── ServerChallenge(challenge) ─────────│
  │                                          │
  │ derive = BLAKE3_KDF(                     │
  │   "iroh-relay handshake v1               │
  │    challenge signature",                 │
  │   challenge)                             │
  │ sig = Ed25519_Sign(secret_key, derive)   │
  │                                          │
  │──── ClientAuth(pubkey, sig) ────────────▶│
  │                                          │ verify Ed25519(pubkey,
  │                                          │   BLAKE3_KDF(domain, challenge),
  │                                          │   sig)
  │◀──── ServerConfirmsAuth ─────────────────│ register in KV
  │                                          │
  │ ═══ Authenticated session ═══════════════│
```

### Domain Separation

The challenge is not signed directly. Instead, BLAKE3's key derivation function
(KDF mode) is used:

```
context = "iroh-relay handshake v1 challenge signature"
derived  = BLAKE3_derive_key(context, challenge)
signature = Ed25519_sign(secret_key, derived)
```

This ensures the signature is domain-separated and cannot be replayed in other
contexts.

## DO Topology

### One DO per Endpoint

Each iroh `Endpoint` that connects gets its own `RelayEndpoint` Durable Object.
The DO name is derived from the endpoint's public key hex:

```
DO name = "ep:" + hex(public_key)
```

### KV Routing Table

After handshake, each DO registers itself in KV:

```
Key:   relay:ep:{public_key_hex}
Value: { "do_name": "ep:{public_key_hex}" }
TTL:   90 seconds (refreshed by alarm)
```

When endpoint A wants to send to endpoint B:

1. A sends `ClientToRelayDatagram` with B's public key as `dst_id`
2. A's DO looks up `relay:ep:{B_hex}` in KV
3. If found, A's DO calls `B_DO.fetch("/forward")` with the pre-encoded frame
4. B's DO pushes the frame to B's WebSocket
5. If not found, A's DO sends `EndpointGone(B)` back to A

### DO-to-DO Communication

```
A's DO ──fetch("/forward", body=encoded_frame)──▶ B's DO
                                                     │
                                               ws.send(frame)
                                                     │
                                                     ▼
                                                 Endpoint B
```

The `/forward` endpoint on each DO accepts a binary body (the pre-encoded
`RelayToClientDatagram` or `RelayToClientDatagramBatch` frame) and pushes it
directly to the connected WebSocket. Returns:

- 200: delivered
- 410: no client connected (endpoint gone)

## Keepalive and Alarm Strategy

Each `RelayEndpoint` DO uses the CF alarm API for two purposes:

1. **Keepalive Pings**: Send `Ping` frame every 15s (± 5s jitter) to detect
   stale connections. If the client doesn't respond, the WebSocket error/close
   handler fires and cleans up.

2. **KV TTL Refresh**: Re-PUT the KV routing entry with a fresh 90s TTL on
   each alarm tick. This ensures the entry doesn't expire while the client is
   still connected.

```
Alarm fires (every 15s ± 5s):
  1. Send Ping(random 8 bytes) to client
  2. PUT relay:ep:{hex} → { do_name } with 90s TTL
  3. Schedule next alarm
```

### Handshake Timeout

A 15-second alarm is set when the WebSocket is first accepted. If the client
hasn't completed the handshake by then, the alarm fires a Ping which will fail
(no authenticated session), and the connection is closed.

## Cost Model

| Resource           | Usage                                     | CF Pricing    |
| ------------------ | ----------------------------------------- | ------------- |
| Worker invocations | 1 per WS upgrade + 1 per DO-to-DO forward | $0.30/M       |
| DO requests        | 1 per WS message + 1 per forward          | $0.15/M       |
| DO duration        | ~15s between alarms (hibernation)         | $12.50/M GB-s |
| KV reads           | 1 per datagram (route lookup)             | $0.50/M       |
| KV writes          | 1 per alarm tick (TTL refresh)            | $5.00/M       |

For a typical session (100 datagrams/sec, 2 endpoints, 1 hour):

- KV reads: ~360K → ~$0.18
- KV writes: ~480 (alarm ticks) → ~$0.002
- DO requests: ~720K → ~$0.11
- Total: ~$0.30/hour for active relay

## Failure Modes

| Failure              | Detection                    | Recovery                                  |
| -------------------- | ---------------------------- | ----------------------------------------- |
| Client disconnects   | `webSocketClose` fires       | Delete KV entry, clean DO state           |
| DO hibernation       | Automatic by CF runtime      | Alarm wakes DO, state persists in storage |
| KV TTL expiry        | Sender gets no route         | `EndpointGone` sent; client reconnects    |
| DO-to-DO forward 410 | Target has no WS             | `EndpointGone` sent to sender             |
| Invalid handshake    | Signature verification fails | `ServerDeniesAuth` + close WebSocket      |
| Alarm missed         | KV entry expires (90s)       | Client reconnect re-registers             |
| Worker restart       | `Restarting` frame sent      | Client reconnects after `reconnect_ms`    |
