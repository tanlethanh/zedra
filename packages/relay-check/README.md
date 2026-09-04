# relay-check

**Run on your machine** — SSH to relay VMs and print live metrics or history (written by the [`relay-monitor`](../relay-monitor/README.md) sidecar).

## Usage

```bash
bun cli.ts --instance sg1,us1,eu1          # live metrics, all instances
bun cli.ts --instance sg1                  # live metrics, one instance
bun cli.ts --instance sg1,us1,eu1 --history       # last 24h from metrics.jsonl
bun cli.ts --instance sg1 --history 6      # last 6h, one instance
```

Each instance must resolve as SSH host `zedra-relay-<instance>` in `~/.ssh/config`.
