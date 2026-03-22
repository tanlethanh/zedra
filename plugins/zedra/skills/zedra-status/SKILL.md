---
name: zedra-status
description: Show status of the running Zedra Host daemon. Use when the user asks if zedra is running, wants to check connection status, or needs to see active sessions.
allowed-tools: Bash
argument-hint: "[--workdir PATH]"
---

# Zedra Host Status

Show the status of the running Zedra Host daemon and all active instances.

## Step 1 — Check specific workspace

```bash
zedra status --workdir "." 2>&1
```

## Step 2 — List all instances

```bash
zedra list
```

Report to the user:
- Whether the daemon is running
- Workdir, uptime, endpoint ID
- Active sessions and terminal count
- Connection status (connected/disconnected)
