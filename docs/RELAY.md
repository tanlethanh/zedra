# Zedra Relay Architecture

Production uses self-hosted `iroh-relay` instances on EC2. Multiple regions are
supported — iroh probes all relays and picks the lowest-latency one as preferred.
If the preferred relay goes down, iroh fails over to the next best.

The relay URL list is `ZEDRA_RELAY_URLS` in `crates/zedra-rpc/src/lib.rs`.
See `deploy/relay/README.md` for deployment, cost estimates, and operations.

| Instance | Region | Hostname |
|----------|--------|----------|
| **sg1** | ap-southeast-1 (Singapore) | `sg1.relay.zedra.dev` |
| **us1** | us-east-1 (N. Virginia) | `us1.relay.zedra.dev` |
| **eu1** | eu-central-1 (Frankfurt) | `eu1.relay.zedra.dev` |

## How iroh-relay Works

iroh-relay is a stateless QUIC/WebSocket relay. When two iroh endpoints cannot
establish a direct P2P path (symmetric NAT, firewalls), traffic flows through
the relay instead. The relay never decrypts payload — it forwards opaque QUIC
datagrams between authenticated endpoints.

```
  Client A                    iroh-relay                    Client B
     │                            │                            │
     │── WS upgrade (TLS 1.3) ──▶│◀── WS upgrade (TLS 1.3) ──│
     │── ClientAuth(pubkey,sig) ─▶│◀── ClientAuth(pubkey,sig) ─│
     │◀── ServerConfirmsAuth ─────│──── ServerConfirmsAuth ───▶│
     │                            │                            │
     │── Datagram(dst=B, data) ──▶│──── Datagram(src=A, data) ▶│
     │◀── Datagram(src=B, data) ──│◀── Datagram(dst=A, data) ──│
```

**Handshake**: On WebSocket connect, the relay sends a 16-byte challenge. The
client signs it with Ed25519 (BLAKE3 KDF domain separation) and sends
`ClientAuth(pubkey, signature)`. The relay verifies and maps the pubkey to the
WebSocket — all subsequent datagrams are routed by destination pubkey.

**Relay selection**: Each iroh endpoint connects to all relays in its
`RelayMap` and reports RTT via STUN/HTTPS probes. The lowest-latency relay
becomes the `preferred_relay` published in pkarr. If it goes down, the next
best relay is promoted automatically.

**Path upgrade**: Even while relaying, iroh continues hole-punch attempts in
the background. If a direct UDP path is established, traffic migrates off the
relay seamlessly.

## Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 80 | TCP | HTTP for ACME (Let's Encrypt cert issuance) |
| 443 | TCP | HTTPS/WebSocket relay + STUN probe (`/generate_204`) |
| 7842 | UDP | QUIC address discovery (lets clients discover their public IP) |
| 9090 | TCP (localhost) | Prometheus metrics (not exposed externally) |

## Configuration (`relay.toml`)

```toml
http_bind_addr = "[::]:80"
enable_relay = true
enable_quic_addr_discovery = true
enable_metrics = true
metrics_bind_addr = "127.0.0.1:9090"

[tls]
cert_mode = "LetsEncrypt"
hostname = "__HOSTNAME__"       # substituted at container start
contact = "admin@zedra.dev"
prod_tls = true
cert_dir = "/data/certs"
```

`__HOSTNAME__` is replaced by `entrypoint.sh` with `${REGION}.relay.zedra.dev`.

## Metrics

The relay exposes Prometheus metrics on `localhost:9090/metrics`:

| Metric | Description |
|--------|-------------|
| `relay_accepts_total` | Total WebSocket connections accepted |
| `relay_disconnects_total` | Total disconnections |
| `relay_bytes_sent_total` | Bytes forwarded to clients |
| `relay_bytes_recv_total` | Bytes received from clients |
| `relay_send_packets_total` | Packets forwarded |
| `relay_send_packets_dropped_total` | Packets dropped (slow client, queue full) |
| `relay_websocket_connections` | Current active WebSocket connections |

The monitor service polls these and sends hourly summaries to Discord.

## Latency

**Vietnam → Singapore EC2:**
```
30 pings  min=112ms  avg=131ms  max=200ms
```
