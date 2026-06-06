Simulate a Claude agent session by firing hook events with delays — useful for testing AgentState transitions, indicator dots, push notifications, and sounds.

## What it does

1. Fires `UserPromptSubmit` → state turns **Running** (blue dot)
2. Waits 5 seconds
3. Fires a random `PermissionRequest` question → state turns **WaitingApproval** (yellow dot), Delta notification if backgrounded
4. Waits 5 seconds
5. Fires `TaskCompleted` → state turns **Completed** (green dot), Delta notification if backgrounded
6. Done — state reverts to **Idle** after the 30-second host timer

## Usage

```
/test-claude-hooks
```

Optional env overrides (set before running):
- `ZEDRA_TERMINAL_ID` — terminal to attribute events to (auto-detected if daemon is running)
- `ZEDRA_TEST_WORKDIR` — working directory of the daemon (defaults to current directory)

## Instructions

Run the following bash script. Do not modify the session_id or timing — the sequence is intentional for testing the full state machine.

```bash
#!/usr/bin/env bash
set -e

WORKDIR="${ZEDRA_TEST_WORKDIR:-.}"
SESSION_ID="test-session-$(date +%s)"
TERMINAL_ID="${ZEDRA_TERMINAL_ID:-}"

QUESTIONS=(
  "This tool will read ~/.ssh/config. Approve?"
  "Allow write access to /etc/hosts?"
  "Run network request to api.example.com. Approve?"
  "Execute shell command: rm -rf ./dist. Approve?"
  "Access keychain item com.apple.dt.Xcode. Approve?"
)
QUESTION="${QUESTIONS[$((RANDOM % ${#QUESTIONS[@]}))]}"

send_hook() {
  local event="$1"
  local extra_json="$2"
  local payload="{\"session_id\":\"$SESSION_ID\"$extra_json}"
  local args=(--kind claude --event "$event" --session-id "$SESSION_ID" --workdir "$WORKDIR" --quiet)
  [ -n "$TERMINAL_ID" ] && args+=(--terminal-id "$TERMINAL_ID")
  echo "$payload" | zedra agent hook receive "${args[@]}"
}

echo "🔵 [1/3] UserPromptSubmit → Running"
send_hook "UserPromptSubmit" ""

echo "   Waiting 5s..."
sleep 5

echo "🟡 [2/3] PermissionRequest → WaitingApproval"
echo "   Question: $QUESTION"
send_hook "PermissionRequest" ",\"message\":\"$QUESTION\""

echo "   Waiting 5s..."
sleep 5

echo "🟢 [3/3] TaskCompleted → Completed"
send_hook "TaskCompleted" ",\"title\":\"Hook test complete\""


echo "✓ Done. State reverts to Idle in ~30s."
```
