---
name: zedra-terminal
description: Open a new terminal on the connected phone via the running Zedra Host daemon. Use when the user wants to open a terminal on their phone, send a command to mobile, or launch a tool on the remote device.
disable-model-invocation: true
allowed-tools: Bash
argument-hint: "[--launch-cmd CMD]"
---

# Open Remote Terminal

Open a new terminal on the connected Zedra mobile app via the running daemon.

## Step 1 — Verify daemon is running

```bash
zedra status --workdir . 2>&1 || echo "NOT_RUNNING"
```

If `NOT_RUNNING`, tell the user to run `/zedra-start` first.

## Step 2 — Open terminal

If `$ARGUMENTS` was provided, use it as the launch command:
```bash
zedra terminal --workdir . --launch-cmd "$ARGUMENTS"
```

If no arguments were provided, auto-detect the current agent tool and resume it:

**Claude Code** (when `${CLAUDE_SESSION_ID}` is set):
```bash
zedra terminal --workdir . --launch-cmd "claude --resume ${CLAUDE_SESSION_ID}"
```

**Codex:**
```bash
zedra terminal --workdir . --launch-cmd "codex resume --last"
```

**OpenCode:**
```bash
zedra terminal --workdir . --launch-cmd "opencode --continue"
```

**Gemini CLI:**
```bash
zedra terminal --workdir . --launch-cmd "gemini --resume"
```

**No agent detected** — open a plain shell:
```bash
zedra terminal --workdir .
```

Report the result. The terminal appears on the user's connected phone.
