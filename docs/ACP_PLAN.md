# ACP (Agent Client Protocol) Integration Plan

## Overview

Bring AI coding agents to Zedra mobile by relaying the open ACP protocol between the mobile client and an agent CLI running as a subprocess on the host machine.

ACP is a JSON-RPC 2.0 over stdio protocol developed by Zed Industries. The agent CLI handles all API calls, tool execution, and context management. Zedra provides the mobile UI and relays messages.

- Spec: https://agentclientprotocol.com
- Rust SDK: `agent-client-protocol` crate (v0.10+)
- TypeScript SDK: `@agentclientprotocol/sdk`
- ACP Agent Registry: https://github.com/agentclientprotocol/registry
- Registry JSON: https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json

## Agent Ecosystem (29 registered agents)

The ACP registry lists 29 agents as of 2026-03-12. Since all speak the same protocol,
zedra-host's ACP client code works with any of them — the user just picks which agent to use.

### Distribution types
- **npx** (Node.js) — 15 agents: `npx <package>@<version> [--acp]`
- **binary** (platform-specific download) — 10 agents: `./<binary> acp`
- **uvx** (Python) — 3 agents: `uvx <package>@<version> acp`

### Major agents

| Agent | Command | Auth | Notes |
|-------|---------|------|-------|
| Claude | `npx @zed-industries/claude-agent-acp@0.21.0` | Anthropic key / Claude Pro | Adapter (wraps Claude Agent SDK) |
| Codex CLI | `npx @zed-industries/codex-acp@0.9.5` | `OPENAI_API_KEY` | Adapter (wraps Codex CLI) |
| Gemini CLI | `npx @google/gemini-cli@0.32.1 --experimental-acp` | Google OAuth | Native ACP |
| GitHub Copilot LS | `npx @github/copilot-language-server@1.449.0 --acp` | GitHub OAuth | Native ACP |
| GitHub Copilot CLI | `npx @github/copilot@1.0.3 --acp` | GitHub OAuth | Native ACP |
| Cursor | `./cursor-agent acp` | Cursor subscription | Native ACP (binary) |
| OpenCode | `./opencode acp` | User's API keys | Native ACP (binary) |
| Goose | `./goose acp` | User's API keys | Native ACP (binary) |
| Cline | `npx cline@2.6.1 --acp` | User's API keys | Native ACP |
| Junie | `./junie --acp=true` | JetBrains account | Native ACP (binary) |
| Kilo | `npx @kilocode/cli@7.0.41 acp` | User's API keys | Native ACP |
| Qwen Code | `npx @qwen-code/qwen-code@0.12.0 --acp` | Alibaba account | Native ACP |
| Mistral Vibe | `./vibe-acp` | Mistral account | Native ACP (binary) |
| Auggie | `npx @augmentcode/auggie@0.18.1 --acp` | Augment Code account | Native ACP |
| Kimi CLI | `./kimi acp` | Moonshot AI account | Native ACP (binary) |

### Other agents (full registry)

Amp, Autohand Code, Codebuddy Code (Tencent), Corust Agent, crow-cli,
DeepAgents (LangChain), DimCode, Factory Droid, fast-agent, Minion Code,
Nova (Compass AI), pi ACP, Qoder CLI, Stakpak.

### Not in registry (no ACP support)

- **aider** — no ACP, uses its own CLI protocol
- **Continue** — VS Code extension only, no ACP

### Authentication model

All agents manage their own auth. zedra-host never touches API keys or tokens.
Two patterns:
1. **Agent Auth** (most common): agent opens browser OAuth flow, stores token itself
2. **Terminal Auth**: agent runs interactive TUI setup (e.g., `claude /login`)

For zedra-host, auth is handled by the agent subprocess on the host machine.
The mobile client never sees credentials.

## How ACP Actually Works (Claude Code example)

There is **no `--acp` flag** on the `claude` CLI. ACP is a 3-layer stack:

```
Editor (Zed)
  │ spawns via npx
  ▼
claude-agent-acp          ← ACP server (npm: @zed-industries/claude-agent-acp)
  │ stdin/stdout JSON-RPC 2.0 (newline-delimited)
  │ reads ACP from stdin, writes ACP to stdout
  │
  │ internally uses Claude Agent SDK
  ▼
@anthropic-ai/claude-agent-sdk
  │ spawns claude CLI as subprocess
  ▼
claude --output-format stream-json --input-format stream-json --verbose
  │ the actual agent doing tool execution, API calls, etc.
```

Key facts:
- Zed spawns `npx @zed-industries/claude-agent-acp@0.21.0` (from ACP registry)
- The ACP adapter (`src/acp-agent.ts`) calls `runAcp()` which reads stdin, writes stdout
- Internally it uses `@anthropic-ai/claude-agent-sdk` `query()` function
- The SDK's `initialize()` spawns the real `claude` binary with stream-json flags
- Other agents may have their own ACP adapters (e.g., `codex-acp`, `goose acp`)

### Invocation from ACP Registry

Each agent has a registry entry (`agents/<name>/agent.json`):
```json
{
  "name": "Claude",
  "distribution": {
    "npx": {
      "package": "@zed-industries/claude-agent-acp@0.21.0"
    }
  }
}
```

Zed runs: `npx @zed-industries/claude-agent-acp@0.21.0` with stdio piped.

## Architecture Options for Zedra

### Option A: Spawn ACP adapter (recommended)

zedra-host spawns the agent's ACP adapter (e.g., `claude-agent-acp`) and speaks full ACP protocol. This is what Zed does.

```
Mobile Client          zedra-host              claude-agent-acp (subprocess)
─────────────          ──────────              ─────────────────────────────
                       npx claude-agent-acp ──> stdin/stdout piped
                       send initialize ───────> negotiate capabilities
                       session/new ───────────> create session

AcpPrompt ──irpc───>   session/prompt ────────> agent processes
                       <──── session/update ───  streaming tokens + tool calls
<──irpc── AcpEvent     relay to client

                       <──── fs/read_text_file   agent reads file
                       handle locally ──────>    respond with contents

                       <──── request_permission  needs user approval
<──irpc── AcpEvent     relay to client
AcpPermission ─irpc─>  respond to agent ──────> continues execution

                       <──── session/update ───  more streaming
<──irpc── AcpEvent     relay to client
                       <──── PromptResponse ───  turn complete
<──irpc── AcpEvent     TurnComplete
```

**Pros**: Agent-agnostic (works with any ACP-compatible agent), standard protocol
**Cons**: Requires Node.js/npx on host for npm-distributed agents

### Option B: Spawn claude directly with stream-json

Skip ACP entirely. zedra-host spawns `claude --output-format stream-json --input-format stream-json` and speaks Claude's native stream-json protocol.

**Pros**: Simpler, no npm dependency, no ACP adapter layer
**Cons**: Claude-specific, need to implement stream-json parsing, not compatible with other agents

### Option C: Implement ACP server in Rust

Use the `agent-client-protocol` Rust crate to implement an ACP server in zedra-host. Call the Anthropic API directly or spawn claude underneath.

**Pros**: Full control, no npm dependency
**Cons**: Most complex, reimplements what the adapter already does

**Recommendation**: Start with **Option A** for full ACP compatibility. Fall back to Option B if Node.js dependency is problematic.

### Critical Design Split

The host handles two distinct roles:

1. **Relay** (mobile <-> agent): User prompts, streaming text, tool call status, permission requests
2. **Local execution** (agent <-> host filesystem): The agent's fs/terminal requests execute on the host machine where the code lives — these are NOT relayed to mobile

| Direction | What | Handled by |
|-----------|------|-----------|
| Mobile -> Agent | User prompts, cancel, permission responses | Relayed via irpc |
| Agent -> Mobile | Text tokens, tool call status, permission requests | Relayed via irpc |
| Agent -> Host | fs/read_text_file, fs/write_text_file, terminal/* | Executed locally on host |

## ACP Protocol Summary (from source analysis)

Sources analyzed:
- `@agentclientprotocol/sdk` v0.11+ (types.gen.d.ts, acp.d.ts)
- `@zed-industries/claude-agent-acp` v0.17.1 (acp-agent.js)
- `@anthropic-ai/claude-agent-sdk` v0.2.45 (sdk.mjs)

### Wire Format

Newline-delimited JSON-RPC 2.0 over stdin/stdout. Each message is one JSON object + `\n`.

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}
{"jsonrpc":"2.0","id":1,"result":{...}}
{"jsonrpc":"2.0","method":"session/update","params":{...}}
```

### Connection Lifecycle

1. Spawn agent binary with stdio piped
2. Send `initialize` -> receive capabilities + auth methods
3. If `auth_required`, send `authenticate`
4. Send `session/new` (or `session/load`) -> receive `session_id`
5. Send `session/prompt` -> receive streaming `session/update` notifications -> handle `request_permission` requests -> turn ends with `PromptResponse`
6. Send `session/cancel` to abort at any time

### Client -> Agent Methods

| Method | Purpose |
|--------|---------|
| `initialize` | Version + capability negotiation |
| `authenticate` | Auth handshake (if required) |
| `session/new` | Create conversation |
| `session/load` | Resume existing conversation (replays history via notifications) |
| `session/prompt` | Send user message (blocks until turn complete) |
| `session/cancel` | Cancel in-progress turn (notification, no response) |
| `session/list` | List known sessions (unstable) |
| `session/set_mode` | Switch mode (default/acceptEdits/plan/dontAsk/bypassPermissions) |
| `session/set_model` | Switch model (unstable) |
| `session/fork` | Fork a session (unstable) |
| `session/resume` | Resume without replaying history (unstable) |

### Agent -> Client Methods

| Method | Purpose |
|--------|---------|
| `session/update` | Stream real-time updates (notification, no response) |
| `request_permission` | Ask user to approve a tool call (request, blocks until response) |
| `fs/read_text_file` | Read file from host filesystem |
| `fs/write_text_file` | Write file on host filesystem |
| `terminal/create` | Spawn shell command on host |
| `terminal/output` | Get terminal stdout/exit status |
| `terminal/wait_for_exit` | Block until command exits |
| `terminal/kill` | Kill running command |
| `terminal/release` | Kill + free terminal |

### Session Update Types (SessionUpdate discriminated union)

Streamed via `session/update` notifications during a prompt turn. Discriminated by `sessionUpdate` field:

```typescript
type SessionUpdate =
  | ContentChunk & { sessionUpdate: "user_message_chunk" }     // echoed user text
  | ContentChunk & { sessionUpdate: "agent_message_chunk" }    // LLM text streaming
  | ContentChunk & { sessionUpdate: "agent_thought_chunk" }    // thinking/reasoning
  | ToolCall    & { sessionUpdate: "tool_call" }               // new tool call started
  | ToolCallUpdate & { sessionUpdate: "tool_call_update" }     // tool status/result update
  | Plan        & { sessionUpdate: "plan" }                    // execution plan (TodoWrite)
  | AvailableCommandsUpdate & { sessionUpdate: "available_commands_update" }
  | CurrentModeUpdate & { sessionUpdate: "current_mode_update" }
  | ConfigOptionUpdate & { sessionUpdate: "config_option_update" }
  | SessionInfoUpdate & { sessionUpdate: "session_info_update" }
  | UsageUpdate & { sessionUpdate: "usage_update" }
```

### Content Blocks (in PromptRequest.prompt and ContentChunk)

```typescript
type ContentBlock =
  | { type: "text", text: string }
  | { type: "image", data?: string, mimeType?: string, uri?: string }
  | { type: "audio", ... }
  | { type: "resource_link", uri: string, name: string, ... }
  | { type: "resource", resource: { uri: string, text: string } }
```

### ToolCall (new tool invocation)

```typescript
type ToolCall = {
  toolCallId: string;           // unique ID within session
  title: string;                // human-readable, e.g. "Read src/auth.rs"
  kind?: ToolKind;              // "read"|"edit"|"delete"|"execute"|"search"|"think"|"fetch"|"other"
  status?: ToolCallStatus;      // "pending"|"in_progress"|"completed"|"failed"
  content?: ToolCallContent[];  // produced content (text, diff, terminal)
  locations?: ToolCallLocation[]; // file paths affected
  rawInput?: unknown;           // raw tool input params
  rawOutput?: unknown;          // raw tool output
  _meta?: { claudeCode?: { toolName: string } }  // extension: actual tool name
}
```

### ToolCallContent (what a tool produces)

```typescript
type ToolCallContent =
  | { type: "content", content: ContentBlock }     // text/image block
  | { type: "diff", path: string, diff: string }   // unified diff
  | { type: "terminal", terminalId: string }        // terminal output reference
```

### ToolCallUpdate (update existing tool call)

Same fields as ToolCall but all optional except `toolCallId`. Only changed fields sent.

### Tool Call Status Flow

`pending` -> `in_progress` -> `completed` | `failed`

### ToolKind (icon/UI hints)

`"read"` | `"edit"` | `"delete"` | `"move"` | `"search"` | `"execute"` | `"think"` | `"fetch"` | `"switch_mode"` | `"other"`

### Permission Request

Agent sends `request_permission` JSON-RPC request (blocks waiting for response):

```typescript
// Agent -> Client (request)
type RequestPermissionRequest = {
  sessionId: string;
  toolCall: ToolCallUpdate;     // details about what needs permission
  options: PermissionOption[];  // choices presented to user
}

// Client -> Agent (response)
type RequestPermissionResponse = {
  outcome: { outcome: "cancelled" } | { outcome: "selected", optionIndex: number }
}
```

### PromptRequest

```typescript
type PromptRequest = {
  sessionId: string;
  prompt: ContentBlock[];  // array of text, image, resource_link, resource blocks
}
```

### PromptResponse (turn complete)

```typescript
type PromptResponse = {
  stopReason: "end_turn" | "max_tokens" | "max_turn_requests" | "refusal" | "cancelled";
  usage?: { cachedReadTokens?: number, ... }  // unstable
}
```

### StopReason

`"end_turn"` | `"max_tokens"` | `"max_turn_requests"` | `"refusal"` | `"cancelled"`

### How claude-agent-acp maps SDK events to ACP

The adapter receives events from the Claude Agent SDK `query()` async iterator:

| SDK event type | ACP SessionUpdate |
|----------------|-------------------|
| `stream_event` → `content_block_start` (text) | `agent_message_chunk` |
| `stream_event` → `content_block_delta` (text_delta) | `agent_message_chunk` |
| `stream_event` → `content_block_start` (thinking) | `agent_thought_chunk` |
| `stream_event` → `content_block_delta` (thinking_delta) | `agent_thought_chunk` |
| `stream_event` → `content_block_start` (tool_use) | `tool_call` (status: pending) |
| `assistant` message → `tool_result` content | `tool_call_update` (status: completed/failed) |
| `result` → `success` | PromptResponse { stopReason: "end_turn" } |
| `result` → `error_max_turns` | PromptResponse { stopReason: "max_turn_requests" } |

### Client Capabilities Declaration

In `initialize`, the client declares what it supports:

```typescript
type ClientCapabilities = {
  fs?: {
    readTextFile?: boolean;   // client can handle fs/read_text_file requests
    writeTextFile?: boolean;  // client can handle fs/write_text_file requests
  };
  terminal?: boolean;         // client can handle terminal/* requests
  _meta?: {
    "terminal-auth"?: boolean;  // client supports terminal-based auth
    "terminal_output"?: boolean; // client sends terminal output updates
  }
}
```

When client declares `fs.readTextFile`, the adapter disallows the agent's built-in `Read` tool and adds `mcp__acp__Read` (routed to the client). Similarly for write/terminal.

For zedra-host (acting as ACP client), we should declare:
- `fs.readTextFile: true` + `fs.writeTextFile: true` (handle locally on host)
- `terminal: true` (handle locally via PTY)
This means the agent's tools route through us, and we execute them on the host filesystem.

### ACP Tool Routing (how tools actually work)

The ACP adapter sets up an MCP server named "acp" that exposes tools like `mcp__acp__Read`, `mcp__acp__Write`, `mcp__acp__Bash`. When Claude uses these, the adapter:
1. Calls `client.readTextFile()` / `client.writeTextFile()` / `client.createTerminal()` — these are JSON-RPC requests BACK to the client
2. The client (zedra-host) handles them locally
3. Results are returned to the agent

The built-in `Read`/`Write`/`Bash` tools are disallowed so Claude must use the MCP-prefixed versions that route through the client.

## irpc Protocol Types (`zedra-rpc/src/proto.rs`)

New types added to `ZedraProto`:

```rust
// ---------------------------------------------------------------------------
// Client -> Host
// ---------------------------------------------------------------------------

/// Start an ACP agent subprocess on the host.
/// agent_id is a registry ID (e.g., "claude-acp", "opencode", "gemini").
/// If None, uses the default from config or first available.
AcpStart { agent_id: Option<String> }
  -> AcpStartResult { ok: bool, agent_name: String, error: Option<String> }

/// Send a user prompt. Returns a bidi stream of AcpEvent.
AcpPrompt { session_id: String, message: String }
  -> streams AcpEvent

/// Cancel the current in-progress turn.
AcpCancel { session_id: String }
  -> AcpCancelResult { ok: bool }

/// Respond to a permission request from the agent.
AcpPermissionResponse { session_id: String, request_id: String, outcome: String, selected_index: Option<u32> }
  -> AcpPermissionAck { ok: bool }

/// List available ACP sessions (conversations).
AcpListSessions {}
  -> AcpSessionList { sessions: Vec<AcpSessionInfo> }

/// Load/resume an existing session. Returns history as AcpEvent stream.
AcpLoadSession { session_id: String }
  -> streams AcpEvent

/// Set agent mode (ask/code/architect).
AcpSetMode { session_id: String, mode: String }
  -> AcpSetModeResult { ok: bool }

// ---------------------------------------------------------------------------
// Host -> Client (streamed via bidi stream)
// ---------------------------------------------------------------------------

enum AcpEvent {
    /// Streaming assistant text token.
    TextDelta { text: String },
    /// Streaming thinking/reasoning token.
    ThoughtDelta { text: String },
    /// A tool call has started.
    ToolCallStarted { tool_use_id: String, tool_name: String, input_preview: String },
    /// A tool call status/result update.
    ToolCallUpdated { tool_use_id: String, status: String, output_preview: Option<String> },
    /// Agent requests user permission for a tool call.
    PermissionRequired { request_id: String, tool_name: String, description: String, options: Vec<String> },
    /// The agent's turn is complete.
    TurnComplete { stop_reason: String },
    /// Error during generation.
    Error { message: String },
}
```

## Host Implementation (`zedra-host`)

### New files

```
crates/zedra-host/src/
  acp.rs              # AcpManager: subprocess lifecycle, JSON-RPC parsing
  acp_handler.rs      # Handle agent->host requests (fs, terminal) locally
```

### AcpManager (`acp.rs`)

**Key insight**: zedra-host implements an ACP **client** (like Zed), not an ACP server.
It spawns the agent subprocess, sends requests to it, and handles agent-to-client
requests (fs, terminal, permission) locally. The mobile app is NOT an ACP client —
it talks to zedra-host via irpc, and zedra-host translates to/from ACP.

Responsibilities:
- Spawn agent adapter subprocess (e.g., `node claude-agent-acp/dist/index.js`)
- Parse newline-delimited JSON-RPC from stdout
- Send JSON-RPC requests to agent via stdin
- Handle agent->client requests locally (fs/read_text_file, fs/write_text_file, terminal/*)
- Relay session/update notifications to mobile client via irpc AcpEvent stream
- Relay request_permission to mobile client, wait for response, reply to agent

```rust
pub struct AcpManager {
    /// Running agent subprocess
    process: Option<Child>,
    /// stdin writer (send JSON-RPC messages to agent)
    stdin_tx: Option<mpsc::Sender<String>>,
    /// Agent capabilities from initialize response
    capabilities: Option<AgentCapabilities>,
    /// Active session ID
    session_id: Option<String>,
    /// JSON-RPC request ID counter
    next_request_id: u64,
    /// Pending JSON-RPC responses awaiting from agent (id -> oneshot sender)
    pending_responses: HashMap<u64, oneshot::Sender<serde_json::Value>>,
    /// Pending permission requests awaiting mobile user response (agent request id -> oneshot sender)
    pending_permissions: HashMap<u64, oneshot::Sender<RequestPermissionResponse>>,
    /// Channel to send AcpEvents to the irpc client stream
    event_tx: Option<mpsc::Sender<AcpEvent>>,
}
```

### Message routing in stdout reader task

The stdout reader task parses each JSON line and routes based on message type:

```
Agent stdout line (JSON-RPC)
  ├─ Has "id" + "result"/"error" → response to our request → resolve pending_responses[id]
  ├─ Has "id" + "method" → agent request TO US (client):
  │   ├─ "fs/read_text_file"     → read file locally, respond with content
  │   ├─ "fs/write_text_file"    → write file locally, respond with ok
  │   ├─ "terminal/create"       → spawn PTY, respond with terminal_id
  │   ├─ "terminal/output"       → read PTY output, respond
  │   ├─ "terminal/wait_for_exit"→ await PTY exit, respond
  │   ├─ "terminal/kill"         → kill PTY process, respond
  │   ├─ "terminal/release"      → kill + cleanup PTY, respond
  │   └─ "request_permission"    → relay to mobile via irpc, await response, respond to agent
  └─ Has "method" only (no "id") → notification:
      └─ "session/update"        → relay to mobile via irpc AcpEvent stream
```

### Agent Binary Discovery

Search order for agent binary (if not explicitly configured):

1. `$ZEDRA_ACP_AGENT` environment variable
2. `~/.config/zedra/acp.toml` config file
3. Auto-detect from PATH: `claude`, `opencode`, `codex` (first found)

Config file format:
```toml
[agent]
binary = "claude"
args = ["--acp"]
```

### Agent -> Host Request Handling (`acp_handler.rs`)

These requests from the agent are executed locally on the host — they never reach the mobile client:

```rust
/// Handle fs/read_text_file: read from host filesystem
fn handle_read_file(path: &str) -> Result<String>

/// Handle fs/write_text_file: write to host filesystem
fn handle_write_file(path: &str, content: &str) -> Result<()>

/// Handle terminal/create: spawn command via PTY
fn handle_terminal_create(command: &str, args: &[String]) -> Result<TerminalId>

/// Handle terminal/output: read terminal stdout buffer
fn handle_terminal_output(terminal_id: &str) -> Result<TerminalOutput>

/// Handle terminal/wait_for_exit: block until process exits
fn handle_terminal_wait(terminal_id: &str) -> Result<ExitStatus>

/// Handle terminal/kill: send SIGTERM to process
fn handle_terminal_kill(terminal_id: &str) -> Result<()>

/// Handle terminal/release: kill + cleanup
fn handle_terminal_release(terminal_id: &str) -> Result<()>
```

### Integration with rpc_daemon.rs

Add ACP dispatch to the existing RPC handler:

```rust
// In handle_rpc()
ZedraProto::AcpStart(req) => { /* spawn agent, initialize, return agent_id */ }
ZedraProto::AcpPrompt(req) => { /* session/prompt + stream AcpEvents back */ }
ZedraProto::AcpCancel(req) => { /* session/cancel notification to agent */ }
ZedraProto::AcpPermissionResponse(req) => { /* respond to pending request_permission */ }
ZedraProto::AcpListSessions(req) => { /* session/list to agent */ }
ZedraProto::AcpLoadSession(req) => { /* session/load + stream history */ }
ZedraProto::AcpSetMode(req) => { /* session/set_mode to agent */ }
```

## Mobile UI

### New files

```
crates/zedra/src/
  acp_view.rs          # AcpView: main chat GPUI view (scrollable message list + input)
  acp_message.rs       # Message bubble rendering (user / assistant / tool call)
  acp_tool_card.rs     # Collapsible tool call card with status + approval buttons
```

### Screen Layout

```
+-----------------------------------+
| [=]    Agent     [status]         |  <- WorkspaceContent header (existing)
+-----------------------------------+
|                                   |
|  +-----------------------------+  |
|  | You                         |  |  <- User message (right-aligned, blue tint)
|  | Fix the login bug in auth   |  |
|  +-----------------------------+  |
|                                   |
|  +-----------------------------+  |
|  | Assistant                   |  |  <- Assistant message (left-aligned)
|  | I'll look at the auth       |  |     Markdown: bold, code, lists
|  | module to find the issue.   |  |
|  |                             |  |
|  | +-------------------------+ |  |  <- Tool call card (inline, collapsible)
|  | | read_file               | |  |     Tap to expand/collapse
|  | | src/auth.rs             | |  |
|  | | [check] Done            | |  |     Status: pending/running/done/error
|  | +-------------------------+ |  |
|  |                             |  |
|  | +-------------------------+ |  |  <- Permission request card
|  | | write_file              | |  |
|  | | src/auth.rs             | |  |
|  | | [Approve]  [Deny]       | |  |     Large touch targets (48px min)
|  | +-------------------------+ |  |
|  |                             |  |
|  | The fix has been applied.   |  |
|  +-----------------------------+  |
|                                   |
+-----------------------------------+
| [clip] Type a message...    [->]  |  <- Input bar (sticky bottom)
+-----------------------------------+
```

### AcpView (`acp_view.rs`)

GPUI view serving as a main content view in WorkspaceContent (same pattern as TerminalView, EditorView).

```rust
pub struct AcpView {
    /// All messages in the conversation (user + assistant + tool calls).
    messages: Vec<AcpMessage>,
    /// Current streaming text buffer (appended to on TextDelta events).
    streaming_buffer: String,
    /// Whether the agent is currently processing a turn.
    is_streaming: bool,
    /// Input text field content.
    input_text: String,
    /// Session ID for this conversation.
    session_id: Option<String>,
    /// Pending permission requests awaiting user action.
    pending_permissions: Vec<PendingPermission>,
    /// Focus handle for keyboard input.
    focus_handle: FocusHandle,
}

pub enum AcpMessage {
    User { text: String },
    Assistant { blocks: Vec<AcpContentBlock> },
}

pub enum AcpContentBlock {
    Text { text: String },
    Thought { text: String, collapsed: bool },
    ToolCall {
        tool_use_id: String,
        tool_name: String,
        input_preview: String,
        status: ToolCallStatus,
        output_preview: Option<String>,
        collapsed: bool,
    },
}

pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

pub struct PendingPermission {
    request_id: String,
    tool_name: String,
    description: String,
    options: Vec<String>,
}
```

### Message Rendering (`acp_message.rs`)

- **User messages**: Right-aligned bubble, `BG_CARD` with subtle blue tint, `TEXT_PRIMARY`
- **Assistant text**: Left-aligned, `TEXT_SECONDARY`, basic markdown (bold, inline code, code blocks)
- **Thinking blocks**: Collapsed by default, `TEXT_MUTED`, italic, tap to expand
- **Tool call cards**: `BG_CARD` with `BORDER_SUBTLE`, status dot (yellow=pending, blue=running, green=done, red=failed)
- **Permission cards**: Same as tool card but with Approve (`ACCENT_GREEN`) / Deny (`ACCENT_RED`) buttons, 48px min touch target

### Tool Call Cards (`acp_tool_card.rs`)

Collapsible cards showing tool execution status:

```
+----------------------------------+
| [icon] read_file    [v] [status] |  <- Header: tool name + collapse toggle + status dot
|   src/auth.rs                    |  <- Input preview (file path, command, etc.)
|   ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~  |  <- Separator (when expanded)
|   fn login(user: &str) {        |  <- Output preview (when expanded)
|     if !validate(user) { ...    |
+----------------------------------+
```

- Tap header to expand/collapse
- Status dot animates (pulse) while `InProgress`
- Output preview truncated to ~10 lines with "Show more" link

### Input Bar

- Sticky at bottom, above keyboard when visible
- Text input field with placeholder "Type a message..."
- Send button (arrow icon) — disabled when empty or streaming
- Stop button (square icon) — shown during streaming, sends `AcpCancel`
- Clip button (paperclip) — future: attach file context

### Keyboard Integration

Reuse existing `keyboard.rs` pattern:
```rust
view.set_keyboard_request(crate::keyboard::make_keyboard_handler());
view.set_is_keyboard_visible_fn(crate::keyboard::make_is_keyboard_visible());
```

Input field tapped -> show keyboard. Send message -> optionally keep keyboard open.

### WorkspaceDrawer Integration

Add Agent section to the drawer:

```rust
pub enum DrawerSection {
    Files,
    Git,
    Terminal,
    Session,
    Agent,     // NEW
}
```

Agent tab content:
- Active conversation highlighted
- List of previous conversations (from `AcpListSessions`)
- "+ New Conversation" button
- Agent status indicator (connected/disconnected)

New drawer events:
```rust
WorkspaceDrawerEvent::AgentConversationSelected(String),  // session_id
WorkspaceDrawerEvent::NewAgentConversation,
```

### WorkspaceView Integration

Same pending slot pattern as terminals:

```rust
pub struct WorkspaceView {
    // ... existing fields ...
    acp_view: Option<Entity<AcpView>>,
    pending_acp_session: SharedPendingSlot<String>,
}
```

When agent conversation selected, swap `WorkspaceContent` main view to `AcpView`.

## Theme Constants

All new UI uses existing `theme.rs` values:

| Element | Color |
|---------|-------|
| User message bubble bg | `BG_CARD` + `ACCENT_BLUE` at 0.08 opacity |
| Assistant message bg | transparent (just text on `BG_PRIMARY`) |
| Tool card bg | `BG_CARD` |
| Tool card border | `BORDER_SUBTLE` |
| Tool status: pending | `ACCENT_YELLOW` |
| Tool status: running | `ACCENT_BLUE` |
| Tool status: done | `ACCENT_GREEN` |
| Tool status: failed | `ACCENT_RED` |
| Approve button text | `ACCENT_GREEN` |
| Deny button text | `ACCENT_RED` |
| Thinking text | `TEXT_MUTED`, italic |
| Input bar bg | `BG_SURFACE` |
| Input bar border | `BORDER_DEFAULT` |

## Implementation Phases

### Phase 1: Host-side ACP subprocess manager
**Goal**: Spawn ACP agent adapter, relay basic prompts, return text responses.

Files:
- `zedra-rpc/src/proto.rs` — add ACP request/response types
- `zedra-host/src/acp.rs` — AcpManager: subprocess spawn, JSON-RPC I/O
- `zedra-host/src/acp_handler.rs` — handle agent->host fs/terminal requests
- `zedra-host/src/rpc_daemon.rs` — wire ACP dispatch

Tasks:
1. Add `AcpStart`, `AcpPrompt`, `AcpCancel`, `AcpEvent` to proto.rs
2. Implement AcpManager: spawn `npx @zed-industries/claude-agent-acp` with stdin/stdout piped
3. Parse newline-delimited JSON-RPC 2.0 messages from agent stdout
4. Send `initialize` request with client capabilities (fs.readTextFile, fs.writeTextFile, terminal)
5. Handle `initialize` response (agent capabilities, auth methods)
6. If auth required, handle `authenticate` flow (agent manages its own API key)
7. Send `session/new` to create conversations
8. Forward `session/prompt` and stream `session/update` notifications as `AcpEvent` over irpc
9. Handle agent->host `fs/read_text_file` requests (read from host filesystem, respond)
10. Handle agent->host `fs/write_text_file` requests (write to host filesystem, respond)
11. Handle agent->host `terminal/*` requests via existing PTY infrastructure
12. Forward `request_permission` to mobile client, relay response back to agent
13. Wire into rpc_daemon.rs dispatch
14. Test with `zedra client` CLI (text-only output, no UI)

Dependencies: Node.js/npx must be available on the host machine.

### Phase 2: Basic chat UI
**Goal**: Working conversation with streaming text on mobile.

Files:
- `crates/zedra/src/acp_view.rs` — AcpView GPUI view
- `crates/zedra/src/acp_message.rs` — message bubble rendering

Tasks:
1. Create AcpView with scrollable message list (uniform_list)
2. Input bar with text field + send button
3. Keyboard show/hide integration
4. Wire AcpView as a swappable main view in WorkspaceContent
5. Send AcpPrompt over irpc on message send
6. Receive AcpEvent stream and append TextDelta to assistant bubble
7. Handle TurnComplete to finalize message
8. Stop button sends AcpCancel during streaming
9. Add Agent tab to WorkspaceDrawer (hardcoded single conversation for now)

### Phase 3: Tool calls + permissions
**Goal**: Display tool execution and handle user approval.

Files:
- `crates/zedra/src/acp_tool_card.rs` — tool call card rendering

Tasks:
1. Render ToolCallStarted as inline cards in assistant messages
2. Update cards on ToolCallUpdated (status transitions + output preview)
3. Render PermissionRequired as approval cards with Approve/Deny buttons
4. Send AcpPermissionResponse on button tap
5. Collapsible card expand/collapse on tap
6. Status dot coloring and pulse animation for in-progress

### Phase 4: Multi-agent + session management
**Goal**: Agent picker, multiple conversations, history, configuration.

Files:
- `zedra-host/src/acp_registry.rs` — fetch/cache ACP registry, agent discovery
- `crates/zedra/src/acp_picker.rs` — agent selection UI on mobile

Tasks:
1. Implement registry fetching + caching (`AcpListAgents` / `AcpInstallAgent`)
2. Agent picker UI on mobile (list agents, show installed status, tap to select)
3. Agent discovery: check Zed cache → global PATH → registry download
4. `~/.config/zedra/acp.toml` config file support
5. Implement AcpListSessions / AcpLoadSession for conversation history
6. Conversation list in drawer Agent tab
7. "+ New Conversation" creates fresh session with selected agent
8. Mode switching (default/acceptEdits/plan/dontAsk/bypassPermissions) via UI toggle
9. Model selection (from agent's `availableModels`)
10. Thinking blocks (collapsed by default, expandable)
11. Basic markdown rendering (bold, code blocks, lists)
12. Diff preview for write_file tool results

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Node.js/npx not on host | Detect at AcpStart; fall back to Option B (direct `claude --output-format stream-json`); clear error if neither available |
| Agent CLI not installed on host | Auto-detect from PATH; clear error message if not found |
| Agent auth required (API key) | Agent handles its own auth (ANTHROPIC_API_KEY env or claude login); host just relays |
| ACP adapter version mismatch | Pin version in config; allow user override |
| Large streaming responses | Virtualized list (uniform_list), batch token rendering per frame |
| Permission request timeout | Keep request pending until user acts; agent blocks waiting |
| Agent subprocess crash | Detect exit code, surface error to mobile, offer restart |
| Multiple concurrent turns | ACP is single-turn; queue prompts or error if busy |
| Network latency | Buffer events client-side; render in batches per frame |
| npx cold start latency | First invocation downloads package (~10s); subsequent runs cached |

## Agent Management

### Registry-based agent discovery

zedra-host fetches the ACP registry JSON and presents available agents to the mobile client.
The user picks an agent from the list on their phone, and zedra-host handles installation + spawning.

Registry URL: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`

Each registry entry contains:
```json
{
  "id": "claude-acp",
  "name": "Claude Agent",
  "version": "0.21.0",
  "description": "ACP wrapper for Anthropic's Claude",
  "distribution": {
    "npx": { "package": "@zed-industries/claude-agent-acp@0.21.0" }
  },
  "authMethods": ["agent-auth", "terminal-auth"]
}
```

### Agent installation strategies

| Distribution | How to spawn | Requirements |
|-------------|--------------|--------------|
| **npx** | `npx <package>@<version> [flags]` | Node.js 18+ |
| **binary** | Download platform archive, extract, run `./<binary> [flags]` | None |
| **uvx** | `uvx <package>@<version> [flags]` | Python 3.10+ / uv |

### Agent discovery order (on host)

1. **User config**: `~/.config/zedra/acp.toml` — explicit agent command
2. **Zed's agent cache**: `~/Library/Application Support/Zed/external_agents/<agent-id>/<version>/`
3. **Global binary**: check PATH for agent binaries (`opencode`, `goose`, `codex`, etc.)
4. **Registry download**: fetch registry, download/install the selected agent
5. **zedra agent cache**: `~/.config/zedra/agents/<agent-id>/<version>/`

### Agent config file (`~/.config/zedra/acp.toml`)

```toml
# Explicitly set the agent to use
[agent]
id = "claude-acp"                    # registry ID (auto-resolves command)
# OR specify a custom command directly:
# command = "node"
# args = ["/path/to/my-agent/index.js", "--acp"]
# env = { "MY_API_KEY" = "..." }

# Override default agent per project
[projects."/home/user/myproject"]
agent_id = "opencode"
```

### Zed's agent cache (reusable)

Zed pre-downloads npm-distributed agents to its app support directory:
```
~/Library/Application Support/Zed/external_agents/
  claude-agent-acp/0.17.1/
    node_modules/@zed-industries/claude-agent-acp/dist/index.js    ← entry point
    node_modules/@anthropic-ai/claude-agent-sdk/cli.js             ← bundled claude
    node_modules/@agentclientprotocol/sdk/                         ← protocol SDK
  registry/registry.json                                           ← cached registry
```

To spawn from Zed's cache:
```bash
node "~/Library/Application Support/Zed/external_agents/claude-agent-acp/0.17.1/node_modules/@zed-industries/claude-agent-acp/dist/index.js"
```

### Agent invocation examples

```bash
# Claude (adapter, npx)
npx @zed-industries/claude-agent-acp@0.21.0

# Codex (adapter, npx)
npx @zed-industries/codex-acp@0.9.5

# Gemini CLI (native, npx)
npx @google/gemini-cli@0.32.1 --experimental-acp

# GitHub Copilot (native, npx)
npx @github/copilot-language-server@1.449.0 --acp

# OpenCode (native, binary)
opencode acp

# Goose (native, binary)
goose acp

# Cline (native, npx)
npx cline@2.6.1 --acp

# Cursor (native, binary)
cursor-agent acp

# Junie (native, binary)
junie --acp=true
```

### irpc types for agent management

```rust
/// List available agents (from registry + locally installed).
AcpListAgents {}
  -> AcpAgentList { agents: Vec<AcpAgentInfo> }

struct AcpAgentInfo {
    id: String,                    // "claude-acp", "opencode", etc.
    name: String,                  // "Claude Agent"
    version: String,               // "0.21.0"
    description: String,
    installed: bool,               // already downloaded/available
    distribution: String,          // "npx", "binary", "uvx"
}

/// Install an agent (download if needed).
AcpInstallAgent { agent_id: String }
  -> AcpInstallResult { ok: bool, error: Option<String> }
```

### Mobile UI: Agent picker

The mobile app shows an agent selection screen (before or during session creation):
- List of agents from `AcpListAgents` response
- Installed agents shown with a checkmark
- Tap to select → `AcpStart { agent_id }` spawns that agent
- Settings gear → opens agent config (model, mode, etc.)

## Future Enhancements (not in initial scope)

- File context attachment from mobile file explorer
- Image input (camera/screenshot)
- Agent output displayed in terminal view (command execution)
- Conversation search
- Export conversation as markdown
- Per-project agent selection
- Agent auth flow on mobile (browser redirect for OAuth agents)
