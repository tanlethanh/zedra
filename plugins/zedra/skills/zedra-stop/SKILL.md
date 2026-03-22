---
name: zedra-stop
description: Stop the running Zedra Host daemon. Use when the user wants to shut down zedra, stop the daemon, or disconnect.
disable-model-invocation: true
allowed-tools: Bash
argument-hint: "[--workdir PATH]"
---

# Stop Zedra Host

Stop the running Zedra Host daemon for the current workspace.

## Step 1 — Confirm status before stopping

```bash
zedra status --workdir "." 2>&1
```

If no daemon is running, inform the user and stop.

## Step 2 — Stop the daemon

```bash
zedra stop --workdir "."
```

Report whether the stop was successful.
