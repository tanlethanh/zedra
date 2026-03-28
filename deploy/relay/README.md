# deploy/relay — iroh-relay multi-instance deployment

Self-hosted `iroh-relay` on cloud compute. Three instances across regions:

| Instance | Region | Hostname |
| -------- | ------ | -------- |
| **ap1** | Asia Pacific (Singapore) | `ap1.relay.zedra.dev` |
| **us1** | US (Iowa / N. Virginia) | `us1.relay.zedra.dev` |
| **eu1** | Europe (Netherlands / Frankfurt) | `eu1.relay.zedra.dev` |

## Provider Quick-Reference

Both AWS and GCP are supported. Use whichever has active credits.

| | AWS | GCP |
|---|-----|-----|
| **Instance** | t4g.small (Graviton2, ARM64) | t2a-standard-1 (Ampere Altra, ARM64) |
| **vCPU / RAM** | 2 vCPU / 2 GB | 1 vCPU / 4 GB |
| **Network** | up to 5 Gbps burst | flat 10 Gbps |
| **Per-node/mo** | ~$13 (us-east-1) | ~$23 (us-central1) |
| **ARM Docker** | native (no `--platform`) | native (no `--platform`) |
| **SSH** | PEM key + `ubuntu@<ip>` | `gcloud compute ssh` or OS Login |
| **Firewall** | Security Group | Firewall rule + network tag |
| **Static IP** | Elastic IP | Reserved address |
| **Billing stop** | Delete or stop instance (EBS still charged when stopped) | Delete or stop (disk still charged when stopped) |

Both use the same `deploy.sh`, `docker-compose.yml`, and OS setup steps.

## Free Egress Allowances

Egress is the main variable cost for the relay. Both providers include a free monthly egress allowance — understanding the limits prevents surprise bills.

### GCP — Always Free Egress

| Destination | Free/month | Rate beyond free |
| ----------- | ---------- | ---------------- |
| North America (from any region) | **1 GB** | $0.08/GB |
| Within same region | Unlimited | $0.00/GB |
| Between GCP regions | — | $0.01–0.08/GB |

1 GB free egress = ~**8 DAU** at moderate usage (0.126 GB/DAU/mo). Essentially zero headroom for a real relay — treat GCP egress as fully paid from day one.

New GCP accounts also receive **$300 credit valid for 90 days** — covers ~3,750 GB of egress, or roughly 30,000 DAU-months of moderate traffic. Burn rate at 1,000 DAU moderate: ~$10.50/mo egress → credits last ~28 months equivalent, but the 90-day wall hits first.

### AWS — Free Egress

| Tier | Free/month | Applies to |
| ---- | ---------- | ---------- |
| New accounts (first 12 months) | **15 GB** | All outbound to internet |
| Always free (all accounts) | **100 GB** | Outbound to internet (announced 2021) |
| CloudFront origin pull | Unlimited | From EC2/S3 to CloudFront |

> **100 GB always free** is the key number. This applies permanently regardless of account age.

100 GB free = ~**794 DAU** at moderate usage. A relay at ≤800 DAU pays **$0 in egress** on AWS. Beyond that, $0.09/GB.

### Egress Free Allowance in Context

| Provider | Free egress/mo | Break-even DAU (moderate) | Beyond free |
| -------- | -------------- | ------------------------- | ----------- |
| **AWS** | 100 GB | **~794 DAU** | $0.09/GB |
| **GCP** | 1 GB | ~8 DAU | $0.08/GB |

**Implication**: for a relay under ~800 DAU, AWS egress is effectively free. GCP has negligible free egress — budget for full egress cost from the start.

### Egress Cost by DAU (3 nodes combined, moderate usage)

| DAU | GB/mo total | AWS egress cost | GCP egress cost |
| --- | ----------- | --------------- | --------------- |
| 500 | 18.9 GB | **$0** (within 100 GB free) | $1.51 |
| 794 | 30 GB | **$0** (at free limit) | $2.40 |
| 1,000 | 37.8 GB | $0 (still free) | $3.02 |
| 2,000 | 75.6 GB | $0 (still free) | $6.05 |
| 3,000 | 113.4 GB | $1.21 | $9.07 |
| 5,000 | 189 GB | $8.01 | $15.12 |
| 10,000 | 378 GB | $25.02 | $30.24 |
| 25,000 | 945 GB | $76.05 | $75.60 |
| 50,000 | 1,890 GB | $161.10 | $151.20 |

> AWS and GCP converge around 25K DAU — AWS's 100 GB free advantage narrows as volume grows.

---

## Architecture

- **Instance**: ARM64 (AWS t4g / GCP t2a) — Ubuntu 24.04, aarch64
- **Runtime**: Docker Compose (`zedra-relay` + `zedra-monitor`) from locally-built images
- **Ports**: 80 (HTTP/ACME), 443 (HTTPS/WebSocket relay), 7842/udp (QUIC addr discovery)
- **TLS**: Let's Encrypt via iroh-relay built-in ACME, certs in Docker volume `zedra-relay-certs`
- **Image build**: Multi-stage (Rust builder → Debian slim), built natively on Apple Silicon (arm64 = arm64, no cross-compilation), streamed to server via `docker save | gzip | ssh`

## Directory Structure

```
deploy/relay/
  Dockerfile          # relay image definition
  docker-compose.yml  # relay + monitor services
  relay.toml          # iroh-relay config template (__HOSTNAME__ substituted at runtime)
  entrypoint.sh       # injects RELAY_HOSTNAME into relay.toml at container start
  deploy.sh           # build + stream images + bring up compose
  .env.example        # template → copy to .env (secrets, gitignored)

packages/relay-check/ # local-only: SSH health daemon + CLI (`INSTANCES=...`) — not in the deploy bundle
```

## Deploy

### Prerequisites

Add SSH aliases to `~/.ssh/config` for each instance (adjust `HostName` and `IdentityFile` per provider):

```
Host zedra-relay-ap1
  HostName <AP1_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/<your-key>    # AWS: .pem file; GCP: google_compute_engine

Host zedra-relay-us1
  HostName <US1_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/<your-key>

Host zedra-relay-eu1
  HostName <EU1_PUBLIC_IP>
  User ubuntu
  IdentityFile ~/.ssh/<your-key>
```

> **GCP alternative**: `gcloud compute ssh INSTANCE_NAME --zone=ZONE` manages keys automatically — no `~/.ssh/config` entry needed.

**Secrets (local):** copy `deploy/relay/.env.example` to `deploy/relay/.env` and set at least `DISCORD_WEBHOOK`. The root `.gitignore` ignores `.env` everywhere.

> **How `.env` works:** `deploy.sh` merges your local `deploy/relay/.env` with injected `INSTANCE=<name>` (from `--instance`), uploads to `/opt/zedra/deploy/relay/.env.local`, then copies to `.env` for Compose. `INSTANCE=` / `INSTANCES=` lines in your local file are ignored. The relay uses `INSTANCE` for hostname (`${INSTANCE}.relay.zedra.dev`). The **Docker** `relay-monitor` sidecar uses **`INSTANCE` only**. **Multi-host SSH checks from your laptop** use **`packages/relay-check`** (`INSTANCES=ap1,us1,eu1 bun monitor.ts` or `bun cli.ts`).

### Deploy one instance

```bash
./deploy/relay/deploy.sh --instance ap1
```

### Deploy all instances in parallel

```bash
./deploy/relay/deploy.sh --instance ap1,us1,eu1
```

### How deploy.sh works

1. Builds `zedra-relay:latest` and `zedra-monitor:latest` locally (native arm64 on Apple Silicon — no `--platform` flag needed)
2. `docker save | gzip | ssh <host> docker load` — streams both images to the server without a registry
3. Uploads `docker-compose.yml`, merges local `deploy/relay/.env` with injected `INSTANCE` to `.env.local` and `.env` on the host, runs `docker compose up -d`

---

## Instance Setup — GCP

### Provision instances

```bash
# ap1 — Singapore
gcloud compute instances create zedra-relay-ap1 \
  --zone=asia-southeast1-b \
  --machine-type=t2a-standard-1 \
  --image-family=ubuntu-2404-lts-arm64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=10GB \
  --network-tier=PREMIUM \
  --tags=zedra-relay

# us1 — Iowa
gcloud compute instances create zedra-relay-us1 \
  --zone=us-central1-a \
  --machine-type=t2a-standard-1 \
  --image-family=ubuntu-2404-lts-arm64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=10GB \
  --network-tier=PREMIUM \
  --tags=zedra-relay

# eu1 — Netherlands
gcloud compute instances create zedra-relay-eu1 \
  --zone=europe-west4-a \
  --machine-type=t2a-standard-1 \
  --image-family=ubuntu-2404-lts-arm64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=10GB \
  --network-tier=PREMIUM \
  --tags=zedra-relay
```

### Firewall rules (one-time per project)

```bash
gcloud compute firewall-rules create zedra-relay-allow \
  --target-tags=zedra-relay \
  --allow=tcp:80,tcp:443,udp:7842 \
  --description="iroh-relay: ACME, WebSocket relay, QUIC addr discovery"
```

SSH (TCP 22) is already allowed by the default `default-allow-ssh` rule.

### Reserve static IPs

```bash
gcloud compute addresses create zedra-relay-ap1-ip --region=asia-southeast1
gcloud compute addresses create zedra-relay-us1-ip --region=us-central1
gcloud compute addresses create zedra-relay-eu1-ip --region=europe-west4
```

---

## Instance Setup — AWS

### Provision instances

```bash
# ap1 — Singapore (ap-southeast-1)
aws ec2 run-instances \
  --region ap-southeast-1 \
  --image-id ami-0c1907b6d738188e5 \   # Ubuntu 24.04 arm64 — verify current AMI
  --instance-type t4g.small \
  --key-name zedra-relay-ap1 \
  --security-group-ids <SG_ID> \
  --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=zedra-relay-ap1}]'

# us1 — N. Virginia (us-east-1)
aws ec2 run-instances \
  --region us-east-1 \
  --image-id ami-0a7a4e87939439934 \   # Ubuntu 24.04 arm64 — verify current AMI
  --instance-type t4g.small \
  --key-name zedra-relay-us1 \
  --security-group-ids <SG_ID> \
  --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=zedra-relay-us1}]'

# eu1 — Frankfurt (eu-central-1)
aws ec2 run-instances \
  --region eu-central-1 \
  --image-id ami-01e444924a2233b07 \   # Ubuntu 24.04 arm64 — verify current AMI
  --instance-type t4g.small \
  --key-name zedra-relay-eu1 \
  --security-group-ids <SG_ID> \
  --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=zedra-relay-eu1}]'
```

> **AMI IDs change per region and over time.** Find the current Ubuntu 24.04 arm64 AMI:
> `aws ec2 describe-images --owners 099720109477 --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-arm64-server-*" --query 'sort_by(Images,&CreationDate)[-1].ImageId' --output text --region <REGION>`

### Security group inbound rules (per region)

```
TCP 22    — SSH (your IP only)
TCP 80    — HTTP / ACME
TCP 443   — HTTPS / WebSocket relay
UDP 7842  — QUIC addr discovery
```

### Elastic IPs (static)

```bash
aws ec2 allocate-address --region ap-southeast-1
aws ec2 allocate-address --region us-east-1
aws ec2 allocate-address --region eu-central-1
# Then associate each with its instance
aws ec2 associate-address --region <REGION> --instance-id <ID> --allocation-id <ALLOC_ID>
```

### Stopping vs deleting on AWS

- **Stop**: instance compute is free; EBS disk (~$0.08/GB/mo) and Elastic IP (~$7.20/mo if unattached) are still charged.
- **Terminate (delete)**: all charges stop. Release Elastic IPs separately.

---

## Common OS Setup (both providers)

Run on each instance after first SSH in.

### Docker

```bash
sudo apt-get update
sudo apt-get install -y docker.io
sudo systemctl enable --now docker
sudo usermod -aG docker ubuntu
```

### File descriptor limits

1 connection = 1 fd; default 1024 is not enough:

```bash
sudo tee /etc/security/limits.d/99-zedra.conf > /dev/null << 'EOF'
* soft nofile 100000
* hard nofile 100000
EOF
```

### TCP tuning

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

### Docker daemon

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

`live-restore: true` keeps containers running across Docker daemon restarts.

---

## DNS

Point each hostname to its public IP (A record, TTL 60):

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

---

## Budget Guardrails

Set these up **before** deploying. The relay's egress cost scales linearly with traffic — a billing alert catches runaway costs early.

### GCP — Budget Alert

```bash
# Via console: Billing → Budgets & Alerts → Create Budget
# Or via CLI:
gcloud billing budgets create \
  --billing-account=BILLING_ACCOUNT_ID \
  --display-name="zedra-relay monthly" \
  --budget-amount=50USD \
  --threshold-rule=percent=50,basis=CURRENT_SPEND \
  --threshold-rule=percent=80,basis=CURRENT_SPEND \
  --threshold-rule=percent=100,basis=CURRENT_SPEND
```

Find your billing account ID: `gcloud billing accounts list`

GCP does **not** auto-stop instances at budget — alerts are notification-only. To auto-stop, add a Pub/Sub notification + Cloud Function trigger.

### AWS — Budget Alert

```bash
# Via console: Billing → Budgets → Create Budget → Cost Budget
# Set monthly budget, alert at 80% actual + 100% forecasted
```

Or via CLI:
```bash
aws budgets create-budget \
  --account-id $(aws sts get-caller-identity --query Account --output text) \
  --budget '{
    "BudgetName": "zedra-relay-monthly",
    "BudgetLimit": {"Amount": "50", "Unit": "USD"},
    "TimeUnit": "MONTHLY",
    "BudgetType": "COST"
  }' \
  --notifications-with-subscribers '[
    {
      "Notification": {
        "NotificationType": "ACTUAL",
        "ComparisonOperator": "GREATER_THAN",
        "Threshold": 80,
        "ThresholdType": "PERCENTAGE"
      },
      "Subscribers": [{"SubscriptionType": "EMAIL", "Address": "you@example.com"}]
    }
  ]'
```

AWS also does **not** auto-stop instances at budget — alerts are notification-only by default.

### Cost Watchpoints

The relay has two cost drivers: **compute** (fixed) and **egress** (variable). Monitor both.

| Signal | Action |
| ------ | ------ |
| Monthly egress > $30 on a single node | Check if traffic is legitimate; consider upgrading instance |
| CPUCreditBalance (AWS t4g) < 20 | Instance CPU is sustained above baseline — upgrade to t4g.medium |
| e2-micro CPU > 80% sustained (GCP) | Upgrade to t2a-standard-1 |
| Budget alert at 80% before mid-month | Investigate unexpected egress spike |
| Budget alert at 100% | Stop non-critical nodes immediately; review traffic |

### Stay Within Free Egress (GCP)

- GCP free egress is only **1 GB/month** to North America — treat it as zero; budget for full egress costs
- Monitor: GCP Console → Billing → Reports → filter by SKU `Network Internet Egress`
- Set a separate sub-budget for egress only: `gcloud billing budgets create` with label filter on network SKUs
- Use `$300` new-account credits aggressively for first 90 days; set a hard reminder to review spend at day 60

### Stay Within Free Egress (AWS)

- **100 GB/month free egress is always-free** — no expiry, no account age requirement
- At ≤800 DAU moderate usage, egress is $0. Confirm you haven't crossed the threshold:
  ```bash
  # AWS Cost Explorer CLI — egress spend this month
  aws ce get-cost-and-usage \
    --time-period Start=$(date +%Y-%m-01),End=$(date +%Y-%m-%d) \
    --granularity MONTHLY \
    --filter '{"Dimensions":{"Key":"USAGE_TYPE_GROUP","Values":["EC2: Data Transfer - Internet (Out)"]}}' \
    --metrics BlendedCost
  ```
- Enable Free Tier Usage Alerts: AWS Console → Billing → Billing Preferences → Free Tier Usage Alerts

---

## Production Checklist

Run through this before and after every first-time deployment or infrastructure change.

### Pre-deploy

- **Billing alert configured** (GCP Budget or AWS Budget) with threshold at 80% + 100% of monthly target
- **Free tier check**: if using free tier, confirm instance count and region comply (1 node max on AWS free tier; GCP e2-micro in us-central1/us-east1/us-west1 only)
- SSH aliases configured in `~/.ssh/config` for all target instances
- Local `deploy/relay/.env` present with `DISCORD_WEBHOOK` set (deploy pushes merged env to each instance)
- DNS A records pointing each hostname to its public IP (TTL ≤ 60)
- Firewall/security group open: TCP 80, TCP 443, UDP 7842
- OS setup complete: Docker installed, sysctl tuned, fd limits raised, Docker daemon configured, ubuntu in docker group
- Verify sysctl applied on each instance:
  ```bash
  ssh zedra-relay-ap1 "sysctl net.core.somaxconn vm.swappiness fs.file-max"
  # expect: 4096 / 10 / 200000
  ```
- Verify Docker daemon config applied (`live-restore`, `default-ulimits`):
  ```bash
  ssh zedra-relay-ap1 "docker info | grep -E 'Live Restore|logging'"
  ```
- Docker running locally and `docker info` succeeds
- Outbound port 443 reachable from instance (needed for Let's Encrypt ACME challenge)

### Post-deploy

- `generate_204` returns HTTP 204 on all instances:
  ```bash
  curl -I https://ap1.relay.zedra.dev/generate_204
  curl -I https://us1.relay.zedra.dev/generate_204
  curl -I https://eu1.relay.zedra.dev/generate_204
  ```
- TLS certificate issued (first deploy only — ACME may take up to 60s):
  ```bash
  curl -vI https://ap1.relay.zedra.dev/generate_204 2>&1 | grep -E "subject:|issuer:|expire"
  ```
- Both containers healthy and `init` process is PID 1:
  ```bash
  ssh zedra-relay-ap1 "docker compose -f /opt/zedra/deploy/relay/docker-compose.yml ps"
  ssh zedra-relay-ap1 "docker exec zedra-relay-relay-1 cat /proc/1/comm"  # expect: tini
  ```
- Container fd limit is raised (not default 1024):
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay-relay-1 sh -c 'ulimit -n'"
  # expect: 100000
  ```
- Relay logs clean (no errors, no panics):
  ```bash
  ssh zedra-relay-ap1 "docker logs zedra-relay --tail=50"
  ```
- Monitor sending Discord heartbeat (check Discord channel for hourly summary)
- Metrics endpoint reachable from inside the container:
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay curl -sf http://localhost:9090/metrics | head -5"
  ```
- Cert volume persisting (not empty):
  ```bash
  ssh zedra-relay-ap1 "docker volume inspect zedra-relay-certs"
  ```

### Ongoing health

- Monitor Discord alerts are firing (test by temporarily lowering a threshold in local `deploy/relay/.env`, then redeploy)
- Logrotate configured for `/var/log/zedra-relay/metrics.jsonl` (done by `deploy.sh`)
- Cert auto-renewal working — Let's Encrypt renews ~30 days before expiry; confirm after first month:
  ```bash
  ssh zedra-relay-ap1 "docker exec zedra-relay ls -la /data/certs/"
  ```
- Review instance metrics after 24h of traffic:
  - **GCP**: Console → Compute Engine → instance → Monitoring
  - **AWS**: Console → EC2 → instance → Monitoring → CloudWatch (check CPUCreditBalance for t4g)

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

## Cost Estimate

iroh-relay is stateless and lightweight — it only relays when direct P2P hole-punching fails.
CPU and memory usage are minimal; bandwidth is the main variable cost.
Both AWS and GCP bill **per second** (1-minute minimum) — charges stop when instance is stopped/deleted.

### Traffic assumptions

- **70% relay rate** — Symmetric NAT is prevalent on mobile/corporate networks.
- Traffic split: ap1 40%, us1 35%, eu1 25% (APAC-weighted user base).
- Blended egress rate: ~$0.09/GB (AWS) · ~$0.08/GB (GCP).

### Per-DAU monthly traffic model

| Usage pattern | Session/day | Terminal I/O | Raw/session | × 70% relay | × 30 days | GB/DAU/mo |
| ------------- | ----------- | ------------ | ----------- | ----------- | --------- | --------- |
| Light — file browse, quick commands | 1 hr | 1 MB/hr | 1 MB | 0.7 MB | ×30 | **0.021 GB** |
| Moderate — active coding, terminal | 2 hr | 3 MB/hr | 6 MB | 4.2 MB | ×30 | **0.126 GB** |
| Heavy — log streaming, large builds | 3 hr | 10 MB/hr | 30 MB | 21 MB | ×30 | **0.630 GB** |

### Instance sizing by DAU

**Memory model per node** (from iroh-relay source, v0.96):

| Component | Size |
| --------- | ---- |
| Process baseline | ~50 MB |
| Key cache (1M endpoint IDs × 32 B, fixed) | ~32 MB |
| Per idle WebSocket connection: TLS buffers + tokio task + 2× MPSC channel | **~40–50 KB** |

Fixed overhead: **~82 MB**. Packets dropped (not buffered) when send queue full — memory is bounded.
Dead connections cleaned up by ping every 15s / pong timeout 5s.

**OS file descriptor limit:** 1 connection = 1 fd. Linux default is 1024 — must be raised. See OS setup above.

Peak concurrent connections = DAU × 15% online × 70% relay = **DAU × 0.105**

| DAU | Peak concurrent | RAM needed | Instance (AWS) | Instance (GCP) | Per-node/mo (AWS) | Per-node/mo (GCP) |
| --- | --------------- | ---------- | -------------- | -------------- | ----------------- | ----------------- |
| ≤120,000 | ≤12,600 | ≤714 MB | **t4g.small** (2 GB) | **t2a-standard-1** (4 GB) | ~$13 | ~$24 |
| 120K–500K | ≤52,500 | ≤2,707 MB | **t4g.large** (8 GB) | **t2a-standard-2** (8 GB) | ~$52 | ~$48 |

> **Network note**: t4g instances use a burst credit model (baseline ~16 MB/s, burst to 5 Gbps). T2A has a flat 10 Gbps NIC with no throttling. For sustained high-throughput workloads, T2A has the advantage.

### 3-node total by DAU (moderate usage, on-demand)

| DAU | AWS fixed (t4g.small×3) | GCP fixed (t2a-standard-1×3) | Data/mo | AWS total | GCP total |
| --- | ----------------------- | ---------------------------- | ------- | --------- | --------- |
| 100 | $39 | $74 | $1.05 | **~$40** | **~$75** |
| 1,000 | $39 | $74 | $10.50 | **~$50** | **~$85** |
| 5,000 | $39 | $74 | $52.50 | **~$92** | **~$127** |
| 10,000 | $39 | $74 | $105 | **~$144** | **~$179** |
| 25,000 | $39 | $74 | $263 | **~$302** | **~$337** |
| 50,000 | $39 | $74 | $525 | **~$564** | **~$599** |
| 100,000 | $39 | $74 | $1,050 | **~$1,089** | **~$1,124** |

### Savings: Reserved (AWS) vs Committed Use (GCP)

**AWS 1yr No-Upfront Reserved (~40% off compute)**:

| Instance | On-demand/mo | Reserved/mo |
| -------- | ------------ | ----------- |
| t4g.small us-east-1 | $12.26 | $7.36 |
| t4g.small eu-central-1 | $13.43 | $8.06 |
| t4g.small ap-southeast-1 | $13.43 | $8.06 |

Reserved (t4g.small × 3): **~$23.48/mo** · With 1,000 DAU moderate: **~$34/mo total**

**GCP 1yr Committed Use Discount (~37% off compute)**:

| Instance | On-demand/mo | 1yr CUD/mo |
| -------- | ------------ | ---------- |
| t2a-standard-1 us-central1 | ~$22.70 | ~$14.26 |
| t2a-standard-1 europe-west4 | ~$24.82 | ~$15.59 |
| t2a-standard-1 asia-southeast1 | ~$26.28 | ~$16.51 |

CUD (t2a-standard-1 × 3): **~$46.36/mo** · With 1,000 DAU moderate: **~$57/mo total**

### When to migrate off cloud

At scale, flat-rate bandwidth providers are dramatically cheaper:

| DAU | AWS (moderate) | GCP (moderate) | Fly.io | Hetzner + Vultr AP |
| --- | -------------- | -------------- | ------ | ------------------ |
| 25,000 | ~$302/mo | ~$337/mo | ~$77/mo | ~$30/mo |
| 50,000 | ~$564/mo | ~$599/mo | ~$150/mo | ~$35/mo |
| 100,000 | ~$1,089/mo | ~$1,124/mo | ~$250/mo | ~$60/mo |
| 200,000 | ~$2,200/mo | ~$2,248/mo | ~$500/mo | ~$80/mo |

```
≤100K DAU   →  t4g.small (AWS) or t2a-standard-1 (GCP)   stay
100–200K    →  upgrade instance tier                       consider Fly.io
200K+ DAU   →  Hetzner (EU/US) + Vultr (AP)               10–20× cheaper
```

Hetzner CAX11 (ARM, 20 TB/mo included): ~€3.29/mo — no Singapore region.
Pair with Vultr Singapore (~$6/mo, 4 TB included) for APAC coverage.

### Notes

- AWS APAC egress ($0.12/GB) costs ~33% more than US/EU ($0.09/GB).
- GCP egress is $0.08/GB from all three relay regions (first 1 TB/mo).
- Inbound data transfer is always free on both providers.
- T2A (GCP) is ARM64 only — available in `us-central1`, `europe-west4`, `asia-southeast1`.
- t4g (AWS) is ARM64 (Graviton2) — available in all major AWS regions.
- Apple Silicon Mac → both ARM instances: Docker images build and run natively without `--platform`.
