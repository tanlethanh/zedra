---
name: test-agent-hooks
description: Exercise a real Hermes turn that drives Zedra Running, WaitingApproval, and Completed state
version: 1.0.0
metadata:
  hermes:
    tags: [zedra, development]
    category: devops
    requires_toolsets: [terminal]
---

# Test Agent Hooks

Exercise a real Hermes turn and confirm its hooks drive Zedra state + Delta notifications. Installs nothing and does not send synthetic hooks.

## Usage

Start a fresh Hermes session from a Zedra terminal, then run:

```text
/test-agent-hooks
```

## Prerequisites

- Zedra daemon running in this repo.
- Hermes CLI on PATH.
- Launch Hermes from a **Zedra terminal** so `ZEDRA_TERMINAL_ID` is set; Delta notifications require it.
- Hermes lifecycle hooks installed. Run `zedra setup hermes --full-bin-path` so
  the hook script uses the current Zedra binary.
- This skill discovered by Hermes. Add this project's `.hermes/skills` directory to `external_dirs` in `~/.hermes/config.yaml` if not already present.
- Use a fresh Hermes session. Hermes emits `on_session_start` only for the first turn, so an existing session will not show the initial Running transition.
- Hermes must be running with approval prompts enabled. Do not use `--yolo` or `-z`/`--oneshot`.

## Hook to state

| Event | Expected Zedra state |
|-------|----------------------|
| `on_session_start` | Running |
| `pre_approval_request` | WaitingApproval |
| `post_approval_response` | Running |
| `post_llm_call` | Completed |
| `on_session_end` | Completed |

Delta notifies on approval and completion when the app is backgrounded or quit.

## Actions

Perform these actions now using only real Hermes tool calls:

1. Run a shell command that waits for 5 seconds.
2. Run `sh -c 'true'` to trigger a harmless Hermes approval request.
3. After the approval is resolved, run another shell command that waits for 5 seconds.
4. Complete the turn.

Do not call `zedra agent hook receive`, `zedra agent hook test`, or manually emit hook events.

## User verification

Do not attempt to verify the Zedra UI or notifications yourself. The user will verify:

Foreground app path:

1. The skill starts the real Hermes turn.
2. Confirm the terminal card shows the Running indicator.
3. Wait 5 seconds.
4. Confirm Hermes asks for approval to run `sh -c 'true'` and the terminal card shows the WaitingApproval indicator.
5. Approve or deny the request.
6. Confirm the terminal card returns to Running after Hermes emits `post_approval_response`.
7. Wait 5 seconds.
8. Confirm the turn completes and the terminal card shows the Completed indicator while the app is foregrounded.

Background or quit app path:

1. The skill starts the real Hermes turn.
2. Background or quit the app while the first 5-second wait is running.
3. Confirm a Delta notification arrives for the approval request.
4. Open the app from the notification and approve or deny the request.
5. Background or quit the app again while the second 5-second wait is running.
6. Confirm a completion notification arrives.

## Rules

- Do not run `zedra agent hook receive` or `zedra agent hook test`; this skill validates only naturally emitted Hermes lifecycle hook events.
- Do not use a normal chat question as a substitute for a real Hermes approval prompt.
- Use `sh -c 'true'` for the approval step. Hermes classifies shell `-c` execution as approval-worthy, while the command itself has no side effects.
