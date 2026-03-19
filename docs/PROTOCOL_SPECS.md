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
2. `Authenticate(AuthReq)` to request challenge nonce + host signature.
3. `AuthProve(AuthProveReq)` with client signature and session attachment.
4. Normal RPCs begin after success.

### 4.2 Reconnect

1. `Authenticate(AuthReq)`
2. `AuthProve(AuthProveReq)`
3. Resume normal RPCs and terminal streams.

### 4.3 Health

- `Ping(PingReq)` / `PongResult` used for RTT and liveness.

---

## 5) RPC Surface (Current)

## 5.1 Auth

- `Register(RegisterReq) -> RegisterResult`
- `Authenticate(AuthReq) -> AuthChallengeResult`
- `AuthProve(AuthProveReq) -> AuthProveResult`

## 5.2 Health

- `Ping(PingReq) -> PongResult`

## 5.3 Session

- `GetSessionInfo(SessionInfoReq) -> SessionInfoResult`
- `ListSessions(SessionListReq) -> SessionListResult`
- `SwitchSession(SessionSwitchReq) -> SessionSwitchResult`

## 5.4 Filesystem

- `FsList(FsListReq) -> FsListResult`
- `FsRead(FsReadReq) -> FsReadResult`
- `FsWrite(FsWriteReq) -> FsWriteResult`
- `FsStat(FsStatReq) -> FsStatResult`
- `FsWatch(FsWatchReq) -> FsWatchResult`
- `FsUnwatch(FsUnwatchReq) -> FsUnwatchResult`

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

### TermAttach conventions

- Client passes `last_seq` to request backlog replay.
- Host may replay missed output before live stream.
- Output `seq` is monotonic per session backlog stream and used for gap detection.

## 5.6 Git

- `GitStatus(GitStatusReq) -> GitStatusResult`
- `GitDiff(GitDiffReq) -> GitDiffResult`
- `GitLog(GitLogReq) -> GitLogResult`
- `GitCommit(GitCommitReq) -> GitCommitResult`
- `GitStage(GitStageReq) -> GitStageResult`
- `GitUnstage(GitUnstageReq) -> GitUnstageResult`
- `GitBranches(GitBranchesReq) -> GitBranchesResult`
- `GitCheckout(GitCheckoutReq) -> GitCheckoutResult`

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

## 7) Compatibility Policy

### 7.1 General

- Avoid breaking wire compatibility unless absolutely required.
- Prefer additive evolution:
  - Add new enum variants
  - Add new optional fields
  - Add new RPC calls instead of repurposing old semantics

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

