# Terminal Session Persistence

How terminal sessions survive network reconnects and app restarts, and how a fresh client can resume running terminals without prior state.

## Problem Statement

The host daemon keeps PTY processes running after a client disconnects. But when the client reconnects, it needs to:

1. **Discover** which terminals exist on the server
2. **Restore** the visual state of each terminal (screen contents, colors, cursor, alternate screen)
3. **Resume** live I/O (keystrokes and output streaming)

Currently only (3) works reliably within a single app session. Discovery and state restoration are incomplete.

## Current Architecture

### Server Side (zedra-host)

Each `ServerSession` in the `SessionRegistry` owns a map of `TermSession` entries:

```
ServerSession
├── terminals: HashMap<String, TermSession>
│   ├── "term-1" → { master, writer, output_sender, vterm }
│   └── "term-2" → { master, writer, output_sender, vterm }
├── notification_backlog: VecDeque<BacklogEntry>  (raw bytes, capped at 1000)
└── next_notif_seq: u64
```

**Key files:**
- `session_registry.rs` — `ServerSession`, `TermSession`, backlog storage
- `rpc_daemon.rs` — `TermCreate` (spawns PTY + reader), `TermAttach` (bidi streaming + backlog replay)
- `pty.rs` — `ShellSession::spawn()` wrapper around portable-pty

**What happens on disconnect:**
1. `handle_connection()` returns → `clear_output_senders()` sets all `output_sender` to `None`
2. PTY reader threads keep running, storing output in `notification_backlog`
3. Session stays in registry for the grace period (~5 minutes)

**What happens on TermAttach reconnect:**
1. Server finds the `TermSession` by ID
2. Replays backlog entries where `seq > last_seq` through the bidi stream
3. Creates a new tokio channel, sets it as the terminal's `output_sender`
4. Bridges client input → PTY stdin, PTY output → client stream

### Client Side (zedra-session)

Process-global state survives `RemoteSession` rebuilds:

```
PERSISTENT_TERMINAL_OUTPUTS   → HashMap<terminal_id, OutputBuffer>
PERSISTENT_TERMINAL_IDS       → Vec<String>
PERSISTENT_ACTIVE_TERMINAL    → Option<String>
SESSION_CREDENTIALS           → (session_id, auth_token)
ENDPOINT_ADDR                 → iroh::EndpointAddr
RECONNECT_ATTEMPT             → u32
LAST_NOTIF_SEQ                → u64
```

**Key file:** `zedra-session/src/lib.rs`

**Reconnect flow (within same app session):**
1. QUIC connection closes → `spawn_reconnect()` fires
2. Exponential backoff (1s → 30s cap, 20 attempts max)
3. Creates new `RemoteSession::connect_with_iroh(stored_addr)`
4. Sends `ResumeOrCreate` with stored `(session_id, auth_token)`
5. Calls `reattach_terminals()` — iterates `PERSISTENT_TERMINAL_IDS`, calls `attach_terminal(id, last_seq)` for each
6. UI terminal views still exist in `app.rs::terminal_views` — they pick up output on next render

### Protocol (zedra-rpc/proto.rs)

```
ResumeOrCreate  → (session_id?, auth_token, last_notif_seq) → ResumeResult { session_id, resumed }
TermCreate      → (cols, rows)                               → TermCreateResult { id }
TermAttach      → (id, last_seq) [bidi: TermInput/TermOutput]
TermList        → ()                                          → TermListResult { terminals: [{ id }] }
TermResize      → (id, cols, rows)                            → TermResizeResult { ok }
TermClose       → (id)                                        → TermCloseResult { ok }
```

### UI (zedra/src/app.rs)

- `terminal_views: Vec<(String, Entity<TerminalView>)>` — terminal ID → GPUI view
- On first connect: always calls `terminal_create()`, never checks for existing terminals
- On disconnect: clears all views and returns to Home screen

## Gaps

### Gap 1: Fresh client cannot discover existing terminals

After `ResumeOrCreate`, the client never calls `TermList` to discover server-side terminals. A fresh client (app restart, new device) always creates a new terminal instead of resuming existing ones.

**Where:** `app.rs:556-566` — always calls `terminal_create()` after connect.

**Impact:** Old PTYs become orphaned. User loses running sessions.

### Gap 2: No terminal screen state restoration

The backlog stores raw PTY output bytes (capped at 1000 entries, ~8 MB). For a long-running session (e.g., `claude code` running for hours), the backlog overflows. Even if it didn't, replaying megabytes of raw output is slow and can produce garbled results because the ANSI state machine's initial state is lost.

**Where:** `session_registry.rs` backlog, `rpc_daemon.rs` TermAttach handler.

**Impact:** Reconnecting client sees blank or garbled terminal instead of the current screen.

### Gap 3: No on-disk credential storage

`SESSION_CREDENTIALS` is in-memory only. App restart resets it. The client cannot resume the same session after restart.

**Where:** `zedra-session/src/lib.rs` — `session_credentials_slot()` is a process-global static.

**Impact:** App restart = new session = new terminals. Old session's terminals are orphaned until grace period expires.

### Gap 4: TermListEntry lacks metadata

`TermListEntry` only contains `id`. No terminal dimensions, title, or working directory. A fresh client can't build meaningful UI for discovered terminals.

**Where:** `proto.rs:293-296`.

### Gap 5: Reconnect discards terminal_list results

`spawn_reconnect()` calls `terminal_list()` but only logs the result. It doesn't reconcile client-side state with server-side reality.

**Where:** `zedra-session/src/lib.rs:1053-1063`.

### Gap 6: ResumeResult.resumed not acted upon

The client doesn't branch on whether the session was resumed or newly created. It should: if resumed, list and attach existing terminals; if new, create one.

**Where:** `app.rs:556-566` — unconditionally creates a terminal.

## Solution: Server-Side Virtual Terminal (`vt100`)

### Why vt100

The [`vt100`](https://crates.io/crates/vt100) crate (v0.16) is purpose-built for "programs like screen or tmux." It provides:

- `Parser::new(rows, cols, scrollback)` — incremental PTY byte parser
- `parser.process(&bytes)` — feed raw bytes, handles partial escape sequences
- `screen().state_formatted()` — dump full screen as ANSI sequences (~2-10 KB)
- Alternate screen buffer support (vim, htop, claude code TUI)
- Cursor position, colors, input modes (bracketed paste, mouse protocol)
- `screen().contents_diff(&prev)` — minimal diff between states

### Comparison with alternatives

| Approach | Correctness | Wire Size | Restore Speed | Alternate Screen |
|----------|-------------|-----------|---------------|------------------|
| Raw backlog replay (current) | Lossy (cap) | Up to 8 MB | Slow | Depends on cap |
| Increase backlog cap | Still lossy | Larger | Slower | Better odds |
| `vt100::state_formatted()` | Perfect | 2-10 KB | Instant | Yes |
| `alacritty_terminal` server-side | Perfect | Custom code needed | Instant | Yes |

`vt100` wins because it has `state_formatted()` as a single method call. `alacritty_terminal` would require writing a custom `grid_to_ansi()` serializer (~200-400 lines).

### Screen dump format

`state_formatted()` produces minimal ANSI sequences:

```
ESC[?1049h            (if alternate screen active)
ESC[?25l              (if cursor hidden)
ESC[H                 (home cursor)
ESC[2J                (clear screen)
<for each row>:
  ESC[row;1H          (move to row start)
  ESC[sgrm            (set colors/attributes)
  <text>              (cell characters)
ESC[row;colH          (restore cursor position)
ESC[?25h              (show cursor)
ESC[?2004h            (if bracketed paste active)
```

**Size:** ~2-10 KB for a typical 24x80 screen (worst case ~38 KB if every cell has different colors).

### How it integrates

```
PTY output bytes
    │
    ├──→ vt100::Parser.process(&bytes)   ← maintains virtual screen
    ├──→ notification_backlog.push()      ← raw bytes for brief disconnects
    └──→ output_sender.send()            ← live streaming to connected client

On reconnect (TermAttach):
    1. state_formatted() → send as first TermOutput message (~5 KB)
    2. Resume live streaming from PTY reader
    3. Client's alacritty_terminal processes the dump → instant screen restore
```

## Implementation Plan

### Phase 1: Server-side vt100 parser

**Files changed:**
- `zedra-host/Cargo.toml` — add `vt100 = "0.16"`
- `session_registry.rs` — add `vterm: Arc<Mutex<vt100::Parser>>` to `TermSession`
- `rpc_daemon.rs` (TermCreate handler) — initialize vterm with terminal dimensions
- `rpc_daemon.rs` (PTY reader loop) — feed bytes into `vterm.process(&data)`
- `rpc_daemon.rs` (TermAttach handler) — send `state_formatted()` as first message
- `rpc_daemon.rs` (TermResize handler) — resize vterm: `screen_mut().set_size(rows, cols)`

### Phase 2: Terminal discovery on connect

**Files changed:**
- `proto.rs` — add `cols`, `rows`, `title` to `TermListEntry`
- `session_registry.rs` — track terminal dimensions; expose terminal metadata
- `rpc_daemon.rs` (TermList handler) — return enriched metadata
- `zedra-session/lib.rs` — after `ResumeOrCreate(resumed=true)`, call `terminal_list()`, register and attach all discovered terminals
- `zedra-session/lib.rs` (reconnect) — use `terminal_list()` to reconcile client vs server state

### Phase 3: UI integration for discovered terminals

**Files changed:**
- `zedra-session/lib.rs` — new `discover_and_attach_terminals()` method that calls `TermList` → registers output buffers → attaches each
- `app.rs` — on connect, if session resumed: create views for server terminals instead of calling `terminal_create()`; expose discovered terminal info to UI
- `terminal_panel.rs` — render discovered terminals with metadata (title, dimensions)

### Phase 4: Credential persistence (optional, cross-restart)

**Files changed:**
- `zedra-session/lib.rs` — serialize `(session_id, auth_token, endpoint_addr)` to file
- `app.rs` or Android bridge — load saved credentials on app start; offer "Reconnect to last session" flow

## Reconnect Flow (After Implementation)

### Scenario A: Brief network drop (within same app session)

```
1. QUIC conn closes → spawn_reconnect() fires
2. connect_with_iroh(stored_addr)
3. ResumeOrCreate(session_id, auth_token) → resumed=true
4. reattach_terminals() iterates PERSISTENT_TERMINAL_IDS
5. For each: TermAttach(id, last_seq)
   → Server sends state_formatted() + any backlog since last_seq
   → Client feeds bytes into existing TerminalView
6. Screen restored instantly
```

### Scenario B: App restart (fresh client, session still alive on server)

```
1. App starts, loads saved credentials from disk
2. connect_with_iroh(saved_addr)
3. ResumeOrCreate(saved_session_id, saved_auth_token) → resumed=true
4. TermList() → ["term-1", "term-2"] with metadata (cols, rows, title)
5. For each terminal:
   a. Create TerminalView in UI with correct dimensions
   b. TermAttach(id, last_seq=0)
   c. Server sends state_formatted() → instant screen restore
   d. Server bridges live PTY output
6. User sees claude code exactly where they left off
```

### Scenario C: Session expired (server restarted or grace period elapsed)

```
1. App starts, loads saved credentials
2. connect_with_iroh(saved_addr)
3. ResumeOrCreate(old_session_id, old_auth_token) → resumed=false (new session)
4. TermList() → [] (no terminals)
5. Client creates new terminal via TermCreate
6. User starts fresh
```

## Backlog vs vt100 Dump: When to Use Each

| Disconnect Duration | Mechanism | Why |
|---------------------|-----------|-----|
| < 5 seconds | Raw backlog replay | Small amount of missed data, fast replay |
| 5s – 5 minutes | `state_formatted()` | Full screen restore, no replay lag |
| > 5 minutes (session expired) | N/A | New session, fresh terminal |

The `state_formatted()` dump is always correct regardless of disconnect duration. The raw backlog is an optimization for brief disconnects where replaying a few recent chunks may be faster than a full screen dump. In practice, always sending `state_formatted()` first is simplest and covers all cases.

## Performance Characteristics

- **vt100 parsing overhead:** Negligible. ANSI parsing runs at hundreds of MB/s. The PTY reader thread already processes every byte; adding `parser.process()` adds microseconds per chunk.
- **Screen dump size:** 2-10 KB typical, 38 KB worst case (24x80, every cell different color).
- **Memory per terminal:** ~100 KB for vt100::Parser with 1000 lines of scrollback.
- **Restore latency:** Single TermOutput message → client processes in <1ms → screen appears immediately.

## References

### Crates

- [`vt100`](https://docs.rs/vt100/latest/vt100/) — server-side terminal emulator; `Parser`, `Screen`, `state_formatted()`
  - [Source (GitHub)](https://github.com/doy/vt100-rust)
  - [Screen API](https://docs.rs/vt100/latest/vt100/struct.Screen.html) — `state_formatted()`, `contents_diff()`, `alternate_screen()`
  - [Parser API](https://docs.rs/vt100/latest/vt100/struct.Parser.html) — `process()`, `screen()`, `screen_mut()`
- [`portable-pty`](https://docs.rs/portable-pty/latest/portable_pty/) — cross-platform PTY (already used in zedra-host)
  - [Source (GitHub)](https://github.com/wezterm/wezterm/tree/main/pty)
- [`alacritty_terminal`](https://docs.rs/alacritty_terminal/latest/alacritty_terminal/) — terminal emulator core (used client-side in zedra-terminal)
  - [Source (GitHub)](https://github.com/alacritty/alacritty/tree/master/alacritty_terminal)
- [`irpc`](https://docs.rs/irpc/latest/irpc/) — typed RPC over iroh QUIC (used for TermAttach bidi streaming)
- [`serde_bytes`](https://docs.rs/serde_bytes/latest/serde_bytes/) — efficient binary serialization for terminal I/O

### Prior Art

- [tmux `capture-pane`](https://github.com/tmux/tmux/blob/master/cmd-capture-pane.c) — tmux's screen capture implementation (C)
- [tmux `grid.c`](https://github.com/tmux/tmux/blob/master/grid.c) — grid-to-SGR serialization (`grid_string_cells_code`)
- [tmux Advanced Use wiki](https://github.com/tmux/tmux/wiki/Advanced-Use) — session persistence model
- [GNU Screen](https://www.gnu.org/software/screen/) — original terminal multiplexer with detach/reattach
- [Zellij](https://github.com/zellij-org/zellij) — Rust terminal workspace; server-side vt100 + client rendering
  - [Zellij performance blog](https://poor.dev/blog/performance/) — bounded channels, backpressure, PTY reader architecture
  - [Building a web-based terminal (Zellij)](https://poor.dev/blog/building-zellij-web-terminal/) — server-side terminal state serialization for remote clients
- [mprocs](https://github.com/pvolok/mprocs) — Rust terminal multiplexer using vt100 fork
- [WezTerm mux-server](https://github.com/wezterm/wezterm/tree/main/wezterm-mux-server) — headless terminal server with codec-based RPC
- [Eternal Terminal (et)](https://github.com/MystenLabs/EternalTerminal) — reconnectable terminal with client-side screen diff

### Terminal Escape Sequences

- [ECMA-48 (ANSI)](https://ecma-international.org/publications-and-standards/standards/ecma-48/) — standard defining SGR, CSI, cursor control
- [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) — comprehensive reference for DEC private modes, alternate screen, mouse protocol
- [Paul Williams' VT parser](https://vt100.net/emu/dec_ansi_parser) — state machine model used by vt100/alacritty VTE
- [Wikipedia: ANSI escape code](https://en.wikipedia.org/wiki/ANSI_escape_code) — overview of SGR, CSI, OSC sequences

### Related Zedra Docs

- [`docs/ARCHITECTURE.md`](./ARCHITECTURE.md) — system overview, crate structure, RPC dispatch, connection flow
- [`docs/RELAY.md`](./RELAY.md) — iroh relay protocol, Cloudflare Worker topology
- [`docs/DEBUGGING.md`](./DEBUGGING.md) — logcat filtering, crash analysis, screenshot verification
