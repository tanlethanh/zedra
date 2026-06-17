# Adding a Managed Agent

This document is the reference for wiring a new agent into the Zedra managed-agent system. Read it alongside an existing agent implementation (Claude or Codex for full-featured agents, Pi or Hermes for simpler ones).

## What "managed agent" means

A managed agent is an AI coding or personal agent whose sessions Zedra can list, resume, and observe via hook events. The system has two loosely coupled halves:

- **Session scanning** — read local session state (files, DBs) and surface it in the Zedra UI.
- **Hook receiving** — receive lifecycle events fired by the agent's hook system and forward them as push notifications and RPC events to connected mobile clients.

An agent can support one or both halves independently.

## File map

| Path | Purpose |
|------|---------|
| `crates/zedra-rpc/src/proto.rs` | `AgentKind` enum variant |
| `crates/zedra-host/src/agent_<name>.rs` | Session scanning, event normalization, config reading |
| `crates/zedra-host/src/agent.rs` | `ManagedAgent` trait impl + dispatch registration |
| `crates/zedra-host/src/agent_utils.rs` | `program_name`, `display_name` entries |
| `crates/zedra-host/src/agent_hook_recv.rs` | `<Name>HookReceiver` — Delta notification logic |
| `crates/zedra-host/src/api.rs` | Hook dispatch arm |
| `crates/zedra-host/src/agent_cli.rs` | CLI kind, synthetic payload, optional `install_hooks` support |

## Step 1 — Add `AgentKind` variant

In `crates/zedra-rpc/src/proto.rs`, add the variant to the `AgentKind` enum. It must also be handled in every `match kind` in `proto.rs` (serialization helpers, display, etc.) and in `docs/PROTOCOL_SPECS.md`.

```rust
// proto.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
    Hermes,
    YourAgent, // new
}
```

This is a breaking protocol change. Bump the relevant protocol version if clients and hosts need to stay in sync.

## Step 2 — Create `agent_<name>.rs`

Create `crates/zedra-host/src/agent_<name>.rs`. Implement the functions expected by the `ManagedAgent` trait. At minimum:

```rust
use zedra_rpc::proto::{AgentEventKind, AgentLifecycleStatus, AgentKind, /* ... */};
use crate::agent_utils::command_on_path;

/// True when the agent is installed or its data directory exists.
pub fn cli_available() -> bool {
    command_on_path("youragent") || sessions_root().is_dir()
}

/// Map the agent's hook event name strings to canonical kinds/statuses.
/// Return None for unknown or uninteresting events; the hook pipeline drops those.
pub fn normalize_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "session_start" => (AgentEventKind::SessionStarted, AgentLifecycleStatus::Starting),
        "session_end" | "done" => (AgentEventKind::TurnCompleted, AgentLifecycleStatus::Completed),
        "permission_request" => (AgentEventKind::PermissionRequested, AgentLifecycleStatus::WaitingForPermission),
        "error" | "failed" => (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed),
        _ => return None,
    })
}
```

### Session scanning functions

The trait requires `session_counts` and `sessions`. Both return typed structs; see `agent_claude.rs` or `agent_pi.rs` for reference. If the agent has no local session storage, return empty counts and an empty slice.

### Account fields

`account_fields` returns a flat list of `AgentInfoField` rows shown in the agent detail panel. Include auth state, model defaults, and plan info where available. Never include raw tokens or secrets.

## Step 3 — Register in `agent.rs`

Add a `struct YourAgentAgent;` block implementing `ManagedAgent` and wire it into `dispatch`:

```rust
struct YourAgentAgent;
impl ManagedAgent for YourAgentAgent {
    fn kind(&self) -> AgentKind { AgentKind::YourAgent }

    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_youragent::normalize_event(event)
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        agent_youragent::cli_available()
    }

    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_youragent::session_counts(ctx.workdir)?.into())
    }

    fn sessions(&self, ctx: &ScanCtx, limit: usize) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_youragent::sessions(ctx.workdir, ctx.cli, limit)
    }

    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField> {
        agent_youragent::account_fields(workdir)
    }

    fn command_matches(&self, command: &str) -> bool {
        command_program_is(&command.to_ascii_lowercase(), "youragent")
    }

    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        value_after_flag(tokens, "--session")
    }

    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("youragent --session {quoted}")
    }
}

// In dispatch():
fn dispatch(kind: AgentKind) -> &'static dyn ManagedAgent {
    match kind {
        // ...existing arms...
        AgentKind::YourAgent => &YourAgentAgent,
    }
}
```

If the agent's sessions are not scoped to a workspace (like Hermes), override `is_global` to return `true`. The scan machinery will cache results across workspace switches.

## Step 4 — Add `program_name` and `display_name`

In `crates/zedra-host/src/agent_utils.rs`:

```rust
pub fn program_name(kind: AgentKind) -> &'static str {
    match kind {
        // ...
        AgentKind::YourAgent => "youragent",
    }
}

pub fn display_name(kind: AgentKind) -> &'static str {
    match kind {
        // ...
        AgentKind::YourAgent => "YourAgent",
    }
}
```

## Step 5 — Add `<Name>HookReceiver` in `agent_hook_recv.rs`

Add a struct for the hook receiver. How much context to include in the notification depends on whether the agent exposes session title information at hook time.

**Minimal receiver (no title enrichment):**

```rust
pub struct YourAgentHookReceiver;

impl YourAgentHookReceiver {
    pub async fn receive(&self, event: AgentEventSummary, ctx: HookContext) {
        let session = ctx.session().await;
        if push_rpc(AgentKind::YourAgent, &event, session).await {
            return;
        }
        if ctx.terminal_id.is_none() {
            return;
        }
        let Some(delta) = DeltaHookClient::from_client(ctx.delta) else {
            return;
        };
        delta.send(self.build_notification(&event)).await;
    }

    fn build_notification(&self, event: &AgentEventSummary) -> HookNotification {
        let agent = agent_utils::display_name(AgentKind::YourAgent);
        HookNotification {
            title: event_title(agent, event.kind),
            body: None,
            content_state: serde_json::json!({
                "agent": agent,
                "event": format!("{:?}", event.kind),
            }),
        }
    }
}
```

**With session title enrichment (like `CodexHookReceiver`):**

If the agent stores session titles locally (transcript JSONL, SQLite DB), look them up in a `spawn_blocking` call using the `event.session_id`:

```rust
let session_id = event.session_id.clone();
let workdir = ctx.workdir.clone();
let body = tokio::task::spawn_blocking(move || {
    session_id.as_deref()
        .and_then(|id| agent_youragent::title_for_session(&workdir, id))
})
.await
.unwrap_or(None);
```

The `terminal_id` guard (`ctx.terminal_id.is_none() → return`) keeps Delta notifications scoped to terminals that were spawned from within a Zedra session. Hooks from unrelated processes sharing the same workdir should not produce mobile notifications.

## Step 6 — Wire the API dispatch arm

In `crates/zedra-host/src/api.rs`, add the import and dispatch arm:

```rust
// At the top with other hook receiver imports:
use crate::agent_hook_recv::{
    ClaudeHookReceiver, CodexHookReceiver, HookContext, OpenCodeHookReceiver,
    YourAgentHookReceiver, // new
};

// In receive_agent_hook_handler, inside the tokio::spawn:
match kind {
    AgentKind::Claude => ClaudeHookReceiver { transcript_path }.receive(event, ctx).await,
    AgentKind::Codex => CodexHookReceiver.receive(event, ctx).await,
    AgentKind::OpenCode => OpenCodeHookReceiver.receive(event, ctx).await,
    AgentKind::YourAgent => YourAgentHookReceiver.receive(event, ctx).await, // new
    _ => {}
}
```

## Step 7 — Add CLI support

In `crates/zedra-host/src/agent_cli.rs`:

**Add `CliManagedAgentKind` variant:**

```rust
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliManagedAgentKind {
    // ...
    YourAgent,
}

impl CliManagedAgentKind {
    fn slug(self) -> &'static str {
        match self {
            // ...
            Self::YourAgent => "youragent",
        }
    }
}

impl From<CliManagedAgentKind> for AgentKind {
    fn from(value: CliManagedAgentKind) -> Self {
        match value {
            // ...
            CliManagedAgentKind::YourAgent => AgentKind::YourAgent,
        }
    }
}
```

**Add a synthetic hook payload for `zedra agent hook test`:**

```rust
fn synthetic_hook_payload(kind: CliManagedAgentKind, event_name: &str, workdir: &Path) -> serde_json::Value {
    match kind {
        // ...
        CliManagedAgentKind::YourAgent => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "event": event_name,
                "sessionId": "zedra-test-session",
                "cwd": cwd,
            })
        }
    }
}
```

**Add `install_hooks` support (optional):**

If the agent has a documented hook configuration format, add a `write_youragent_hook_config` function and register it in `install_hooks`. If not, add a warning and skip:

```rust
CliManagedAgentKind::YourAgent => {
    eprintln!("warning: YourAgent hook config format is undocumented; skipping");
}
```

## Step 8 — Add to `scan_bench` and `scan_usage`

In `agent_cli.rs`, `scan_bench` and `scan_usage` iterate over a fixed list of agent kinds. Add `AgentKind::YourAgent` to both lists.

## Step 9 — Validate

```sh
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host
cargo test -p zedra-host -- agent
```

Smoke-test the hook path end-to-end with the daemon running:

```sh
zedra agent hook test youragent session_start --workdir .
zedra agent hook test youragent error --workdir .
```

Add a `normalize_event` test in `agent_youragent.rs` following the pattern in `agent_claude.rs` and `agent_opencode.rs`.

---

## Hook event name conventions

When defining `normalize_event`, match the exact strings the agent fires. Common patterns across agents:

| Agent style | Example event names |
|-------------|---------------------|
| Claude (PascalCase) | `SessionStart`, `Stop`, `PermissionRequest`, `PostToolUse` |
| OpenCode (dot.case) | `session.status`, `session.idle`, `tool.execute.before` |
| Codex (PascalCase) | `SessionStart`, `PermissionRequest`, `PostToolUse`, `Stop` |
| Pi (snake_case, normalized in the extension) | native `before_agent_start`, `agent_end`, `session_shutdown` → wire `UserPromptSubmit`, `Stop` |
| Generic (snake_case) | `session_start`, `session_end`, `permission_request`, `error` |

Map to `AgentEventKind` variants. Only handle events the agent actually fires. Unknown events return `None` and are dropped — that is intentional and keeps the pipeline clean.

### Pi: extension-based hook delivery

Pi has no shell-hook config file. Instead `zedra setup pi` writes a TypeScript
extension to `~/.pi/agent/extensions/zedra-agent-hooks.ts`, which pi
auto-discovers at session start. The extension shells back into the zedra binary
(`zedra agent hook receive --agent pi`) on lifecycle events, mirroring the
OpenCode plugin pattern. It is a no-op outside a Zedra terminal (no
`ZEDRA_TERMINAL_ID`) and for non-interactive pi runs (`ctx.hasUI === false`).
Pi exposes no approval/permission event, so `PiHookReceiver` only drives
`Running`/`Completed` state and notifies on `Stop`. See `pi_hook_extension` in
`setup.rs` and `pi_hooks_installed` in `agent_setup.rs`.

## Global vs workspace-scoped agents

Most agents (Claude, Codex, OpenCode, Pi) scope their sessions to a workspace directory. Pass the `workdir` through to session scans and per-project config reads.

**Global agents** (currently Hermes) ignore `workdir` for sessions because their history is stored in a single user-level directory. Override `is_global() -> bool` to return `true`. The scan cache machinery will not invalidate these results on workspace switches.

Even for global agents, `HookContext.workdir` is still set in the hook receiver — it reflects the daemon's working directory and can be used for logging or correlation.
