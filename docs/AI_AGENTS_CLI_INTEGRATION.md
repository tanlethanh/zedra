# AI Agents CLI Integration

Research on how TUI coding agents expose diff/edit events to terminal emulators and editors.

## Landscape

Each tool rolls its own protocol. No universal standard yet, but ACP is converging.

| Tool | Protocol | Transport | Diff access |
|------|----------|-----------|-------------|
| Claude Code | MCP | WebSocket + lock file | `openDiff` tool (blocking, full spec below) |
| OpenAI Codex | MCP | stdio | CLI only, no IDE diff |
| opencode | HTTP + SSE | localhost HTTP | TUI-native, supports ACP |
| Aider | File watching | Filesystem | `# AI!` comments, stdout diffs |

---

## Claude Code — WebSocket MCP Protocol

Fully reverse-engineered by [coder/claudecode.nvim](https://github.com/coder/claudecode.nvim). Source: `PROTOCOL.md`.

### Discovery

IDE extension writes `~/.claude/ide/[PORT].lock` (0600). CLI scans this dir on startup.

Lock file:
```json
{
  "pid": 12345,
  "workspaceFolders": ["/path/to/project"],
  "ideName": "VS Code",
  "transport": "ws",
  "authToken": "550e8400-e29b-41d4-a716-446655440000"
}
```

Env vars set by IDE when launching `claude`:
- `CLAUDE_CODE_SSE_PORT` — the port
- `ENABLE_IDE_INTEGRATION=true`

### Connection

`ws://127.0.0.1:[PORT]` — localhost only. Auth via WebSocket header:
```
x-claude-code-ide-authorization: <authToken>
```

Protocol: MCP spec 2025-03-26, JSON-RPC 2.0 over WebSocket frames.

### IDE → Claude notifications

**selection_changed** — fires on every cursor/selection change:
```json
{
  "jsonrpc": "2.0", "method": "selection_changed",
  "params": {
    "text": "selected text",
    "filePath": "/abs/path/file.rs",
    "fileUrl": "file:///abs/path/file.rs",
    "selection": {
      "start": { "line": 10, "character": 5 },
      "end": { "line": 15, "character": 20 },
      "isEmpty": false
    }
  }
}
```

**at_mentioned** — when user explicitly @-sends context to Claude:
```json
{
  "jsonrpc": "2.0", "method": "at_mentioned",
  "params": { "filePath": "/path/to/file", "lineStart": 10, "lineEnd": 20 }
}
```

### 12 MCP tools (Claude → IDE)

Claude calls these via `tools/call`. Response format: `{content: [{type: "text", text: "..."}]}`.

| Tool | Key params | Response |
|------|-----------|----------|
| `openFile` | `filePath`, `startText`, `endText`, `preview`, `makeFrontmost` | `"Opened file: ..."` or JSON with `languageId`, `lineCount` |
| **`openDiff`** | `old_file_path`, `new_file_path`, `new_file_contents`, `tab_name` | `"FILE_SAVED"` or `"DIFF_REJECTED"` |
| `getCurrentSelection` | — | JSON: `{text, filePath, selection}` |
| `getLatestSelection` | — | JSON: `{text, filePath, selection}` |
| `getOpenEditors` | — | JSON: `{tabs: [{uri, isActive, label, languageId, isDirty}]}` |
| `getWorkspaceFolders` | — | JSON: `{folders: [{name, uri, path}], rootPath}` |
| `getDiagnostics` | `uri` (optional) | JSON: `[{uri, diagnostics: [{message, severity, range, source}]}]` |
| `checkDocumentDirty` | `filePath` | JSON: `{isDirty, isUntitled}` |
| `saveDocument` | `filePath` | JSON: `{saved, message}` |
| `close_tab` | `tab_name` | `"TAB_CLOSED"` |
| `closeAllDiffTabs` | — | `"CLOSED_N_DIFF_TABS"` |
| `executeCode` | `code` | mixed content: text + base64 images (Jupyter) |

### openDiff in detail

**Blocking** — Claude waits for user accept/reject before continuing.

```json
{
  "jsonrpc": "2.0", "id": "req-1", "method": "tools/call",
  "params": {
    "name": "openDiff",
    "arguments": {
      "old_file_path": "/path/to/file.rs",
      "new_file_path": "/path/to/file.rs",
      "new_file_contents": "// full new content...",
      "tab_name": "Proposed changes"
    }
  }
}
```

Response after user action:
```json
{ "jsonrpc": "2.0", "id": "req-1", "result": { "content": [{ "type": "text", "text": "FILE_SAVED" }] } }
```

User can also edit the diff before accepting — Claude is notified the file may differ from the proposal.

### Context injection — images and rich content

Claude Code supports image context via:
- **Drag & drop** into the terminal window
- **Clipboard paste** — copy a screenshot (Shift+Cmd+Ctrl+4 on macOS), paste with Ctrl+V
- **MCP servers** — `Playwright MCP` or `Screenshot MCP` capture programmatically:
  ```
  claude mcp add playwright npx @playwright/mcp@latest
  ```

`executeCode` returns mixed content including `{type: "image", data: "base64...", mimeType: "image/png"}` — used for Jupyter plot output.

Tools visible to the model: `mcp__ide__getDiagnostics`, `mcp__ide__executeCode`. All others are CLI-internal (hidden from the model).

CVE-2025-52882: pre-v1.0.24 had no auth — any local process could connect.

---

## ACP — Agent Client Protocol

Emerging open standard. JSON-RPC 2.0 over **stdio** — editor spawns agent as subprocess, pipes stdin/stdout. No lock files, no WebSocket.

Backed by: Zed, JetBrains (preview), Neovim (2 plugins), opencode, Gemini CLI, Goose. Claude Code does not support ACP yet.

### Features beyond file editing

- **Session management** — create, load, resume, close with streaming responses
- **Plan tracking** — agent emits real-time progress (analysis → context gathering → drafting); Zed renders this as a live progress UI
- **Bidirectional** — agent can request files, run shell commands, read diagnostics from the editor side
- **Multi-buffer diff review** — full workspace context, not single-file
- **Workspace policy file** — auto-approve specific tool prompts

Tools: `read_text_file`, `write_text_file`, `run_command`, `bash`, `grep_file`, `run_pty_cmd`.

Spec: https://agentclientprotocol.com, https://zed.dev/acp

---

## Terminal notification protocols

Claude Code emits **OSC 9** at the end of each agent turn:
```
ESC ] 9 ; <message> BEL
```

Other formats:
- `OSC 99 ; title=<t> ; subtitle=<s> BEL` — with subtitle/ID
- `OSC 777 ; notify ; <message> BEL` — tmux format

Terminal support matrix:
| Terminal | Support |
|---|---|
| Kitty, Ghostty | Native |
| iTerm2, Windows Terminal | OSC 9 |
| VS Code terminal | Silently drops (bug) |
| tmux | Requires `set -g allow-passthrough on` |

---

## Terminal graphics protocols

No AI agent renders inline graphics today, but the infrastructure exists.

**Kitty graphics protocol** (`ESC _ G <control_data> ; <payload> ESC \`):
- Formats: 24-bit RGB, 32-bit RGBA, PNG
- Data: direct bytes, file path, shared memory, chunked
- Features: z-index, alpha, animations, scaling
- Supported by: Kitty, WezTerm, Ghostty, Konsole (adding)

**iTerm2 OSC 1337 inline images**:
```
ESC ] 1337 ; File=inline=1;height=70% : <base64 data> ST
```

**Sixel** — DEC 1980s bitmap encoding, supported by mlterm, libsixel.

Claude Code has open feature requests for Sixel/Kitty rendering (#2266, #29254). Not implemented yet.

---

## OSC sequences relevant to AI agents

| Sequence | Purpose | Status in zedra-osc |
|---|---|---|
| OSC 0/2 | Terminal title | ✅ parsed |
| OSC 7 | CWD | ✅ parsed |
| OSC 133 A/B/C/D | Semantic zones (prompt/command boundaries) | ✅ parsed |
| **OSC 633 A/B/C/D/E/P** | VS Code shell integration; `633;E` = command text | ❌ missing |
| **OSC 8** | Hyperlinks | ❌ missing |
| **OSC 9** | Desktop notifications (agent turn end) | ❌ missing |
| OSC 1337 | iTerm2 inline images | ❌ missing (future) |

OSC 633;E is particularly useful — captures the exact command line before execution, so we know when `claude` or `opencode` was invoked.

---

## opencode features

- HTTP + SSE transport; TUI is the primary client, VS Code extension just wraps it
- Custom commands: Markdown files in `commands/`, support `$ARGUMENTS`, `!bash`, `@file`, subtask spawning
- Tool permissions per command: `"ask"`, `"allow"`, `"deny"`
- Supports ACP — can be driven by Zed or any ACP editor
- MCP tool support alongside built-in tools

---

## Integration options for zedra-host

**Option A — Fake IDE (Claude Code only)**
Write `~/.claude/ide/[port].lock` from zedra-host. Run WebSocket MCP server. Implement the 12 tools. Intercept `openDiff` — forward `{old_file_path, new_file_path, new_file_contents}` over RPC to mobile. Respond `FILE_SAVED`/`DIFF_REJECTED` based on user action. Full spec known, implementable today.

Beyond diffs, also enables: live selection sync to mobile (`selection_changed`), diagnostics display (`getDiagnostics`), file navigation (`openFile`), workspace awareness (`getWorkspaceFolders`).

**Option B — ACP server (multi-agent)**
zedra-host implements ACP server. Spawns coding agent as subprocess, wires stdin/stdout. Covers opencode, Gemini CLI, Goose, and future ACP agents. Claude Code does not support ACP yet.

**Option C — PTY JSON stream scanning (Claude Code, no setup)**
Scan PTY bytes for `--output-format stream-json` JSON lines alongside OSC scanning. Extract tool_use edits client-side. Fragile (ANSI interleaved) but requires zero user setup.

**Option D — OSC 633 + semantic blocks**
Add 633 parsing to zedra-osc. Capture command text (`633;E`) and output byte range (`633;C` → `633;D`). Agent-agnostic. Lets Zedra build "command blocks" like Warp. Doesn't give diff content directly but works for any tool.

**Option E — OSC 9 notification capture**
Add OSC 9 parsing. Surface agent turn-end notifications as mobile push or in-app toast. Zero-friction — works for any agent that emits OSC 9, no server needed.
