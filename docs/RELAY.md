# Zedra Relay Architecture

Two relay options are maintained:

| Option | URL | Purpose |
|--------|-----|---------|
| **EC2 iroh-relay** | `https://sg1.relay.zedra.dev` | Production — self-hosted stateless iroh-relay on EC2 Singapore |
| **CF Worker relay** | `https://relay.zedra.dev` | Alternative — Cloudflare Workers + Durable Objects |

The active relay URL is the constant `ZEDRA_RELAY_URL` in `crates/zedra-rpc/src/lib.rs`.

## EC2 iroh-relay (production)

Standard open-source `iroh-relay` binary. See `deploy/relay/README.md` and
`deploy/relay/deploy.sh`.

**Measured latency (Vietnam → Singapore EC2):**
```
30 pings  min=112ms  avg=131ms  max=200ms
```

## CF Worker Relay

A re-implementation of the iroh relay wire protocol on Cloudflare Workers.
Wire-compatible with standard iroh clients — no fork required.

### Architecture

One `RelayRoom` Durable Object per host. Both the host and all its clients
connect to the same DO via the `?host=<hex>` query parameter. Datagram
forwarding is a pure in-memory `Map<hex, WebSocket>.send()` — no KV, no
DO-to-DO HTTP calls.

```
                  iroh Host                         iroh Client
                     │                                   │
         WS /relay?host=<hex>                 WS /relay?host=<hex>
                     │                                   │
                     ▼                                   ▼
              ┌─────────────────────────────────────────────┐
              │            CF Worker (edge)                 │
              │   routes both to same RelayRoom DO          │
              └─────────────────────────────────────────────┘
                                    │
                                    ▼
                     ┌─────────────────────────┐
                     │  RelayRoom DO           │
                     │  name: "room:<hostHex>" │
                     │                         │
                     │  _clients Map<hex, WS>  │
                     │  (in-memory, O(1))      │
                     └─────────────────────────┘
```

The DO is placed at the CF edge PoP nearest to whoever connects first (the
host). Subsequent connections are routed to the same PoP by CF's DO routing.

**Measured latency (Vietnam → Singapore CF PoP):**
```
30 pings  min=114ms  avg=143ms  max=241ms
```

The occasional spikes (200–240ms) are Durable Object hibernation wake-ups
(~80ms cold-start cost after ~5s idle).

### HTTP Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/` | Health check (`{"ok": true}`) |
| `GET` | `/ping` | HTTPS probe for iroh `net_report` |
| `GET` | `/generate_204` | Captive portal detection |
| `GET` | `/relay?host=<64-hex>` | WebSocket upgrade to RelayRoom DO |

### Wire Protocol

All frames: `[1B type][body]`. Type byte is a QUIC VarInt (single byte for
types 0–12).

| Type | Name | Direction | Body |
|------|------|-----------|------|
| 0x00 | ServerChallenge | S→C | `[16B]` random challenge |
| 0x01 | ClientAuth | C→S | `[32B pubkey][varint sigLen][sig]` (postcard) |
| 0x02 | ServerConfirmsAuth | S→C | (empty) |
| 0x03 | ServerDeniesAuth | S→C | `[varint len][UTF-8 reason]` |
| 0x04 | ClientToRelayDatagram | C→S | `[32B dst][1B ECN][data…]` |
| 0x05 | ClientToRelayDatagramBatch | C→S | `[32B dst][1B ECN][2B BE seg][data…]` |
| 0x06 | RelayToClientDatagram | S→C | `[32B src][1B ECN][data…]` |
| 0x07 | RelayToClientDatagramBatch | S→C | `[32B src][1B ECN][2B BE seg][data…]` |
| 0x08 | EndpointGone | S→C | `[32B endpoint_id]` |
| 0x09 | Ping | bidir | `[8B payload]` |
| 0x0a | Pong | bidir | `[8B payload]` |
| 0x0b | Health | S→C | `[UTF-8]` (raw, no length prefix) |
| 0x0c | Restarting | S→C | `[4B BE reconnect_ms][4B BE try_for_ms]` |

### ClientAuth Encoding

`ClientAuth` is postcard-serialized. The `signature` field uses
`#[serde(with = "serde_bytes")]`, which emits a varint length prefix:

```
[32B pubkey] [0x40] [64B signature]   = 97 bytes total
              ^^^^
         varint(64) = 1 byte
```

The challenge is not signed directly. BLAKE3 key derivation provides domain
separation:

```
context  = "iroh-relay handshake v1 challenge signature"
derived  = BLAKE3_derive_key(context, challenge_16B)
signature = Ed25519_sign(secret_key, derived)
```

### Handshake Flow

```
Client                               RelayRoom DO
  │                                       │
  │── GET /relay?host=<hex> (WS) ────────▶│  acceptWebSocket(server, ["pid:<uuid>"])
  │                                       │  store chal:<uuid> = random[16]
  │◀── ServerChallenge(challenge) ────────│
  │                                       │
  │── ClientAuth(pubkey, sig) ───────────▶│  verify Ed25519
  │                                       │  store pid:<uuid> = endpointIdHex
  │◀── ServerConfirmsAuth ────────────────│
  │                                       │
  │══ authenticated ═══════════════════════│
```

### Hibernation Recovery

CF hibernates the DO between messages. State is persisted in DO storage so
it survives:

- Each WebSocket is accepted with tag `["pid:<uuid>"]`
- On auth success: `storage["pid:<uuid>"] = endpointIdHex`
- On wake: `ensureClientsMap()` calls `storage.list("pid:")` + `getWebSockets()`
  to rebuild `_authed` and `_clients` maps

### Keepalive

An alarm fires every 15 ± 5 seconds. It sends a `Ping` frame to all
authenticated sockets to keep connections alive and detect stale ones.
The alarm is deleted when the last WebSocket disconnects.

### Source Files

```
packages/relay-worker/
  src/
    index.ts          — Worker router: health, ping, generate_204, relay WS upgrade
    relay-room.ts     — RelayRoom Durable Object
    frame-codec.ts    — Encode/decode all 13 frame types
    crypto.ts         — BLAKE3 KDF + Ed25519 verify
    types.ts          — Env bindings
    utils.ts          — jsonResponse, errorResponse
  wrangler.toml       — CF Worker config, DO migrations, Smart Placement
```
