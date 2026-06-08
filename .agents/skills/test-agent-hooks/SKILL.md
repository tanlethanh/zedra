---
name: test-agent-hooks
description: Exercise a real Codex hook turn that drives Zedra Running, WaitingApproval, and Completed state.
allowed-tools: Bash
argument-hint: "codex"
---

# Test Agent Hooks

Exercise a real Codex turn and confirm its hooks drive Zedra state + Delta notifications. Installs nothing and does not send synthetic hooks.

Argument: `$1` is `codex`.

## Prerequisites

- Zedra daemon running in this repo.
- Codex CLI on PATH.
- Launch Codex from a **Zedra terminal** so `ZEDRA_TERMINAL_ID` is set; Delta notifications require it.
- Project Codex hooks installed and trusted. `.codex/hooks.json` must register `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`, and `Stop` hooks that call the Zedra hook script.

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

## Hook → state

| Event | Expected Zedra state |
|-------|----------------------|
| `UserPromptSubmit` | Running |
| `PermissionRequest` | WaitingApproval |
| `PostToolUse` | Running |
| `Stop` | Completed |

Delta notifies on approval and completion when the app is backgrounded or quit.

## Residual risk

Codex has no documented permission-resolved hook. The approved path returns to Running through `PostToolUse`, but a denied approval or a tool path that does not emit `PostToolUse` may remain WaitingApproval until `Stop`.

## Prompt to submit

Submit this prompt in the Zedra terminal running Codex:

```text
Use only real Codex tool calls for this hook test. First run a shell command that waits for 5 seconds. Then read ~/.ssh/config in a way that requires Codex approval. After the approval is resolved, wait another 5 seconds before completing. Do not call zedra agent hook receive, zedra agent hook test, or manually emit hook events.
```

## Manual verification

Foreground app path:

1. Submit the prompt.
2. Confirm the terminal card shows the Running indicator.
3. Wait 5 seconds.
4. Confirm Codex asks for approval to read `~/.ssh/config` and the terminal card shows the WaitingApproval indicator.
5. Approve or deny the request.
6. If approved, confirm the terminal card returns to the Running indicator after the read tool completes.
7. Wait 5 seconds.
8. Confirm the turn completes and the terminal card shows the Completed indicator while the app is foregrounded.

Background or quit app path:

1. Submit the prompt.
2. Background or quit the app while the first 5-second wait is running.
3. Confirm a Delta notification arrives for the approval request after Codex asks for approval.
4. Open the app from the notification and approve or deny the request.
5. Background or quit the app again while the second 5-second wait is running.
6. Confirm a completion notification arrives.

## Rules

- Do not run `zedra agent hook receive` or `zedra agent hook test`; this skill validates only naturally emitted Codex hooks.
- Do not use a normal chat question as a substitute for the Codex approval prompt.
- The sensitive file read should be `~/.ssh/config` unless the user chooses another file that Codex will require approval to access.
