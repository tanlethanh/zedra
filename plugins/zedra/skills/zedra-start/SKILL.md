---
name: zedra-start
description: Start the Zedra Host daemon to enable remote control from the Zedra mobile app. Checks if zedra CLI is installed, installs it if missing, then launches the daemon and prints a QR code for pairing. Use when the user wants to connect their phone, start zedra, or pair a mobile device.
disable-model-invocation: true
allowed-tools: Bash
argument-hint: "[--workdir PATH]"
---

# Start Zedra Host

Launch the Zedra Host daemon so the user can pair their mobile device via QR code.

## Step 1 — Check if `zedra` CLI is installed

```bash
command -v zedra && zedra --help 2>&1 | head -1 || echo "NOT_INSTALLED"
```

If `NOT_INSTALLED`, proceed to Step 2. Otherwise skip to Step 3.

## Step 2 — Install `zedra` CLI

Ask the user which installation method they prefer, then run it:

**Option A — Pre-built binary (fastest, macOS Apple Silicon):**
```bash
curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh
```

**Option B — Build from source (any platform with Rust toolchain):**
```bash
cargo install --git https://github.com/tanlethanh/zedra zedra-host
```

After installation, verify:
```bash
zedra --help 2>&1 | head -1
```

If it fails, check that `~/.local/bin` or `~/.cargo/bin` is in PATH and advise the user.

## Step 3 — Check if daemon is already running

```bash
zedra status --workdir "." 2>/dev/null && echo "RUNNING" || echo "NOT_RUNNING"
```

If `RUNNING`, skip to Step 5 (open terminal on existing daemon).
If `NOT_RUNNING`, proceed to Step 4.

## Step 4 — Start the daemon

Start zedra-host in the background. The daemon itself does NOT take `--launch-cmd`;
the launch command is passed per-terminal via `zedra terminal --launch-cmd`.

```bash
nohup zedra start --workdir "." > /tmp/zedra-start.log 2>&1 &
sleep 3
cat /tmp/zedra-start.log
```

**IMPORTANT**: Display the full ASCII QR code from the output to the user. The QR code is essential — the user must scan it with the Zedra mobile app to pair. Also display:
- The pairing URL (starts with `zedra://connect?ticket=`)
- Host info (hostname, endpoint ID)
- Relay and direct address info

If the log shows errors, diagnose and report them.

After the daemon is running and QR is displayed, open a terminal with the agent
resume command so the phone gets a live session immediately (Step 5).

## Step 5 — Open a resumed terminal on the phone

Open a new terminal with `--launch-cmd` to resume the current agent session.
Pick the right command based on which agent tool is invoking this skill:

| Agent tool | launch-cmd value | How resume works |
|------------|-----------------|-----------------|
| Claude Code | `claude --resume ${CLAUDE_SESSION_ID}` | Resumes exact session by ID |
| Codex | `codex resume --last` | Resumes most recent session |
| OpenCode | `opencode --continue` | Continues last session |
| Gemini CLI | `gemini --resume` | Resumes most recent session |
| Unknown / none | _(omit --launch-cmd)_ | Plain shell |

**Detection logic:**
1. If `${CLAUDE_SESSION_ID}` is set and non-empty → Claude Code
2. Else `command -v codex` succeeds → Codex
3. Else `command -v opencode` succeeds → OpenCode
4. Else `command -v gemini` succeeds → Gemini CLI
5. Else → omit `--launch-cmd` (plain shell)

**Claude Code:**
```bash
zedra terminal --workdir "." --launch-cmd "claude --resume ${CLAUDE_SESSION_ID}"
```

**Codex:**
```bash
zedra terminal --workdir "." --launch-cmd "codex resume --last"
```

**OpenCode:**
```bash
zedra terminal --workdir "." --launch-cmd "opencode --continue"
```

**Gemini CLI:**
```bash
zedra terminal --workdir "." --launch-cmd "gemini --resume"
```

Report the result to the user.

## Notes

- The daemon runs in the background and survives shell exit
- Use `/zedra:zedra-stop` to stop it, or `zedra list` to see all instances
- The QR code encodes a one-time pairing ticket; after first scan, the phone reconnects automatically via PKI
- `--launch-cmd` is per-terminal, not global — only terminals opened with it get the agent session
- `/zedra:zedra-terminal` opens additional terminals with agent resume
- `/zedra:zedra-status` checks if the daemon is healthy
