---
disable-model-invocation: true
allowed-tools:
  - Bash(*)
---

# /zedra-start

Start the zedra daemon and display a QR code for mobile pairing.

## Instructions

Follow these steps exactly:

### 1. Find or install zedra

Check for `zedra` in PATH first, then fall back to `~/.local/bin/zedra`, then `./target/release/zedra`:

```bash
which zedra 2>/dev/null || (test -x "$HOME/.local/bin/zedra" && echo "$HOME/.local/bin/zedra") || (test -x ./target/release/zedra && echo "./target/release/zedra")
```

If none exist, install it automatically:

```bash
curl -fsSL https://zedra.dev/install.sh | sh
```

After the install script finishes, resolve the binary path again — it will be at `~/.local/bin/zedra` (the default install prefix). If the install fails, show the error and stop.

### 2. Check if already running

```bash
pgrep -f "zedra start" >/dev/null 2>&1 && echo "RUNNING" || echo "NOT_RUNNING"
```

If already running, tell the user zedra is already active and skip to step 5.

### 3. Launch the daemon with --json

Use `--json` to get machine-readable startup output. The daemon prints a single JSON line to stdout when ready, then continues running.

Use the resolved binary path from step 1 (either `zedra` or `./target/release/zedra`).

```bash
ZEDRA_PIPE=$(mktemp -u /tmp/zedra-XXXXXX.pipe)
mkfifo "$ZEDRA_PIPE"
nohup <BINARY> start --workdir "$(pwd)" --json > "$ZEDRA_PIPE" 2>/dev/null &
ZEDRA_PID=$!
# Read the first line (JSON startup info), with a 15-second timeout
read -t 15 ZEDRA_JSON < "$ZEDRA_PIPE"
rm -f "$ZEDRA_PIPE"
echo "$ZEDRA_JSON"
```

Replace `<BINARY>` with the actual path found in step 1.

If the read times out (empty output), tell the user startup may have failed and suggest checking `ps aux | grep zedra` and logs.

### 4. Parse and display

The JSON line has this structure:
```json
{
  "status": "ready",
  "host": "my-macbook",
  "endpoint_id": "7mkinx4w4zey...",
  "device_id": "d3e9f8b4",
  "relay_url": "https://relay.zedra.dev",
  "direct_addrs": ["192.168.1.100:3456"],
  "pairing_uri": "zedra://pair?d=...",
  "qr_code": "█▀▀▀▀▀█..."
}
```

Parse the JSON and display the `qr_code` field as-is (it contains pre-rendered Unicode).

### 5. Show pairing instructions

Display to the user:
- The QR code from the `qr_code` field
- Host: `host`, Endpoint: first 16 chars of `endpoint_id`
- Relay status from `relay_url`
- Instructions: "Scan this QR code with the Zedra mobile app to pair."
- After pairing: "Run `claude --continue` in the mobile terminal to resume your current Claude Code session on your phone."
- The daemon PID so they can stop it later with `kill <PID>`
