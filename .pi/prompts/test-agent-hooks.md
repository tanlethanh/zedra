---
description: Exercise a real pi turn that drives Zedra Running and Completed state
---

# Test Agent Hooks

Exercise a real pi turn and confirm its hooks drive Zedra state and Delta notifications. Does not send synthetic hooks.

## Prerequisites

- Zedra daemon running in this repo.
- Pi CLI on PATH.
- Launch pi from a **Zedra terminal** so `ZEDRA_TERMINAL_ID` is set; Delta notifications require it.
- Global pi hook extension installed. `~/.pi/agent/extensions/zedra-agent-hooks.ts` must forward pi events to Zedra. Run `zedra agent hook install --provider pi` to install it.

## Hook to state

| Event | Expected Zedra state |
|-------|----------------------|
| `before_agent_start` | Running |
| `tool_execution_end` | Running |
| `agent_end` | Completed |
| `session_shutdown` | Completed |

Pi has no permission approval event, so there is no WaitingApproval transition.
Delta notifies on completion when the app is backgrounded or quit.

## Actions

Perform these actions now using only real pi tool calls:

1. Run a shell command that waits for 5 seconds.
2. After it completes, run another shell command that waits for 5 seconds.
3. Complete the turn.

Do not call `zedra agent hook receive`, `zedra agent hook test`, or manually emit hook events.

## User verification

Do not attempt to verify the Zedra UI or notifications yourself. The user will verify:

Foreground app path:

1. The command starts the real pi turn.
2. Confirm the terminal card shows the Running indicator.
3. Wait 5 seconds.
4. Confirm the terminal card returns to Running after the first tool completes.
5. Wait 5 seconds.
6. Confirm the turn completes and the terminal card shows the Completed indicator while the app is foregrounded.

Background or quit app path:

1. The command starts the real pi turn.
2. Background or quit the app while the first 5-second wait is running.
3. Confirm a Delta completion notification arrives after the turn finishes.

## Rules

- Do not run `zedra agent hook receive` or `zedra agent hook test`; this command validates only naturally emitted pi extension events.
- Do not use a normal chat question as a substitute for a real tool call.
