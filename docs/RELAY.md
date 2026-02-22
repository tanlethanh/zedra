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

## In-Memory Caching

### Problem

Every datagram forwarded required a KV read for route lookup and DO storage
reads for `authenticated` and `endpoint_id`. At 100 datagrams/sec, this meant
100 KV reads/sec and 200 DO storage reads/sec — the dominant cost driver.

### Solution

`RelayEndpoint` maintains in-memory caches for both KV routes and DO storage
values. These caches survive while the DO is awake and are lost on hibernation.

```
Datagram arrives
    ↓
_authenticated cached? → yes → skip storage read
    ↓                     no → read from storage, cache it
_endpointId cached?   → yes → skip storage read
    ↓                     no → read from storage, cache it
_routeCache hit?      → yes → skip KV read, use cached DO name
    ↓                     no → KV read, cache result (10s TTL)
DO-to-DO forward
```

**Cached fields** (populated on auth success, lazy-loaded after hibernation wake):

- `_authenticated` — avoids `storage.get("authenticated")` per message
- `_endpointId` — avoids `storage.get("endpoint_id")` per datagram
- `_endpointIdHex`, `_doName` — avoids storage reads in alarm handler
- `_routeCache` — `Map<dstHex, { doName, cachedAt }>` with 10s TTL

### Why This Works with Hibernation

The Hibernation API clears all in-memory state when the DO sleeps. This sounds
like it would defeat caching, but:

1. **Hibernation only happens when idle.** During active datagram transfer (when
   KV reads are highest), the DO stays awake and the cache stays warm.
2. **When idle, there are no KV reads anyway.** No datagrams = no route lookups.
3. **Self-correcting on stale routes.** If an endpoint reconnects to a new DO,
   the cached route returns 410 from the old DO. The cache entry is invalidated
   and refreshed from KV on the next miss.

### Invariants (DO NOT VIOLATE)

1. **Always update both storage and cache** when writing values that are cached.
   If you add `storage.put("foo", val)`, also set `this._foo = val`.
2. **Always invalidate cache on cleanup.** The `cleanup()` method must reset all
   cache fields to `undefined` and clear `_routeCache`.
3. **Never trust cached routes after a 410 or fetch error.** Always delete the
   cache entry so the next attempt re-reads from KV.
4. **Route cache TTL must be short** (currently 10s). Longer TTLs increase the
   window where a stale route causes unnecessary `EndpointGone` messages.

## Cost Model

| Resource           | Usage                                          | CF Pricing    |
| ------------------ | ---------------------------------------------- | ------------- |
| Worker invocations | 1 per WS upgrade + 1 per DO-to-DO forward      | $0.30/M       |
| DO requests        | 1 per WS message + 1 per forward               | $0.15/M       |
| DO duration        | ~15s between alarms (hibernation)              | $12.50/M GB-s |
| KV reads           | ~1 per 10s per dest (route cache, was 1/dgram) | $0.50/M       |
| KV writes          | 1 per alarm tick (TTL refresh)                 | $5.00/M       |
| DO storage reads   | ~3-4 per wake (cached, was 2/dgram)            | $0.20/M       |

### Datagram Rate Assumptions

The cost model uses **100 datagrams/sec** as a baseline. This represents a
moderately active terminal session where iroh QUIC packets flow between the
Android client and the host. Datagrams are generated by:

- **Terminal output** — each chunk of stdout/stderr becomes one or more QUIC
  datagrams relayed to the client
- **Terminal input** — keystrokes from the client relayed to the host (much
  lower rate)
- **QUIC ACKs/control** — acknowledgments and congestion control frames

| Scenario                            | Approx. rate      |
| ----------------------------------- | ----------------- |
| Idle terminal, occasional typing    | 5–10 dgram/sec    |
| Interactive use (shell, vim)        | 20–50 dgram/sec   |
| Sustained output (build, cat, grep) | 100–200 dgram/sec |
| Burst (large ls, log dump)          | 500+ dgram/sec    |

Realistic average across all sessions is closer to **10–20 dgram/sec** since
most sessions are idle most of the time. The 100/sec baseline represents a
worst-case "all users actively streaming" scenario.

### Per-Session Cost (100 datagrams/sec, 2 endpoints, 1 hour)

**Before caching:**

- KV reads: ~360K → ~$0.18
- DO storage reads: ~720K → ~$0.14
- KV writes: ~480 → ~$0.002
- DO requests: ~720K → ~$0.11
- Total: ~$0.43/hour

**After caching:**

- KV reads: ~720 (1 per 10s per dest × 2 endpoints) → ~$0.0004
- DO storage reads: ~16 (4 fields × ~4 wake events) → ~$0.000003
- KV writes: ~480 → ~$0.002
- DO requests: ~720K → ~$0.11
- Total: ~$0.11/hour (74% reduction, KV reads reduced 99.8%)

### Projected Cost at Scale

Assumes 100 dgram/sec per session (worst-case), 2 endpoints per session, all
users active simultaneously. Realistic costs would be 5–10x lower since most
sessions are idle most of the time.

| Metric               | 1,000 users (500 sessions) | 10,000 users (5,000 sessions) |
| -------------------- | -------------------------- | ----------------------------- |
| KV reads/hr (before) | 180M                       | 1.8B                          |
| KV reads/hr (after)  | 360K                       | 3.6M                          |
| KV cost/month before | ~$64,800                   | ~$648,000                     |
| KV cost/month after  | ~$130                      | ~$1,296                       |
| DO requests/hr       | 360M                       | 3.6B                          |
| DO req cost/month    | ~$39,000                   | ~$390,000                     |

At scale, **DO requests** (unchanged by caching) become the dominant cost.
Batching could reduce this, but the fundamental issue is the billing model
(see below).

### Why Serverless Is Not Viable for Relay

The CF Worker relay was built on the assumption that serverless would optimize
cost through pay-per-use efficiency. In practice, the opposite is true:

**Serverless billing penalizes high-frequency small messages.** CF charges per
DO request ($0.15/M), per KV read ($0.50/M), and per Worker invocation
($0.30/M). A relay forwards thousands of small QUIC datagrams per second —
each one incurs per-message billing. This is the worst-case workload for
serverless pricing, which is optimized for infrequent, bursty HTTP requests.

**Comparison at 1,000 concurrent users:**

| Approach                  | Estimated monthly cost |
| ------------------------- | ---------------------- |
| CF Worker relay (current) | ~$39,000               |
| Self-hosted `iroh-relay`  | ~$50–100 (VPS)         |
| iroh public relays (n0)   | $0 (free, no SLA)      |

The CF Worker is 400–800x more expensive than self-hosting for the same
workload. The `iroh-relay` binary is stateless, horizontally scalable, and
uses a persistent connection model with zero per-message overhead.

**Recommendation:** Migrate to self-hosted `iroh-relay` or iroh's default
public relays. The CF Worker relay remains useful for development and low-
traffic scenarios but should not be used at scale.

### Serverless Ceiling: Shared DO Optimization

The maximum optimization possible on CF Workers is a **shared DO model** where
both endpoints connect to the same Durable Object (keyed by a shared room ID
from QR pairing). This eliminates DO-to-DO forwards and all KV usage:

```
Current:  A's WS → A's DO → KV → DO-to-DO fetch → B's DO → B's WS  (2 DO req)
Shared:   A's WS → Shared DO → B's WS (local push)                  (1 DO req)
```

Even with this optimization, the cost floor is **~$8,100/mo at 1K users** —
still 100x more expensive than self-hosting. The per-message billing model
cannot be optimized past this point.

## Hosting Platform Analysis

### Bandwidth Model

A relay forwards every datagram bidirectionally — each byte in must go back
out. Egress is the dominant cost factor on most platforms.

| Users | Sessions | Outbound/month (realistic 20 dgram/sec) | Outbound/month (worst-case 100 dgram/sec) |
| ----- | -------- | --------------------------------------- | ----------------------------------------- |
| 100   | 50       | 2.5 TB                                  | 12.7 TB                                   |
| 1,000 | 500      | 25 TB                                   | 127 TB                                    |
| 10K   | 5,000    | 253 TB                                  | 1,266 TB                                  |

### Platform Comparison (realistic traffic, monthly cost)

| Platform                   | Billing model          | 100 users | 1K users   | 10K users   |
| -------------------------- | ---------------------- | --------- | ---------- | ----------- |
| **CF Workers (current)**   | Per-message            | ~$5       | ~$39,000   | ~$390,000   |
| **CF Workers (shared DO)** | Per-message (optimized)| ~$3       | ~$8,100    | ~$81,000    |
| **AWS API GW + Lambda**    | Per-message            | ~$17,300  | ~$173,000  | ~$1,700,000 |
| **AWS EC2 t4g.small**      | Per-GB egress ($0.09)  | $241      | $2,201     | $17,203     |
| **AWS Lightsail $12**      | 3 TB included + $0.09  | **$12**   | $1,941     | $22,364     |
| **Railway**                | Per-GB egress ($0.10)  | $263      | $2,513     | $25,313     |
| **Fly.io**                 | Per-GB egress ($0.02)  | $5        | $265       | $2,610      |
| **Hetzner CAX11**          | 20 TB included         | **$4**    | **$8**     | **~$50**    |
| **n0des Cloud (managed)**  | Flat rate              | $199      | $199+      | $400–800    |
| **n0 public relays**       | Free                   | **$0**    | **$0**     | **$0**      |

### Why Per-GB Egress Fails for Relay

A relay's entire job is moving bytes in and out. Every platform that charges
per-GB egress follows the same pattern — cost scales linearly with traffic
while the compute needed is negligible:

| Platform    | Egress rate | 25 TB cost (1K users) | Notes                          |
| ----------- | ----------- | --------------------- | ------------------------------ |
| Railway     | $0.10/GB    | $2,500                | Also: 100 GB/mo soft throttle  |
| AWS EC2     | $0.09/GB    | $2,188                | Tiered pricing helps at scale  |
| Fly.io      | $0.02/GB    | $500                  | Best per-GB rate               |
| Hetzner     | 20 TB incl  | $0                    | Overage at ~€1/TB              |

**Only bandwidth-inclusive providers** (Hetzner, OVH) or **flat-rate managed
services** (n0des) are cost-effective for relay workloads.

### Platform-Specific Notes

**AWS Lightsail**: Best AWS option at small scale ($12/mo for 100 users).
The 3 TB included bandwidth covers realistic traffic. Beyond that, overages
at $0.09/GB make it no better than EC2. No UDP support limitations.

**AWS EC2**: Cheapest AWS option at scale due to tiered egress pricing
($0.07/GB after 50 TB). Compute is <1% of total cost — reserved instances
and spot pricing save almost nothing. A t4g.small handles 10K users; the
bottleneck is bandwidth cost, not compute.

**AWS API Gateway + Lambda**: Per-message pricing ($1/M messages + Lambda
invocations) is even worse than CF Workers. **$17,300/mo at just 100 users.**
Completely non-viable.

**AWS App Runner**: Does not support WebSocket connections (120s request
timeout). Eliminated.

**Railway**: Per-GB egress at $0.10/GB (worse than AWS). Additional
dealbreakers: no inbound UDP (QUIC Address Discovery and STUN won't work),
100 GB/mo soft throttle triggers investigation, ~3,000 concurrent WebSocket
ceiling caps sessions at ~1,500.

**Fly.io**: Best PaaS option at $0.02/GB egress. Supports UDP, 30+ edge
regions, persistent VMs (not serverless). Reasonable at small scale but
still 30x more expensive than Hetzner at 1K+ users.

**Hetzner**: Clear winner for self-hosting. CAX11 (ARM) at ~$4/mo includes
20 TB bandwidth. Covers 100–1K users with zero marginal egress cost.
Multiple instances for 10K+ users (~$50/mo). Full UDP support, EU regions
(US locations get only 1 TB included — use EU).

### iroh Relay Options

**n0 public relays (free):**
- 4 global regions (US East, US West, EU, Asia-Pacific)
- Rate-limited (exact thresholds undocumented)
- No SLA, no uptime guarantee
- Switch: change `RelayMode::custom([url])` → `RelayMode::Default` (1 line)

**n0des Cloud ($199/mo):**
- Fully managed multi-region relays
- Uptime SLA, version locking, blue/green deployments
- Up to 10K concurrent connections per relay
- Built-in monitoring dashboards
- Priority support from the iroh team
- 30-day trial available at https://n0des.iroh.computer

**n0des Enterprise (custom pricing):**
- Single-tenant or on-premises deployment
- Dedicated support team, code reviews, security audits
- Custom SLAs and protocol development
- Contact: hello@n0.computer

**Self-hosted iroh-relay (open source):**
- `cargo install --bin iroh-relay iroh-relay` or Docker `n0computer/iroh-relay`
- Stateless binary — no database, no migrations, horizontally scalable
- Built-in ACME TLS (Let's Encrypt), Prometheus metrics on port 9090
- Ports: HTTPS (443), QUIC QAD (7842), STUN (3478)
- Each instance handles up to 10K concurrent connections

### Migration Path

The relay URL is configured in one constant:

```
crates/zedra-transport/src/lib.rs → DEFAULT_RELAY_URL
```

Both host (`iroh_listener.rs`) and client (`zedra-session/lib.rs`) read this
and pass it to `RelayMode::custom([url])`. The QR pairing payload already
carries `relay_url`, so clients learn the relay from the host at pairing time.

| Stage                   | Approach                              | Cost      |
| ----------------------- | ------------------------------------- | --------- |
| Now (dev / early users) | n0 public relays (`RelayMode::Default`) | $0        |
| First paying users      | n0des Cloud (30-day trial first)      | $199/mo   |
| Scale (1K+ users)       | n0des Cloud or Hetzner self-hosted    | $4–800/mo |

The entire `packages/relay-worker/` directory (~1,200 lines of TypeScript)
can be archived once migration is validated.

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
