# Zedra Protocol Specs

Canonical specification for Zedra's wire protocol, RPC contracts, and protocol-layer change process.

- Primary implementation: `crates/zedra-rpc/src/proto.rs`
- Host dispatcher: `crates/zedra-host/src/rpc_daemon.rs`
- Client integration: `crates/zedra-session/src/handle.rs`

When this document conflicts with code, treat code as current behavior and immediately update this file.

---

## 1) Scope and Source of Truth

This document defines protocol and RPC conventions for:

- Message enum and transport channels (`ZedraProto`)
- All request/response structures
- Host-initiated event contracts (`HostEvent`)
- Stream semantics (terminal bidirectional stream, subscribe stream)
- Compatibility/versioning rules for evolving protocol safely

The protocol layer includes:

- `crates/zedra-rpc/src/proto.rs`
- `crates/zedra-host/src/rpc_daemon.rs`
- `crates/zedra-session/src/handle.rs`
- Any code that serializes/deserializes protocol messages

---

## 2) Transport and Encoding

### 2.1 Transport

- Network transport: iroh QUIC.
- RPC framing: `irpc` over iroh streams.
- ALPN: `zedra/rpc/1` (see `ZEDRA_ALPN`).

### 2.2 Serialization

- Binary serialization: `postcard`.
- No JSON/base64 at protocol message layer.
- Terminal data uses raw bytes (`serde_bytes`) to avoid text encoding overhead.

### 2.3 Message Envelope

- All RPC operations are declared in `ZedraProto` and expanded by `#[rpc_requests]`.
- Each operation explicitly declares channel shape:
  - Unary request/response: oneshot sender
  - Server stream: mpsc sender from host to client
  - Bidirectional stream: client rx + host tx
- `ZedraProto` variant order is append-only. Never insert/reorder existing variants.

---

## 3) Protocol Design Conventions

### 3.1 Naming

- Requests end with `Req`.
- Responses end with `Result`.
- Event types use explicit enum variants (`HostEvent::...`).
- Enum variant names are verb-first for RPCs (`FsList`, `GitStatus`, `TermCreate`).

### 3.2 Path Rules

- Client paths are always workspace-relative strings.
- Host resolves and jails paths using canonicalization.
- Absolute paths and traversal escapes are rejected host-side.

### 3.3 Streaming Rules

- `Subscribe` is long-lived server stream for host-initiated events.
- Only one active subscribe stream is expected per session; new subscribe replaces old sender.
- `TermAttach` is long-lived bidirectional stream for PTY I/O.

### 3.4 Event Delivery Semantics

- Host events are best-effort, at-most-once.
- Clients must treat host events as invalidation signals and refresh via canonical RPC reads.
- Observers should coalesce duplicate invalidations when possible.

---

## 4) Authentication and Session Flow

### 4.1 First Pairing

1. `Register(RegisterReq)` with HMAC proof from QR ticket handshake secret.
2. `Connect(ConnectReq)` with `session_token: None` — server always issues a `Challenge` since no token exists yet.
3. Verify `Challenge.host_signature` against stored host `EndpointId`.
4. `AuthProve(AuthProveReq)` with client signature and session attachment.
5. `AuthProveResult::Ok(SyncSessionResult)` — bootstrap data is piggybacked, no separate `SyncSession` needed.
6. Normal RPCs begin after success.

### 4.2 Token Resume (fast path — 1 RTT)

1. `Connect(ConnectReq)` with `session_token: Some(token)` — in-memory token from last successful connect.
2. `ConnectResult::Ok(SyncSessionResult)` — session attached immediately, fresh token issued.
3. Normal RPCs begin after success.

### 4.3 PKI Reconnect (2 RTTs)

If the client has no valid session token (first connection after restart, or token expired):

1. `Connect(ConnectReq)` with `session_token: None`.
2. `ConnectResult::Challenge { nonce, host_signature }` — server embeds the PKI challenge, saving an `Authenticate` RTT.
3. Verify `host_signature` against stored host `EndpointId`.
4. `AuthProve(AuthProveReq)` with client signature.
5. `AuthProveResult::Ok(SyncSessionResult)` — bootstrap data piggybacked.
6. Normal RPCs begin after success.

### 4.4 Session Token Properties

- **In-memory only**: never persisted to disk. Host restart requires PKI reconnect.
- **Single-slot per session**: one token at a time, bound to the currently active client pubkey.
- **Consumed on validation**: token is atomically removed when validated, preventing replay.
- **Rotated on every successful connect**: both `ConnectResult::Ok` and `AuthProveResult::Ok` return a fresh token.

### 4.5 Health

- `Ping(PingReq)` / `PongResult` used for RTT and liveness.

---

## 5) RPC Surface (Current)

## 5.1 Auth

- `Register(RegisterReq) -> RegisterResult`
- `Connect(ConnectReq) -> ConnectResult`
- `AuthProve(AuthProveReq) -> AuthProveResult`

## 5.2 Health

- `Ping(PingReq) -> PongResult`

## 5.3 Session

- `SyncSession(SyncSessionReq) -> SyncSessionResult` (mid-session only; bootstrap payload is piggybacked on `ConnectResult::Ok` and `AuthProveResult::Ok`)
- `GetSessionInfo(SessionInfoReq) -> SessionInfoResult`
- `ListSessions(SessionListReq) -> SessionListResult`
- `SwitchSession(SessionSwitchReq) -> SessionSwitchResult`
- `SubscribeHostInfo(SubscribeHostInfoReq) -> stream HostInfoSnapshot`

## 5.4 Filesystem

- `FsList(FsListReq) -> FsListResult`
- `FsRead(FsReadReq) -> FsReadResult`
- `FsWrite(FsWriteReq) -> FsWriteResult`
- `FsStat(FsStatReq) -> FsStatResult`
- `FsWatch(FsWatchReq) -> FsWatchResult`
- `FsUnwatch(FsUnwatchReq) -> FsUnwatchResult`

### Error convention

Most result structs carry `error: Option<String>`. When set, the operation failed and the host has already logged the cause. Client rules:

- Treat `error: Some(msg)` as a terminal failure for that request.
- All other fields are zero-valued when `error` is set (empty string, empty vec, `false`, `0`, `None`).
- Never silently ignore a set `error` — propagate as `Err` or show in UI.

Result types that carry `error`:
`FsListResult`, `FsReadResult`, `FsStatResult`, `SessionSwitchResult`, `TermCreateResult`,
`GitStatusResult`, `GitDiffResult`, `GitLogResult`, `GitCommitResult`, `GitStageResult`,
`GitUnstageResult`, `GitBranchesResult`, `LspDiagnosticsResult`.

Types that do **not** carry `error` (use dedicated status fields or enum variants instead):
`FsWriteResult` (`ok: bool`), `GitCheckoutResult` (`ok: bool`), `FsWatchResult`/`FsUnwatchResult` (enum).

### FsRead additional fields

- `content`: file contents (empty on error or when `too_large`)
- `too_large`: true when file exceeds the 500 KB limit

### FsList paging conventions

- `offset` is zero-based index into stable listing order returned by host.
- `limit` is clamped by host to `FS_LIST_DEFAULT_LIMIT` when necessary.
- `has_more` indicates additional entries exist after this page.

### FsWatch/FsUnwatch result enums

- `FsWatchResult`:
  - `Ok`
  - `InvalidPath`
  - `RateLimited`
  - `QuotaExceeded`
  - `Unsupported` (client-local fallback when host does not support observer RPCs)
- `FsUnwatchResult`:
  - `Ok`
  - `InvalidPath`
  - `RateLimited`
  - `NotWatched`
  - `Unsupported` (client-local fallback when host does not support observer RPCs)

## 5.5 Terminals

- `TermCreate(TermCreateReq) -> TermCreateResult`
- `TermAttach(TermAttachReq) <-> TermInput/TermOutput` (bidirectional)
- `TermResize(TermResizeReq) -> TermResizeResult`
- `TermClose(TermCloseReq) -> TermCloseResult`
- `TermList(TermListReq) -> TermListResult`
- `SyncSessionResult.terminals -> Vec<TerminalSyncEntry>`
- Terminal ids are opaque host-generated UUID strings.

### TermAttach conventions

- Client passes `last_seq` to request backlog replay.
- Host may replay missed output before live stream.
- Host may send a synthetic metadata preamble as `TermOutput { seq: 0, ... }` before backlog replay. Clients must process the bytes as normal PTY output but must not use `seq=0` for backlog sequence tracking.
- The synthetic preamble replays cached OSC metadata that may have fallen out of the backlog, including title, icon name, cwd, shell command line, command start/idle state, and last exit code.
- Output `seq` is monotonic per session backlog stream and used for gap detection.
- `TerminalSyncEntry.last_seq` is the host's latest backlog sequence observed for that terminal at sync time.
- `TerminalSyncEntry.title`, `TerminalSyncEntry.cwd`, and `TerminalSyncEntry.icon_name` are the host's latest cached terminal metadata at sync time. `TermAttach` still replays the same metadata as PTY bytes so normal terminal-event consumers are seeded through one path.
- Clients should keep local terminal tabs keyed by terminal id and use `last_seq` to seed reconnect `TermAttach` calls.

### SyncSession conventions

- `SyncSession` is the canonical bootstrap payload after a successful PKI attach.
- Host rotates and returns a fresh `reconnect_token` on every successful `SyncSession` and `Reconnect`.
- `session_id` in `SyncSessionResult` is authoritative and must replace any stale client-side session id.
- `SyncSessionResult.terminals` is the authoritative server-side terminal set at bootstrap time.
- `ReconnectReq.reconnect_token` is opaque, host-issued, session-bound, and client-bound.
- Reconnect tokens are currently ephemeral host memory only; host restart may invalidate them and force PKI fallback.

## 5.6 Git

- `GitStatus(GitStatusReq) -> GitStatusResult`
- `GitDiff(GitDiffReq) -> GitDiffResult`
- `GitLog(GitLogReq) -> GitLogResult`
- `GitCommit(GitCommitReq) -> GitCommitResult`
- `GitStage(GitStageReq) -> GitStageResult`
- `GitUnstage(GitUnstageReq) -> GitUnstageResult`
- `GitBranches(GitBranchesReq) -> GitBranchesResult`
- `GitCheckout(GitCheckoutReq) -> GitCheckoutResult`

### Git error handling

All Git result types carry `error: Option<String>`. Host sends error when git repo cannot be opened or the operation fails. Client `git_*` handle methods propagate these as `Err`.

### Git status conventions

- `GitStatusEntry` reports index and working-tree state independently via `staged_status` and `unstaged_status`.
- A file may appear in both UI sections when both fields are present (for example, partially staged edits).
- Status strings use lowercase semantic names such as `modified`, `added`, `deleted`, `renamed`, `untracked`, and `conflicted`.
- `GitStage` stages the provided paths with `git add -- <paths>`.
- `GitUnstage` removes the provided paths from the index while preserving working tree contents.

## 5.7 AI and LSP

- `AiPrompt(AiPromptReq) -> AiPromptResult`
- `LspDiagnostics(LspDiagnosticsReq) -> LspDiagnosticsResult`
- `LspHover(LspHoverReq) -> LspHoverResult`

---

## 6) Host Events

Current host event variants:

- `TerminalCreated { id, launch_cmd }`
- `GitChanged`
- `FsChanged { path }`

Client rules:

- `TerminalCreated`: attach/open terminal view if relevant.
- `GitChanged`: invalidate cached git state and refresh when appropriate.
- `FsChanged { path }`: invalidate cached file tree for the watched path and reload affected expanded nodes.

---

## 6.1 Host Info Subscription

`SubscribeHostInfo` is a separate server-streaming subscription for host resource display. It does not reuse `HostEvent`, because resource snapshots are periodic state rather than invalidation events.

Current sampling behavior:

- Host sends the first `HostInfoSnapshot` after CPU counters are primed.
- Host sends subsequent snapshots every 5 seconds.
- Stream ends when the client disconnects or stops reading.
- Battery list may be empty when the platform does not expose battery data.

Current snapshot fields:

- `captured_at_ms`
- `cpu_usage_percent`
- `cpu_count`
- `memory_used_bytes`
- `memory_total_bytes`
- `swap_used_bytes`
- `swap_total_bytes`
- `system_uptime_secs`
- `batteries: Vec<HostBatteryInfo>`

---

## 7) Compatibility Policy

### 7.1 General

- Avoid breaking wire compatibility unless absolutely required.
- Prefer additive evolution:
  - Add new enum variants
  - Add new RPC calls instead of repurposing old semantics
- With postcard-encoded structs, prefer new request/response types over widening existing structs when mixed-version decoding matters.

### 7.2 Breaking Changes

If a breaking change is unavoidable:

1. Document it in this file under "Protocol Changelog".
2. Update ALPN/version strategy (`zedra/rpc/N`) if cross-version interop is impacted.
3. Ship host/client changes together.

### 7.3 Unknown Variants

- New variants may fail on old binaries; do not assume forwards compatibility across arbitrary versions.
- Keep deployment pairs synchronized when introducing enum variants.

---

## 8) Error Handling Conventions

- For authorization/validation failures, return typed protocol results when possible.
- For transport-level failure paths, dropping sender/stream is acceptable where already established.
- Client should treat RPC failure as recoverable unless mapped to fatal auth/session errors.

Recommended rule:

- Prefer explicit `Result` payload structs/enums over implicit stream closure for new APIs.

---

## 9) Performance and Reliability Conventions

- Keep protocol payloads compact and typed.
- Use streaming for unbounded/high-volume channels (terminal output, host events).
- Coalesce high-frequency invalidation events at source when possible.
- Never block async executors with shell/fs heavy operations; use `spawn_blocking`.

### 9.1 Observer Hardening (Abuse Protection)

For filesystem observer control RPCs (`FsWatch`, `FsUnwatch`):

- Per-session watch quota: max `128` watched paths.
- Per-session rate limit: token bucket, `10 req/s` refill, `20` burst.
- Invalid/absolute/traversal paths are rejected by normalization.

For host event emission:

- Event emit is non-blocking (`try_send`) to avoid observer stalls.
- Bounded channel applies backpressure by dropping when full.
- Drops are counted via metrics counters:
  - `observer_events_dropped_full`
  - `observer_events_dropped_no_subscriber`

Operational metrics/logging:

- Periodic observer metrics log (current watched paths, sent/dropped counters, rate-limit/quota rejections).
- Explicit warn logs on watch RPC rate-limit and quota rejections.

---

## 10) Protocol Change Checklist (Required)

Any protocol-layer change must include all applicable steps:

1. Update `crates/zedra-rpc/src/proto.rs`.
2. Update host dispatch in `crates/zedra-host/src/rpc_daemon.rs`.
3. Update client handlers in `crates/zedra-session/src/handle.rs`.
4. Add/adjust UI consumers if behavior is user-visible.
5. Update this document (`docs/PROTOCOL_SPECS.md`).
6. Add or update tests:
   - Serialization roundtrip tests in `zedra-rpc`
   - Host/client behavior tests when feasible
7. Mention protocol changes explicitly in PR description.

---

## 11) Protocol Changelog

### 2026-04-27

- Extended the `TermAttach` `seq=0` synthetic metadata preamble to replay cached OSC command lifecycle metadata, including command line, running/idle state, and last exit code, in addition to title, icon name, and cwd.
- Added cached `icon_name` to `TerminalSyncEntry` so `SyncSession` carries the host's latest OSC 1 value alongside title and cwd.
- Clients must continue processing `seq=0` preamble bytes as PTY output while excluding `seq=0` from terminal backlog sequence tracking.

### 2026-04-26

- Added host resource subscription:
  - `SubscribeHostInfo(SubscribeHostInfoReq) -> stream HostInfoSnapshot`
- Added host resource snapshot types:
  - `HostInfoSnapshot`
  - `HostBatteryInfo`
  - `HostBatteryState`
- Client displays the latest snapshot from `WorkspaceState` in the session panel.

### 2026-03-19

- Added filesystem observer RPCs:
  - `FsWatch(FsWatchReq) -> FsWatchResult`
  - `FsUnwatch(FsUnwatchReq) -> FsUnwatchResult`
- Added host invalidation events:
  - `HostEvent::GitChanged`
  - `HostEvent::FsChanged { path }`
- Added observer hardening:
  - per-session watch quota (`128`)
  - per-session watch RPC token-bucket rate limit (`10/s`, burst `20`)
  - non-blocking event emission (`try_send`) with drop metrics
- Switched watch control RPC responses from `ok: bool` to explicit enums:
  - `FsWatchResult::{Ok, InvalidPath, RateLimited, QuotaExceeded, Unsupported}`
  - `FsUnwatchResult::{Ok, InvalidPath, RateLimited, NotWatched, Unsupported}`
- Added compatibility guard:
  - observer RPCs auto-disable client-side on protocol decode/variant mismatch,
    returning `Unsupported` instead of repeatedly retrying incompatible calls.

### 2026-03-28

- Removed `Authenticate(AuthReq) -> AuthChallengeResult` RPC.
- Removed `Reconnect(ReconnectReq) -> ReconnectResult` RPC.
- Added unified `Connect(ConnectReq) -> ConnectResult` replacing both:
  - `ConnectReq` carries optional `session_token` for 1-RTT fast path.
  - `ConnectResult::Ok(SyncSessionResult)` — token valid; session attached immediately.
  - `ConnectResult::Challenge { nonce, host_signature }` — no valid token; host embeds PKI challenge inline, saving a separate `Authenticate` RTT.
- `AuthProveResult::Ok` now carries `SyncSessionResult` (piggybacked bootstrap); separate `SyncSession` call no longer needed at connect time.
- `SyncSessionResult.reconnect_token` renamed to `session_token`.
- Session token storage changed to single-slot (`Option<([u8; 32], SessionToken)>`) per session — only one token is valid at a time, consumed atomically on validation. No TTL.
- ALPN bumped to `zedra/rpc/2` (breaking change).

### 2026-03-26

- Added fast reconnect bootstrap RPC:
  - `Reconnect(ReconnectReq) -> ReconnectResult`
- Added canonical session bootstrap RPC:
  - `SyncSession(SyncSessionReq) -> SyncSessionResult`
- Added `TerminalSyncEntry` to return resumable terminal ids + latest backlog sequence + cached title/CWD.
- Updated connection lifecycle:
  - first pairing and PKI fallback now end with `SyncSession`
  - reconnect may skip `Authenticate/AuthProve` entirely when a valid reconnect token is present
- Added rotating host-issued reconnect tokens bound to `(session_id, client_pubkey)`.
