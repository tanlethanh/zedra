# relay-monitor

**Docker only** — long-running poller shipped beside `zedra-relay` on each relay VM (`deploy/relay/docker-compose.yml`). Uses `INSTANCE` + `DISCORD_WEBHOOK` from the merged `.env`.

**Local SSH checks from your laptop** (multi-instance CLI) live in [`packages/relay-check`](../relay-check/README.md).

## Build

The relay stack image is built by `deploy/relay/deploy.sh` (see [`deploy/relay/README.md`](../../deploy/relay/README.md)).

```bash
docker build -f Dockerfile -t zedra-monitor:latest ../..
# context: repo root; Dockerfile copies this package only
```
