# relay-monitor

Polls iroh-relay nodes every 5 minutes, sends Discord alerts on bad signals, and hourly health summaries when all is well.

## Config

All configuration is via environment variables.

| Env var | Description | Default |
|---|---|---|
| `DISCORD_WEBHOOK` | Discord webhook URL | required |
| `INSTANCES` | Comma-separated instance names (e.g. `ap1,us1,eu1`) | required |
| `MAX_CPU_PCT` | Alert CPU above this % | `80` |
| `MAX_LOAD1` | Alert 1-min load average above this | `2.0` |
| `MAX_MEM_PCT` | Alert memory above this % | `85` |
| `MAX_DISK_PCT` | Alert disk above this % | `80` |
| `POLL_MS` | Poll interval | `300000` (5 min) |
| `SUMMARY_MS` | Healthy summary interval | `3600000` (1 hr) |

SSH alias for each region must be `zedra-relay-<region>` in `~/.ssh/config`. Metrics are collected via SSH (`docker exec zedra-relay curl localhost:9090/metrics`).

## Run

```bash
# Daemon — locally, watches all instances
INSTANCES=ap1,us1,eu1 DISCORD_WEBHOOK=... bun monitor.ts

# CLI — live metrics from all instances
INSTANCES=ap1,us1,eu1 bun cli.ts
bun cli.ts ap1
bun cli.ts ap1 --history
bun cli.ts ap1 --history 6
```

## Deploy

Deployed as part of the relay stack via `deploy/relay/deploy.sh`. The deployed monitor receives `INSTANCES=${REGION}` from Compose to watch the local relay. See [`deploy/relay/README.md`](../../deploy/relay/README.md).
