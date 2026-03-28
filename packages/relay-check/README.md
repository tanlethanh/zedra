# relay-check

**Run on your machine** — SSH to relay VMs and print live metrics or history (written by the [`relay-monitor`](../relay-monitor/README.md) sidecar).

## Usage

```bash
INSTANCES=ap1,us1,eu1 bun cli.ts          # live metrics, all instances
INSTANCES=ap1,us1,eu1 bun cli.ts ap1      # live metrics, one instance
INSTANCES=ap1,us1,eu1 bun cli.ts --history       # last 24h from metrics.jsonl
INSTANCES=ap1,us1,eu1 bun cli.ts ap1 --history 6 # last 6h, one instance
```

Each instance must resolve as SSH host `zedra-relay-<instance>` in `~/.ssh/config`.
