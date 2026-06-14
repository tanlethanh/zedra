---
name: test-agent-hooks
description: Exercise a real agent hook turn that drives Zedra Running, WaitingApproval, and Completed state.
allowed-tools: Bash
argument-hint: "codex | opencode | pi"
---

# Test Agent Hooks

Exercise a real agent turn and confirm its hooks drive Zedra state and Delta notifications. Installs nothing and does not send synthetic hooks.

Argument: `$1` is the provider under test: `codex`, `opencode`, or `pi`. If omitted, use the agent currently running this skill.

## Prerequisites

- Zedra daemon running in this repo.
- The provider CLI on PATH.
- Launch the agent from a **Zedra terminal** so `ZEDRA_TERMINAL_ID` is set; Delta notifications require it.
- Provider hooks installed (`zedra agent hook install --provider <provider>` writes them):
  - `codex`: `.codex/hooks.json` must register `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`, and `Stop` hooks that call the Zedra hook script.
  - `opencode`: `.opencode/plugins/zedra.js` in this repo, or the global `~/.config/opencode/plugins/zedra-agent-hooks.js`.
  - `pi`: `~/.pi/agent/extensions/zedra-agent-hooks.ts`.

## Hook → state

`codex` (Claude-style hook events):

| Event | Expected Zedra state |
|-------|----------------------|
| `UserPromptSubmit` | Running |
| `PermissionRequest` | WaitingApproval |
| `PostToolUse` | Running |
| `Stop` | Completed |

`opencode` (native plugin events):

| Event | Expected Zedra state |
|-------|----------------------|
| `session.status` (busy/retry) | Running |
| `permission.asked` | WaitingApproval |
| `permission.replied` | Running |
| `session.idle` | Completed |

`pi` (extension normalizes native events to Claude-style names):

| Event | Expected Zedra state |
|-------|----------------------|
| `before_agent_start` → `UserPromptSubmit` | Running |
| `tool_execution_end` → `PostToolUse` | Running (no state change) |
| `agent_end` / `session_shutdown` → `Stop` | Completed |

Pi has no permission approval event, so there is no WaitingApproval transition for pi.

Delta notifies on approval (codex/opencode) and completion when the app is backgrounded or quit.

## Codex hook output contract

Codex accepts either no stdout with exit code `0`, plain text context on stdout, or the documented JSON shape for `UserPromptSubmit`:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "Optional context for this turn."
  }
}
```

Do not print ad hoc JSON from a `UserPromptSubmit` hook. If Codex reports `UserPromptSubmit hook (failed): error: hook returned invalid user prompt submit JSON output`, fix the hook command output before using this skill. The Zedra hook script should normally run quiet and print nothing.

## Residual risk

Codex and OpenCode expose approval events, but a denied approval or a tool path that emits no follow-up event may leave the card in WaitingApproval until the turn ends. Codex has no documented permission-resolved hook; the approved path returns to Running through `PostToolUse`.

## Actions

Perform these actions now using only real tool calls from the provider under test:

For `codex` and `opencode`:

1. Run a shell command that waits for 5 seconds.
2. Read `~/.ssh/config` in a way that requires approval.
3. After the approval is resolved, run another shell command that waits for 5 seconds.
4. Complete the turn.

For `pi` (no approval step):

1. Run a shell command that waits for 5 seconds.
2. After it completes, run another shell command that waits for 5 seconds.
3. Complete the turn.

Do not call `zedra agent hook receive`, `zedra agent hook test`, or manually emit hook events.

## User verification

Do not attempt to verify the Zedra UI or notifications yourself. The user will verify:

Foreground app path:

1. The turn starts and the terminal card shows the Running indicator.
2. Wait 5 seconds.
3. `codex`/`opencode`: the agent asks for approval to read `~/.ssh/config` and the terminal card shows the WaitingApproval indicator. Approve or deny; if approved, the card returns to Running after the read tool completes. `pi`: the card stays Running after the first tool completes.
4. Wait 5 seconds.
5. The turn completes and the terminal card shows the Completed indicator while the app is foregrounded.

Background or quit app path:

1. The turn starts; background or quit the app while the first 5-second wait is running.
2. `codex`/`opencode`: confirm a Delta notification arrives for the approval request, open the app from it, and approve or deny. Background or quit the app again while the second 5-second wait is running.
3. Confirm a completion notification arrives.

## Rules

- Do not run `zedra agent hook receive` or `zedra agent hook test`; this skill validates only naturally emitted provider hooks and extension events.
- Do not use a normal chat question as a substitute for a real tool call or the approval prompt.
- The sensitive file read should be `~/.ssh/config` unless the user chooses another file the provider will require approval to access.
