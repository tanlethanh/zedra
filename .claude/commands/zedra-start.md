Start the Zedra daemon to enable remote control with Zedra App.

## Steps

**Step 1 — Check if daemon is already running:**
```bash
zedra status 2>/dev/null && echo "running" || echo "not running"
```

**Step 2a — Start daemon if not running:**

If Step 1 printed "not running", start it with the launch command (substitute the real session ID) and capture the QR output:
```bash
zedra start --workdir . --launch-cmd "claude --resume ${CLAUDE_SESSION_ID}" > /tmp/zedra-start.log 2>&1 &
sleep 2
cat /tmp/zedra-start.log
```

Display the ASCII QR code from the log to the user so they can scan it from the phone to pair.

**Step 2b — Daemon already running: open a resumed Claude terminal:**

If Step 1 printed "running", trigger a new terminal manually (substitute the real session ID):
```bash
zedra terminal --launch-cmd "claude --resume ${CLAUDE_SESSION_ID}"
```
