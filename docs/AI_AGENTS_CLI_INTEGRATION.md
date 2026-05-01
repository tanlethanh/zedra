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

## Common OSC sequences and terminal metadata

OSC sequences are terminal control strings, usually shaped as `ESC ] <id> ; <payload> ST` or `ESC ] <id> ; <payload> BEL`.
For Zedra, OSC should feed generic terminal metadata first. Agent identity, icons, notifications, command progress, and adapter routing should be downstream classifiers over that metadata, not special cases inside the OSC parser.

Agent icon assets are intentionally duplicated for the two renderers that need them. GPUI uses SVGs from `crates/zedra/assets/icons`, while native iOS selection sheets use matching template SVG image sets in `ios/Zedra/Assets.xcassets`. When adding or renaming an agent icon, update both locations in the same change so terminal cards and native Add to Chat pickers stay in sync across light and dark appearances.

Sources: [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html), [Ghostty VT reference](https://ghostty.org/docs/vt/reference), [VS Code shell integration](https://code.visualstudio.com/docs/terminal/shell-integration#_supported-escape-sequences), [iTerm2 escape codes](https://iterm2.com/documentation-escape-codes.html), [iTerm2 inline images](https://iterm2.com/documentation-images.html), and [Kitty desktop notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/).

Status legend: ✅ parsed = `zedra-osc` decodes it into a typed `OscEvent`. ✅ wired = `WorkspaceTerminal` handler routes the event into `TerminalState`. ➖ parsed but unwired = decoded but ignored by the app (no consumer yet).

| Sequence | Function | Zedra metadata use | Current Zedra status |
|---|---|---|---|
| `OSC 0 ; <text>` | Set icon name and window title. Many shells use this as the broad "title" update. | Store latest title text; optionally remember source as `osc0`. | ✅ parsed + wired — `OscEvent::Title` updates `TerminalMeta.title` and feeds workspace header subtitle |
| `OSC 1 ; <text>` | Set icon name only. In our observed shells, title hooks emit the command name here at launch, then a path-like icon name when the prompt returns. | Primary app-side agent identity signal. A supported agent name latches `active_agent_icon`; a non-agent icon name clears it. | ✅ parsed + wired — `OscEvent::IconName` drives primary `active_agent_icon` detection |
| `OSC 2 ; <text>` | Set window title. This is the common title signal used by shells and TUIs. | Store latest title text; source should be separate from derived agent identity. | ✅ parsed + wired — same path as OSC 0 |
| `OSC 3 ; <prop=value>` | xterm top-level window property. Rare outside X11-style terminals. | Usually ignore; do not sync unless a concrete use case appears. | ❌ missing |
| `OSC 4 ; <index> ; <color>` | Query/change ANSI palette entries. | Renderer/theme state, not app metadata. Avoid syncing as terminal identity. | ❌ missing |
| `OSC 5 ; <index> ; <color>` | Query/change special colors. | Renderer/theme state only. | ❌ missing |
| `OSC 7 ; <file-uri>` | Report terminal current working directory, usually from shell integration. | Store normalized cwd plus original URI/source. | ✅ parsed + wired — `OscEvent::Cwd` updates `TerminalMeta.cwd` |
| `OSC 8 ; <params> ; <uri>` | Begin/end explicit hyperlinks. Empty params/URI ends the current hyperlink. | Store in terminal grid/link spans, not global terminal metadata. Useful for file/URL preview and tap handling. | ✅ handled by terminal grid via alacritty hyperlink state; not emitted as `zedra-osc` metadata |
| `OSC 9 ; <message>` | Desktop notification, originally iTerm-style and now supported by several terminals. | Store bounded notification event with terminal id, timestamp, body, and source. | ➖ parsed (`OscEvent::Notification` with `NotificationSource::Osc9`) — no consumer yet |
| `OSC 9 ; 4 ; <state> ; <value>` | ConEmu/Ghostty progress report. States commonly map to inactive, in-progress, error, indeterminate, paused. | Store transient progress metadata with timeout/expiry; treat as advisory because programs may exit without clearing it. | ➖ parsed (`OscEvent::Progress`) — no consumer yet |
| `OSC 10..19 ; <color>` | Query/change dynamic colors: foreground, background, cursor, pointer, highlight variants. | Renderer/theme state only. | ❌ missing |
| `OSC 22 ; <shape>` | Pointer shape in terminals that support it. | Renderer/input affordance only, low priority for mobile. | ❌ missing |
| `OSC 52 ; <clipboard> ; <base64>` | Query/change clipboard or selection data. Security-sensitive and may contain private data. | Do not store raw payload. If supported later, gate behind explicit policy and only keep minimal audit metadata. | ❌ missing |
| `OSC 99 ; <params> : <payload>` | Kitty rich notification protocol with IDs, title/body payloads, close events, actions, and support queries. | Store typed notification fields and IDs; do not expose action execution without policy. | ➖ parsed (`OscEvent::Notification` with `NotificationSource::Osc99`) — no consumer yet |
| `OSC 104 ; <indexes>` | Reset ANSI palette entries. | Renderer/theme state only. | ❌ missing |
| `OSC 105 ; <indexes>` | Reset special colors. | Renderer/theme state only. | ❌ missing |
| `OSC 110..119` | Reset dynamic colors such as foreground, background, and cursor color. | Renderer/theme state only. | ❌ missing |
| `OSC 133 ; A/B/C/D` | FinalTerm/iTerm2 shell integration semantic zones: prompt start, command start, command executed, command finished with optional exit code. | Store shell state, command lifecycle timestamps, last exit code. Command text can be inferred from screen range but is less reliable than OSC 633 E. | ✅ parsed + wired — `PromptReady`/`CommandStart`/`CommandEnd { exit_code }` drive `TerminalMeta.shell_state` and `last_exit_code` |
| `OSC 633 ; A/B/C/D/E/P` | VS Code shell integration. `A/B/C/D` mark prompt and command lifecycle, `E` provides escaped command line, `P` sets properties such as cwd. | Store command lifecycle, exact command line, cwd, nonce/provenance, and rich-detection capability. Strong generic source for "what is running". | ✅ parsed; A–D + E wired (`E` populates `TerminalMeta.current_command`, cleared on idle); `P` (`OscEvent::ShellProperty`) parsed but unwired |
| `OSC 777 ; notify ; <title> ; <body>` | rxvt/urxvt-style simple notification, also used by some modern terminal tools. | Store notification title/body with source. | ➖ parsed (`OscEvent::Notification` with `NotificationSource::Osc777`) — no consumer yet |
| `OSC 1337 ; File=...` | iTerm2 inline image/file-transfer protocol. | Renderer/media state; may be useful for future inline previews, but payload can be large. Do not put raw file data in app metadata. | ❌ missing |
| `OSC 1337 ; RemoteHost=...` | iTerm2 shell integration remote user/host. | Optional remote host metadata; likely privacy-sensitive, avoid syncing unless needed. | ➖ parsed (`OscEvent::RemoteHost`) — no consumer yet |
| `OSC 1337 ; CurrentDir=...` | iTerm2 shell integration cwd report; OSC 7 is the preferred generic equivalent. | Store cwd if OSC 7 is absent. | ✅ parsed + wired — emits `OscEvent::Cwd`, same path as OSC 7 |
| `OSC 1337 ; SetUserVar=...` | iTerm2 user variable, base64-encoded. | Store only allowlisted keys if a feature needs them; otherwise ignore. | ➖ parsed (`OscEvent::UserVar`, raw base64) — no consumer yet |
| `OSC 1337 ; ShellIntegrationVersion=...` | iTerm2 shell integration version and shell name. | Store shell integration capability/version metadata. | ➖ parsed (`OscEvent::ShellIntegrationVersion` + `OscEvent::ShellName`) — no consumer yet |
| `OSC 1337 ; Block/UpdateBlock/Button/...` | iTerm2 blocks, folding, copy/custom buttons, and other UI extensions. | Future renderer/UI features; not core terminal identity. Buttons/actions need policy before execution. | ❌ missing |

Implementation rule of thumb:

- Parse known OSC sequences into typed events in `zedra-osc`.
- Store normalized terminal facts in `TerminalMeta`, with source/provenance such as `osc2`, `osc7`, `osc133`, or `osc633`.
- Keep payloads bounded. Do not persist arbitrary unknown OSC strings, clipboard contents, inline image data, or untrusted action commands.
- Let AI-agent detection read terminal metadata as a classifier. Today `OSC 1` is the primary app-side identity signal because it is the signal emitted by the shells we run in practice. `OSC 633;E` command lines remain useful supporting metadata and a fallback for terminals that do not emit icon names.

OSC 633;E is still useful for command identity because it captures the exact shell-interpreted command line before execution. That makes it useful for any program, not just AI agents, but Zedra does not currently depend on it for the primary AI-agent icon path.

---

## opencode features

- HTTP + SSE transport; TUI is the primary client, VS Code extension just wraps it
- Custom commands: Markdown files in `commands/`, support `$ARGUMENTS`, `!bash`, `@file`, subtask spawning
- Tool permissions per command: `"ask"`, `"allow"`, `"deny"`
- Supports ACP — can be driven by Zed or any ACP editor
- MCP tool support alongside built-in tools

---

## Current implementation status

Wired through `zedra-osc` → `zedra-terminal` (`TerminalEvent::OscEvent`) → `WorkspaceTerminal` → `TerminalState`:

- **TerminalMeta** carries `title`, `cwd`, `shell_state` (Unknown/Idle/Running), `last_exit_code`, `current_command`, `active_agent_kind`, and the command-scoped `active_agent_icon`. Keyed by terminal id, owned by the workspace-level `TerminalState` entity.
- **Workspace header** (`crates/zedra/src/workspace.rs`) reads the active terminal's `TerminalMeta` and renders the title (PS1 prefix stripped) below the project name. Agent icons remain on terminal cards and related terminal surfaces, not in the header subtitle.
- **Agent classifier** (`crates/zedra/src/agent.rs::detect`) recognises the confirmed terminal AI CLIs we have icon support for: Amp, Claude Code, Cline, Codex, Cursor Agent, Gemini CLI, GitHub Copilot, Goose, Hermes Agent, Junie, Kilo Code, OpenClaw, opencode, OpenHands, Pi, Qoder, Qwen Code, Trae Agent, and Zencoder. Terminal title text is display metadata only and is not used for agent identity.
- **Add to Chat** supports every detected non-shell agent. Claude Code receives an `@file#Lx-Ly` mention; all other agents receive a bracketed-paste fenced context block with the source range, without automatic submit.
- **Shell lifecycle**: OSC 1 icon name drives the primary active-agent state: supported agent names set `active_agent_kind` / `active_agent_icon`, while non-agent prompt/path icon names clear them. OSC 133/633 lifecycle still records idle/running state, exit code, and command-line metadata where available, but it does not override OSC 1 once icon-name identity has appeared for a terminal.
- **Host reattach replay**: `zedra-host` caches the same OSC-derived title, icon name, cwd, command line, shell state, and last exit code while reading PTY output. `SyncSessionResult.terminals` includes the cached OSC 1 icon name for the host snapshot, `Workspace` seeds `TerminalState` from that snapshot, and `TermAttach` also emits a synthetic `seq=0` OSC preamble before backlog replay so normal terminal-event consumers restore metadata even when the original OSC bytes are no longer in the backlog.

Not yet wired (parsed only, no consumer): notifications (OSC 9/99/777), progress (OSC 9;4), shell integration version, remote host, user vars, OSC 633;P key/value properties. These will be plumbed when the corresponding UI surfaces are built.

## Integration options for zedra-host

**Option A — Fake IDE (Claude Code only)**
Write `~/.claude/ide/[port].lock` from zedra-host. Run WebSocket MCP server. Implement the 12 tools. Intercept `openDiff` — forward `{old_file_path, new_file_path, new_file_contents}` over RPC to mobile. Respond `FILE_SAVED`/`DIFF_REJECTED` based on user action. Full spec known, implementable today.

Beyond diffs, also enables: live selection sync to mobile (`selection_changed`), diagnostics display (`getDiagnostics`), file navigation (`openFile`), workspace awareness (`getWorkspaceFolders`).

**Option B — ACP server (multi-agent)**
zedra-host implements ACP server. Spawns coding agent as subprocess, wires stdin/stdout. Covers opencode, Gemini CLI, Goose, and future ACP agents. Claude Code does not support ACP yet.

**Option C — PTY JSON stream scanning (Claude Code, no setup)**
Scan PTY bytes for `--output-format stream-json` JSON lines alongside OSC scanning. Extract tool_use edits client-side. Fragile (ANSI interleaved) but requires zero user setup.

**Option D — OSC 633 + semantic blocks** *(parser landed; semantic blocks pending)*
633 parsing is live in `zedra-osc`: command text (`633;E`) and prompt/command/exit lifecycle (`633;A/B/C/D`) are decoded and the lifecycle is wired into `TerminalMeta`. Capturing the per-command output byte range (between `C` and `D`) so Zedra can render Warp-style command blocks is the next step — agent-agnostic, works for any tool.

**Option E — OSC 9 notification capture** *(parser landed; surface pending)*
OSC 9 / 99 / 777 are decoded into `OscEvent::Notification { title, body, source }`. Surfacing them as mobile push or in-app toast still needs a notification consumer + platform bridge wiring.
