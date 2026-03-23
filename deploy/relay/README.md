# deploy/relay — iroh-relay multi-instance deployment

Self-hosted `iroh-relay` on AWS EC2. Three instances:

| Instance | AWS | Hostname |
|----------|-----|----------|
| **ap1** | ap-southeast-1 (Singapore) | `ap1.relay.zedra.dev` |
| **us1** | us-east-1 (N. Virginia) | `us1.relay.zedra.dev` |
| **eu1** | eu-central-1 (Frankfurt) | `eu1.relay.zedra.dev` |

## Architecture

- **Instance**: EC2 t4g.small ARM64 (Ubuntu 24.04) — Graviton2, aarch64
- **Runtime**: Docker Compose (`zedra-relay` + `zedra-monitor`) from locally-built images
- **Ports**: 80 (HTTP/ACME), 443 (HTTPS/WebSocket relay), 7842/udp (QUIC addr discovery)
- **TLS**: Let's Encrypt via iroh-relay built-in ACME, certs in Docker volume `zedra-relay-certs`
- **Image build**: Multi-stage (Rust builder → Debian slim), cross-compiled locally, streamed to EC2 via `docker save | gzip | ssh`

## Directory Structure

```
deploy/relay/
  Dockerfile          # relay image definition
  docker-compose.yml  # relay + monitor services
  relay.toml          # iroh-relay config template (__HOSTNAME__ substituted at runtime)
  entrypoint.sh       # injects RELAY_HOSTNAME into relay.toml at container start
  deploy.sh           # build + stream images + bring up compose
  .env.example        # env var reference for .env.local on each EC2 instance
```

## Deploy

### Prerequisites

Add SSH aliases to `~/.ssh/config` for each instance:

```
Host zedra-relay-ap1
  HostName <AP1_EC2_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/zedra-relay-ap1.pem

Host zedra-relay-us1
  HostName <US1_EC2_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/zedra-relay-us1.pem

Host zedra-relay-eu1
  HostName <EU1_EC2_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/zedra-relay-eu1.pem
```

Create `/opt/zedra/deploy/relay/.env.local` on each EC2 instance (see `.env.example`):

```bash
DISCORD_WEBHOOK=https://discord.com/api/webhooks/YOUR_ID/YOUR_TOKEN
```

> **How `.env` works:** `deploy.sh` generates `.env` by prepending `REGION=<instance>` (from the `--instance` flag) to `.env.local`. The relay uses `REGION` for its hostname (`${REGION}.relay.zedra.dev`). The monitor receives `INSTANCES=${REGION}` via Compose to watch the local relay. The CLI (`bun cli.ts`) is run locally with `INSTANCES=ap1,us1,eu1` to fetch from all nodes.

### Deploy one instance

```bash
./deploy/relay/deploy.sh --instance ap1
```

### Deploy all instances in parallel

```bash
./deploy/relay/deploy.sh --instance ap1,us1,eu1
```

### How deploy.sh works

1. Builds `zedra-relay:latest` and `zedra-monitor:latest` locally
2. `docker save | gzip | ssh <host> docker load` — streams both images to EC2 without a registry
3. Uploads `docker-compose.yml`, writes `.env` from `.env.local`, runs `docker compose up -d`

## EC2 Setup (first time per instance)

```bash
# Install Docker
sudo apt-get update
sudo apt-get install -y docker.io
sudo systemctl enable --now docker
sudo usermod -aG docker ubuntu
```

**File descriptor limits** — 1 connection = 1 fd; default 1024 is not enough:

```bash
sudo tee /etc/security/limits.d/99-zedra.conf > /dev/null << 'EOF'
* soft nofile 100000
* hard nofile 100000
EOF
```

**TCP tuning** — accept queue and connection handling for high concurrency:

```bash
sudo tee /etc/sysctl.d/99-zedra.conf > /dev/null << 'EOF'
# File descriptor ceiling
fs.file-max = 200000

# Accept queue — default 128 drops connections under load
net.core.somaxconn = 4096
net.ipv4.tcp_max_syn_backlog = 4096
net.core.netdev_max_backlog = 4096

# Reduce swap usage (relay is memory-sensitive; prefer OOM over swapping)
vm.swappiness = 10
EOF
sudo sysctl --system
```

**Docker daemon** — production logging defaults and live-restore:

```bash
sudo tee /etc/docker/daemon.json > /dev/null << 'EOF'
{
  "log-driver": "json-file",
  "log-opts": { "max-size": "50m", "max-file": "3" },
  "default-ulimits": { "nofile": { "Name": "nofile", "Hard": 100000, "Soft": 100000 } },
  "live-restore": true
}
EOF
sudo systemctl restart docker
```

`live-restore: true` keeps containers running across Docker daemon restarts (e.g. during `apt upgrade docker.io`).

Security group inbound rules:
- TCP 22 (SSH, your IP only)
- TCP 80 (ACME / HTTP)
- TCP 443 (HTTPS / WebSocket relay)
- UDP 7842 (QUIC addr discovery)

## DNS

Point each hostname to its EC2 public IP (A record, TTL 60):

```
ap1.relay.zedra.dev  →  <AP1_IP>
us1.relay.zedra.dev  →  <US1_IP>
eu1.relay.zedra.dev  →  <EU1_IP>
```

## iroh-relay Version

Pinned to iroh git commit `82e0695` (post-v0.96.1, includes TCP_NODELAY fix from PR #3995).
Once iroh v0.98 ships on crates.io, update the `Dockerfile` builder stage to:

```dockerfile
cargo install iroh-relay --version 0.98 --features server --locked
```

## Production Checklist

Run through this before and after every first-time deployment or infrastructure change.

### Pre-deploy

- [ ] SSH aliases configured in `~/.ssh/config` for all target instances (`zedra-relay-ap1`, etc.)
- [ ] `.env.local` present on each EC2 instance with `DISCORD_WEBHOOK` set
- [ ] DNS A records pointing each hostname to its EC2 public IP (TTL ≤ 60)
- [ ] Security group inbound: TCP 22 (your IP only), TCP 80, TCP 443, UDP 7842
- [ ] EC2 setup complete: Docker installed, sysctl tuned, fd limits raised, Docker daemon configured, ubuntu in docker group
- [ ] Verify sysctl applied on each instance:
  ```bash
  ssh zedra-relay-ap1 "sysctl net.core.somaxconn vm.swappiness fs.file-max"
  # expect: 4096 / 10 / 200000
  ```
- [ ] Verify Docker daemon config applied (`live-restore`, `default-ulimits`):
  ```bash
  ssh zedra-relay-ap1 "docker info | grep -E 'Live Restore|logging'"
  ```
- [ ] Docker running locally and `docker info` succeeds
- [ ] Outbound port 443 reachable from EC2 (needed for Let's Encrypt ACME challenge)

### Post-deploy

- [ ] `generate_204` returns HTTP 204 on all instances:
  ```bash
  curl -I https://ap1.relay.zedra.dev/generate_204
  curl -I https://us1.relay.zedra.dev/generate_204
  curl -I https://eu1.relay.zedra.dev/generate_204
  ```
- [ ] TLS certificate issued (first deploy only — ACME may take up to 60s):
  ```bash
  curl -vI https://ap1.relay.zedra.dev/generate_204 2>&1 | grep -E "subject:|issuer:|expire"
  ```
- [ ] Both containers healthy and `init` process is PID 1:
  ```bash
  ssh zedra-relay-ap1 "docker compose -f /opt/zedra/deploy/relay/docker-compose.yml ps"
  ssh zedra-relay-ap1 "docker exec zedra-relay-relay-1 cat /proc/1/comm"  # expect: tini
  ```
- [ ] Container fd limit is raised (not default 1024):
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay-relay-1 sh -c 'ulimit -n'"
  # expect: 100000
  ```
- [ ] Relay logs clean (no errors, no panics):
  ```bash
  ssh zedra-relay-ap1 "docker logs zedra-relay --tail=50"
  ```
- [ ] Monitor sending Discord heartbeat (check Discord channel for hourly summary)
- [ ] Metrics endpoint reachable from inside the container:
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay curl -sf http://localhost:9090/metrics | head -5"
  ```
- [ ] Cert volume persisting (not empty):
  ```bash
  ssh zedra-relay-ap1 "docker volume inspect zedra-relay-certs"
  ```

### Ongoing health

- [ ] Monitor Discord alerts are firing (test by temporarily lowering a threshold in `.env.local`)
- [ ] Logrotate configured for `/var/log/zedra-relay/metrics.jsonl` (done by `deploy.sh`)
- [ ] Cert auto-renewal working — Let's Encrypt renews ~30 days before expiry; confirm after first month:
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay ls -la /data/certs/"
  ```
- [ ] Review instance metrics after 24h of traffic — check CPU credits aren't draining:
  ```bash
  # In AWS Console: EC2 → instance → Monitoring → CPUCreditBalance
  # Network: check CloudWatch NetworkIn + NetworkOut vs baseline
  ```

## Verify

```bash
curl https://ap1.relay.zedra.dev/generate_204   # 204
curl https://us1.relay.zedra.dev/generate_204   # 204
curl https://eu1.relay.zedra.dev/generate_204   # 204
```

## Logs

```bash
ssh zedra-relay-ap1 "docker logs -f zedra-relay"
ssh zedra-relay-us1 "docker logs -f zedra-relay"
ssh zedra-relay-eu1 "docker logs -f zedra-relay"
```

---

## AWS Cost Estimate

iroh-relay is stateless and lightweight — it only relays when direct P2P hole-punching fails.
CPU and memory usage are minimal; bandwidth is the main variable cost.

### Traffic assumptions

- **70% relay rate** — Symmetric NAT is prevalent on mobile/corporate networks; expect most
  connections to require relay rather than direct P2P.
- Traffic split across nodes: ap1 40%, us1 35%, eu1 25% (APAC-weighted user base).
- Blended egress rate across 3 nodes: ~$0.103/GB.

### Per-DAU monthly traffic model

Each relayed session carries 100% of that connection's traffic through the relay.

| Usage pattern | Session/day | Terminal I/O | Raw/session | × 70% relay | × 30 days | GB/DAU/mo |
|---------------|-------------|-------------|-------------|-------------|-----------|-----------|
| Light — file browse, quick commands | 1 hr | 1 MB/hr | 1 MB | 0.7 MB | ×30 | **0.021 GB** |
| Moderate — active coding, terminal | 2 hr | 3 MB/hr | 6 MB | 4.2 MB | ×30 | **0.126 GB** |
| Heavy — log streaming, large builds | 3 hr | 10 MB/hr | 30 MB | 21 MB | ×30 | **0.630 GB** |

### Instance sizing by DAU

**Memory model per node** (from iroh-relay source, v0.96):

| Component | Size |
|-----------|------|
| Process baseline | ~50 MB |
| Key cache (1M endpoint IDs × 32 B, fixed) | ~32 MB |
| Per idle WebSocket connection: TLS buffers (16 KB read + 16 KB write) + tokio task + 2× MPSC channel | **~40–50 KB** |

Fixed overhead: **~82 MB**. Per backed-up send queue: up to 512 packets × packet size (terminal I/O ~1–2 KB each, capped at 64 KB max) — packets are **dropped** when queue is full, never blocked, so memory is bounded. Slow clients are disconnected after `SERVER_WRITE_TIMEOUT = 2s`. Dead connections cleaned up by ping every 15s / pong timeout 5s.

**OS file descriptor limit:** 1 connection = 1 TCP socket = 1 fd. Linux default (`ulimit -n`) is 1024 — must be raised for production. See EC2 setup section.

Peak concurrent connections = DAU × 15% online × 70% relay = **DAU × 0.105**

| DAU | Peak concurrent | RAM needed | Net avg (NIC total) | Instance | Net baseline | Per-node/mo |
|-----|----------------|------------|---------------------|----------|-------------|-------------|
| ≤120,000 | ≤12,600 | ≤714 MB | ≤12 MB/s | **t4g.small** (2 GB) | **16 MB/s** | ap1 $13.43 · us1 $12.26 · eu1 $13.43 |
| 120,000–500,000 | ≤52,500 | ≤2,707 MB | ≤50 MB/s | **t4g.large** (8 GB) | **64 MB/s** | ap1 $53.73 · us1 $49.06 · eu1 $53.73 |

Net avg is NIC total (egress + ingress ≈ 2× user egress rate). **Network baseline is the tighter ceiling** — t4g.small RAM can hold ~32K concurrent connections but sustained NIC load hits 16 MB/s around 120K DAU. Upgrade to **t4g.large** (64 MB/s baseline, same aarch64 build) before that point.

### Network bursting explained

All t4g instances use a **network I/O credit model** with separate inbound and outbound buckets:

- Instance **launches with credits full**
- Credits **accumulate** whenever traffic is below baseline; **drain** when above
- Credits fund bursting up to 5 Gbps for a limited window — typically **5 to 60 minutes** before hard throttle back to baseline
- **Best-effort only**: burst is not guaranteed even with credits — the burst pool is shared across the physical host
- **No unlimited mode for network**: T Unlimited only applies to CPU credits, not network I/O — there is no equivalent override for network throttling
- **Dedicated bandwidth** only on instances with >16 vCPUs (8xlarge+) — not practical for this workload

**Why this is fine for relay traffic:** Terminal I/O is inherently bursty — connection setup, initial screen render, and large pastes or build output are short spikes lasting seconds. The relay earns network credits during the long idle stretches between activity. At 50K DAU the average NIC load is ~5 MB/s (31% of the 16 MB/s baseline), so t4g.small accumulates credits continuously and handles all realistic traffic peaks well within the burst window.

### 3-node total by DAU (moderate usage, on-demand)

Fixed base (t4g.small × 3): **$39.12/mo** · Fixed base (t4g.large × 3): **$156.52/mo**

| DAU | Instance | 3-node fixed | Data/mo | **3-node total** |
|-----|----------|-------------|---------|------------------|
| 100 | t4g.small | $39.12 | $1.30 | **~$40** |
| 500 | t4g.small | $39.12 | $6.49 | **~$46** |
| 1,000 | t4g.small | $39.12 | $12.98 | **~$52** |
| 5,000 | t4g.small | $39.12 | $64.90 | **~$104** |
| 10,000 | t4g.small | $39.12 | $129.80 | **~$169** |
| 25,000 | t4g.small | $39.12 | $324.50 | **~$364** |
| 50,000 | t4g.small | $39.12 | $649.00 | **~$688** |
| 100,000 | t4g.small | $39.12 | $1,298.00 | **~$1,337** |
| 200,000 | t4g.large | $156.52 | $2,596.00 | **~$2,753** |

### When to migrate off AWS

At scale, flat-rate bandwidth providers are dramatically cheaper:

| DAU | AWS (moderate) | Fly.io | Hetzner + Vultr AP |
|-----|---------------|--------|-------------------|
| 25,000 | ~$364/mo | ~$77/mo | ~$30/mo |
| 50,000 | ~$688/mo | ~$150/mo | ~$35/mo |
| 100,000 | ~$1,337/mo | ~$250/mo | ~$60/mo |
| 200,000 | ~$2,753/mo | ~$500/mo | ~$80/mo |

```
≤100,000 DAU  →  t4g.small on AWS          stay
100–200K DAU  →  t4g.large on AWS          consider Fly.io
200K+ DAU     →  Hetzner (EU/US) + Vultr   10–20× cheaper than AWS
```

Hetzner CAX11 (ARM, 20 TB/mo included): ~€3.29/mo — no Singapore region.
Pair with Vultr Singapore (~$6/mo, 4 TB included) for APAC coverage.

### 1-year Reserved Instance savings (~40% on compute)

| Instance | On-demand/mo | 1yr no-upfront/mo |
|----------|-------------|-------------------|
| t4g.small us-east-1 | $12.26 | $7.36 |
| t4g.small eu-central-1 | $13.43 | $8.06 |
| t4g.small ap-southeast-1 | $13.43 | $8.06 |

Reserved compute (t4g.small × 3): **$23.48/mo**

With 1yr reserved + 1,000 DAU moderate traffic: **~$36/mo total** ($23.48 compute + $12.98 data).

### Notes

- Data transfer between AWS regions (inter-region) is not needed — each relay node is independent.
- Inbound data transfer is always free.
- APAC egress ($0.12/GB) costs ~33% more than US/EU ($0.09/GB).
- t4g.nano (not used here) qualifies for the AWS Free Tier (750 hrs/mo for first 12 months).
