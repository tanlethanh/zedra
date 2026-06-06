Test Claude hook handling during the current Claude turn using only hooks emitted naturally by Claude.

## What it does

1. Uses the real `UserPromptSubmit` emitted for this command → state turns **Running** (blue dot)
2. Requests permission to read `~/.ssh/config` → the real `PermissionRequest` turns state **WaitingApproval** (yellow dot), Delta notification if backgrounded
3. Uses the real `Stop` emitted when the enclosing Claude turn finishes → state turns **Completed** (green dot), Delta notification if backgrounded
4. Done — state reverts to **Idle** after the 30-second host timer

## Usage

```
/test-claude-hooks
```

## Instructions

Use the Read tool to read `~/.ssh/config`, requiring user approval through Claude's tool permission dialog. Do not use Bash, ask for approval as a normal chat question, invoke `zedra agent hook receive`, or manually send any hook events; Claude emits the hooks for the enclosing command and permission request.

After the command is approved or denied, report: `Claude hook test complete. This turn used only real Claude hook events.`
