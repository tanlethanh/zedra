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
- ALPN: `zedra/rpc/4` (see `ZEDRA_ALPN` in `crates/zedra-rpc/src/proto.rs`). Bump the trailing integer on any change that alters how existing bytes decode (see §2.4).

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

### 2.4 Schema Evolution and Decode Compatibility

postcard is schema-positional and non-self-describing. There is no field tag,
no enum-variant length prefix, and no way to skip an unknown value. The decoder
reads bytes by position against the type it was compiled with. Two consequences
drive every wire-schema decision:

- An enum is encoded as a bare varint discriminant. A peer that receives a
  discriminant it does not know **cannot skip it** — `postcard::from_bytes`
  fails the **entire message**, not just that field. One unknown enum value in a
  `Vec<T>` response fails the whole vec; one undecodable item in a server stream
  kills the whole stream.
- A struct is a fixed, untagged sequence of fields. Appending a field breaks the
  peer with fewer fields (it runs out of bytes or misreads). `#[serde(default)]`
  and `#[serde(other)]` do **not** rescue postcard — they only work for
  self-describing formats.

A decoder fix cannot reach an already-shipped binary, so compatibility is a
property of the wire schema, not of any per-client negotiation (the host does
not tailor responses per client). Two strategies are available; Zedra currently
relies on the first.

**Current mechanism — ALPN version gate.** `ZEDRA_ALPN` (`zedra/rpc/N`) is the
single compatibility boundary. A host and client connect only when their ALPN
strings match exactly, so within one protocol version the wire schema — every
struct's field set and every enum's variant set — is fixed and known to both
ends. Forward skew (a host emitting a discriminant the client lacks) cannot
happen across an ALPN boundary: the mismatched peer never connects, it does not
connect and then fail to decode. Therefore:

- **Append-only within a version.** Never reorder, remove, retype, or renumber
  an existing variant, field, or enum code. Appending a new enum variant or a
  new struct field changes how existing bytes decode for a peer that lacks it,
  so it is **not** additive under postcard.
- **Bump `ZEDRA_ALPN` on any wire change.** Adding an agent kind, an event kind,
  a struct field, or a request/response shape all alter decoding for an older
  peer. Bump the trailing integer; the older peer then declines to connect
  rather than corrupting a response.

**Target mechanism — forward-compatible encodings (not yet applied; issue
`#140`).** To let the schema grow *without* an ALPN bump, the undecodable-unknown
problem has to be removed at the encoding layer. Candidate techniques:

- **Open enums.** Serialize a growable enum through a self-delimiting primitive
  with an `Unknown` fallback (`#[serde(into = "u32", from = "u32")]` +
  `Unknown(u32)`) so an unrecognized code decodes to `Unknown` instead of
  failing the whole response; consumers filter `Unknown` out of display and
  dispatch. Known codes stay wire-identical to the derived discriminant.
- **Key/value record data.** Carry optional data as `Vec<{label,value}>` entries
  (the `account.fields` pattern) so a struct's field count never changes.
- **Versioned variants.** Add `FooReqV2` / `FooResultV2` rather than mutating a
  shipped struct (the `TermCreateReq` → `TermCreateReqV2` pattern).

These remain the direction for growable enum fields. Agent identity is already
different: the live agent APIs use stable slug strings, so registering a new
agent actor does not change their postcard schema or require an ALPN bump.

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
2. `ConnectResult::Challenge { nonce, host_signature }` — server embeds the PKI challenge, saving a separate challenge request RTT.
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

### 4.6 Deprecated Append-Only Auth Variant

`Authenticate(AuthReq) -> AuthChallengeResult` remains in `ZedraProto` only
because protocol enum order is append-only. It is reserved for wire compatibility
and is not part of the active connection lifecycle. Current clients start auth
with `Register` for first pairing or `Connect` for all other paths; the PKI
challenge is carried by `ConnectResult::Challenge`.

---

## 5) RPC Surface (Current)

## 5.1 Auth

- `Register(RegisterReq) -> RegisterResult`
- `Connect(ConnectReq) -> ConnectResult`
- `AuthProve(AuthProveReq) -> AuthProveResult`
- `Authenticate(AuthReq) -> AuthChallengeResult` (deprecated/reserved append-only variant; do not use in current clients)

## 5.2 Health

- `Ping(PingReq) -> PongResult`

## 5.3 Session

- `SyncSession(SyncSessionReq) -> SyncSessionResult` (mid-session only; bootstrap payload is piggybacked on `ConnectResult::Ok` and `AuthProveResult::Ok`)
- `GetSessionInfo(SessionInfoReq) -> SessionInfoResult`
- `ListSessions(SessionListReq) -> SessionListResult`
- `SwitchSession(SessionSwitchReq) -> SessionSwitchResult` (reserved/unsupported for active workspace switching; current host dispatch stays bound to the originally authenticated session)
- `SubscribeHostInfo(SubscribeHostInfoReq) -> stream HostInfoSnapshot`

## 5.4 Filesystem

- `FsList(FsListReq) -> FsListResult`
- `FsSearch(FsSearchReq) -> FsSearchResult`
- `FsRead(FsReadReq) -> FsReadResult`
- `FsWrite(FsWriteReq) -> FsWriteResult`
- `FsStat(FsStatReq) -> FsStatResult`
- `FsDocsTree(FsDocsTreeReq) -> FsDocsTreeResult`
- `FsWatch(FsWatchReq) -> FsWatchResult`
- `FsUnwatch(FsUnwatchReq) -> FsUnwatchResult`

### Error convention

Most result structs carry `error: Option<String>`. When set, the operation failed and the host has already logged the cause. Client rules:

- Treat `error: Some(msg)` as a terminal failure for that request.
- All other fields are zero-valued when `error` is set (empty string, empty vec, `false`, `0`, `None`).
- Never silently ignore a set `error` — propagate as `Err` or show in UI.

Result types that carry `error: Option<String>`:
`FsListResult`, `FsSearchResult`, `FsReadResult`, `FsStatResult`, `SessionSwitchResult`, `TermCreateResult`,
`GitStatusResult`, `GitDiffResult`, `GitLogResult`, `GitCommitResult`, `GitStageResult`,
`GitUnstageResult`, `GitBranchesResult`, `AgentListResult`, `AgentSessionsResult`,
`AgentResumeResult`, `LspDiagnosticsResult`.

Types that use non-string status fields or enum variants instead:
`FsWriteResult` (`ok: bool`), `GitCheckoutResult` (`ok: bool`), `FsWatchResult`/`FsUnwatchResult` (enum),
`FsDocsTreeResult` (`error: Option<FsDocsTreeError>`).

### FsRead additional fields

- `content`: file contents (empty on error or when `too_large`)
- `too_large`: true when file exceeds the 500 KB limit

### FsList paging conventions

- `offset` is zero-based index into stable listing order returned by host.
- `limit` is clamped by host to `FS_LIST_DEFAULT_LIMIT` when necessary.
- `has_more` indicates additional entries exist after this page.

### FsSearch conventions

- `path` is the workspace-relative directory to search from; clients usually send `"."`.
- `query` fuzzy-matches file and directory paths case-insensitively. File contents are never read.
- The host walks recursively with gitignore/global ignore support, does not follow symlink directories, and skips common generated directories such as `.git`, `node_modules`, `target`, `dist`, and `build`.
- Each `FsSearchEntry` carries `rel_path` (search-root-relative) and `match_indices`: the host matcher's matched character positions into `rel_path`, sorted and deduplicated. Clients highlight using these indices rather than re-running a separate matcher.
- `limit` is clamped to `FS_SEARCH_MAX_LIMIT`; `0` means `FS_SEARCH_DEFAULT_LIMIT`.
- `truncated = true` means the result cap or visited-entry cap was hit before the host could prove there were no more matches.

### FsDocsTree conventions

- `rebuild = true` scans the requested directory, replaces the per-session in-memory docs-tree snapshot, and returns page 0.
- `rebuild = false` serves a page from the matching `snapshot_id`; `CacheMiss` or `StaleSnapshot` means the client should ask the user to rebuild.
- The host detects markdown files with case-insensitive `.md` and `.markdown` extensions only.
- The host combines gitignore matching with built-in fallback ignores for dot-prefixed, generated, dependency, and build directories, and never follows symlink directories.
- Limits: default page size `200`, max page size `1_000`, max offset `5_000`, max visited entries per rebuild `10_000`.
- `truncated = true` means host caps prevented proving the full docs tree was scanned.
- The client treats `Unsupported` as a compatibility result for older hosts and should not keep showing an active build state.

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
- `TermReorder(TermReorderReq) -> TermReorderResult`
- `SyncSessionResult.terminals -> Vec<TerminalSyncEntry>`
- Terminal ids are opaque host-generated UUID strings.
- `TermCreateReq.color_scheme` is optional. New clients send `Dark` or `Light`
  so the host can answer startup OSC 10/11/12 color queries immediately for
  launch-command TUIs before a client terminal view attaches.

### TermAttach conventions

- Client passes `last_seq` to request backlog replay.
- Host may replay missed output before live stream.
- Host may send a synthetic metadata preamble as `TermOutput { seq: 0, ... }` before backlog replay. Clients must process the bytes as normal PTY output but must not use `seq=0` for backlog sequence tracking.
- The synthetic preamble replays cached OSC metadata that may have fallen out of the backlog, including title, icon name, cwd, shell command line, command start/idle state, and last exit code.
- While a foreground command is still latched (command start seen without a matching command end — e.g. an agent emitting prompt-ready between turns), an idle preamble replays the latched command line (`633;E`), command start (`633;C`), and prompt-ready (`633;A`) instead of the stale command end (`633;D`), so a freshly attached client re-derives the agent identity and keeps it across reattach.
- Output `seq` is monotonic per session backlog stream and used for gap detection.
- `SyncSessionResult.terminals` and `TermListResult.terminals` are ordered by host-owned terminal order. Creation order is the default until a client submits an explicit order.
- `TerminalSyncEntry.position` and `TermListEntry.position` are zero-based positions in that host-owned order.
- `TerminalSyncEntry.last_seq` is the host's latest backlog sequence observed for that terminal at sync time.
- `TerminalSyncEntry` carries the host's full cached terminal metadata at sync time: `title`, `cwd`, `icon_name`, `agent_command`, `shell_state`, `last_exit_code`, `agent_state`, and `agent_slug`. Clients seed their terminal metadata from this snapshot on every sync; the host is the source of truth across reconnects. `TermAttach` still replays the same metadata as PTY bytes so normal terminal-event consumers are seeded through one path.
- `TerminalSyncEntry.agent_command` is the foreground command latched at command start (or from the spawn launch command). It survives prompt-ready between agent turns and clears only on command end.
- `TerminalSyncEntry.agent_slug` is the host-resolved agent identity for that command (`None` for a plain shell). The host resolves it from `agent_command` (with `icon_name` as a fallback) using the registered actors' detection rules. Clients render this slug directly and must not re-detect agent identity from the command line; live changes arrive via `HostEvent::TerminalAgentChanged`.
- Clients should keep local terminal tabs keyed by terminal id and use `last_seq` to seed reconnect `TermAttach` calls.

### SyncSession conventions

- `SyncSession` is a mid-session state refresh; connect-time bootstrap is piggybacked on `ConnectResult::Ok` and `AuthProveResult::Ok`.
- Host rotates and returns a fresh `session_token` on every successful `SyncSession`, token-accepted `Connect`, and `AuthProve` attach.
- `session_id` in `SyncSessionResult` is authoritative and must replace any stale client-side session id.
- `SyncSessionResult.delta_pubkey` is the dedicated host Delta node authorization public key. Mobile uses it when registering the host with Delta; it is not a Zedra transport or telemetry identity.
- Zedra persists the host Delta pubkey and resolved host node id per workspace, then reuses that binding on later reconnects or after app launch. If Delta sign-in appears later, the client can replay the same workspace-scoped binding without inventing a new host identity.
- A signed-in mobile client reads its current `stack_id` and `client_node_id` from app-owned Delta state, then sends `SetClientDeltaInfo { delta_url, stack_id, client_node_id, host_node_id }` after registering the host node or after reloading a persisted workspace binding. The host holds these Delta IDs in daemon memory so agent-hook notifications target the previous known mobile client without persisting transport identity or broadcasting to the stack.
- When the client signs out of Delta, it clears the host-side in-memory Delta binding so later agent-hook notifications do not target stale client IDs until the client signs back in and replays the workspace binding.
- `SyncSessionResult.terminals` is the authoritative server-side terminal set at bootstrap time. During reconnect, clients preserve the existing local order for terminals still present in that set and append any newly discovered host terminals unless the client has submitted an explicit host order.
- `TermReorderReq.ordered_ids` must be an exact permutation of the current active terminal ids. The host rejects partial, duplicate, or unknown-id orders.
- `ConnectReq.session_token` and `SyncSessionResult.session_token` are opaque, host-issued, session-bound, and client-bound.
- Session tokens are currently ephemeral host memory only; host restart may invalidate them and force PKI fallback.

### SwitchSession status

`SwitchSession` is reserved for a future active-session transfer protocol. The
current host handler returns an explicit unsupported error because it does not
replace the session bound to the authenticated dispatch loop. Clients must not
use it to switch the active workspace; connect to the target session through the
normal `Connect`/`AuthProve` flow instead.

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

## 5.7 AI, Managed Agents, and LSP

- `AiPrompt(AiPromptReq) -> AiPromptResult`
- `AgentList(AgentListReq) -> AgentListResult`
- `AgentSessions(AgentSessionsReq) -> AgentSessionsResult`
- `AgentResume(AgentResumeReq) -> AgentResumeResult`
- `AgentInstalledList(AgentInstalledListReq) -> AgentInstalledListResult`
- `AgentFiles(AgentFilesReq) -> AgentFilesResult`
- `LspDiagnostics(LspDiagnosticsReq) -> LspDiagnosticsResult`
- `LspHover(LspHoverReq) -> LspHoverResult`

### Managed agent conventions

**Terminology:** An *agent actor* is identified by a stable slug such as
`claude`, `codex`, or `maki`. A single host registry supplies both managed
features and installed-CLI metadata. Every actor appears in `AgentListResult`;
unsupported actor methods return their default empty or unsupported result. Most session-capable actors scope
their history per workspace; `hermes` is global.

- `AgentListResult.agents` returns one `AgentSummary` for every registered actor, even when the CLI is missing or no sessions exist. `AgentSummary.slug` is the canonical identity; clients resolve the icon from the slug (see `InstalledAgentEntry.icon_name` for the launcher's native asset).
- `AgentSummary.shows_detail` is host-owned and marks actors that aggregate sessions/account/usage worth a detail screen (`claude`, `codex`, `opencode`, `pi`, `hermes`). Detect-only actors return `false`; the app's manage-agents list shows only CLI-detected agents with `shows_detail = true`.
- `AgentListReq.refresh` and `AgentInstalledListReq.refresh` default to `false`. When `false`, the host serves its startup cache; when `true`, the host rescans synchronous fields before responding.
- `AgentInstalledList` is a launch-oriented projection of the same actor registry.
- `AgentSessionsReq.slug`, `AgentResumeReq.slug`, and `AgentFilesReq.slug` select an actor. Unknown or unsupported features return typed errors; `AgentSessionsResult.sessions` contains the latest workspace-matching sessions and `total` is the full match count before applying `limit`.
- `AgentSessionsReq.limit` defaults to `0`, which uses the host default (`50`, overridable with `ZEDRA_AGENT_SESSION_LIMIT`, max `200`).
- `AgentSessionsReq.refresh` follows the same cache rule as `AgentListReq.refresh`.
- `AgentResume` creates a new terminal and starts the provider-specific resume command on the host. Clients must not build provider shell commands from summary fields.
- `AgentInstalledList` returns all supported terminal agent slugs with host-owned `launch_cmd` values for available CLIs. Clients create agents through `TermCreate` with that launch command.
- `AgentSummary.usage` carries a live `AgentUsageSnapshot` fetched asynchronously from the provider's API at daemon start and on `AgentListReq{refresh:true}`. Fields populated per agent:
  - **Claude** (preferred): the gauge windows `rate_limit_*_used_percent` + `rate_limit_*_resets_at`, and `extra` fields (`Opus weekly`, `Sonnet weekly`, `Extra credits` for spend) — from `api.anthropic.com/api/oauth/usage` when `~/.claude/.credentials.json` has a valid OAuth token (structured JSON; same reliability model as Codex).
  - **Claude** (PTY fallback, unix host only): when there is no readable credentials file (typical Keychain-only CLI login). The host spawns `claude` with `CLAUDE_CODE_NO_FLICKER=0` so the `/usage` panel renders line-by-line instead of the alt-screen TUI (which collapses labels onto one line and defeats parsing), captures `/usage` text, and scrapes percentages and optional `resets_at`. This path is **best-effort**: Claude’s TUI layout is not a protocol contract; reset timestamps may be absent or approximate (host local time). Zedra does not read the macOS Keychain for Claude tokens. One PTY probe is cached ~60s and shared by usage + plan scans.
  - **Codex**: `rate_limit_five_hour_used_percent` (primary window), `rate_limit_seven_day_used_percent` (secondary window) — from `chatgpt.com/backend-api/wham/usage` using `~/.codex/auth.json`.
  - `None` when credentials are missing and the CLI PTY probe fails. When a credentials file token is present but the OAuth usage/profile call fails, the host falls back to the CLI PTY probe (still no Keychain read).
  - `AgentUsageSnapshot` is the shared usage-gauge contract: only `rate_limit_five_hour_*`/`rate_limit_seven_day_*` (used percent + reset unix-seconds). Clients render the gauge from these. Everything else — spend, extra credits, per-model weekly windows — is per-agent and lives in `AgentUsageSnapshot.extra`, a `Vec<AgentInfoField>` each actor populates (e.g. Claude `Opus weekly`/`Sonnet weekly`/`Extra credits`, OpenCode `Spend`); the detail screen renders it as `label: value`.
  - `AgentSummary.highlight` is a one-line string rendered on the manage-screen **card** (empty when none). The host composes it — currently by joining `usage.extra` into one line, but it may be curated from usage, account, or other agent data without changing clients. The detail screen shows the richer `usage.extra`/`account.fields` instead.
- `AgentSummary.account.fields` exposes locally discovered account/setup metadata for manage-agent detail views. Values must remain privacy-safe. Current fields per agent kind:
  - **Claude**: Logged in, Plan (from `~/.claude/.credentials.json` `subscriptionType` / `rateLimitTier`, refreshed from `api.anthropic.com/api/oauth/profile` when OAuth is available, or CLI PTY when the token lives in Keychain); Organization (OAuth profile only); Model, Effort, Permission mode (from `~/.claude/settings.json`); Total cost (USD), Today msgs (from `~/.claude/stats-cache.json` `dailyActivity`). Live rate limits and extra spend come from `AgentUsageSnapshot` on the summary (OAuth API or CLI PTY), not account fields.
  - **Codex**: Logged in, Account (name from JWT), Plan, Plan until (from `~/.codex/auth.json` `id_token` payload, re-read on async refresh); Model, Personality, Reasoning effort (from `~/.codex/config.toml`); Week threads, Total threads (from `~/.codex/state_5.sqlite`).
  - **OpenCode**: Config dir presence (from `~/.config/opencode`).
  - **Pi**: Logged in (presence of `~/.pi/agent/sessions/`). Sessions are scanned from `~/.pi/agent/sessions/--<workdir>--/<timestamp>_<uuid>.jsonl`; resume uses `pi --session <id>`. No lifecycle hooks; live binding falls back to terminal command detection (`pi` as first token).
  - **Hermes**: per-provider auth + active provider (from `$HERMES_HOME/auth.json`, default `~/.hermes`), Default model / Default provider (from `config.yaml` `model:` block), Skills count (`$HERMES_HOME/skills/**/SKILL.md`), and `state.db` rollups (Total spend, Platforms = `DISTINCT source`). Sessions are **global** (not workspace-scoped): read from `$HERMES_HOME/state.db` (`sessions` table — curated title, `source` platform, `tool_call_count`, per-session cost, model; falls back to `sessions/session_*.json` only when the db is absent), so `AgentSessionsReq.kind == Hermes` ignores the workdir and returns all sessions incl. gateway/ACP; resume uses `hermes --resume <session_id>`. Shell hooks map `on_session_start` and `post_approval_response` to Running, `pre_approval_request` to WaitingApproval, and `post_llm_call`/`on_session_end` to Completed. `AgentFiles` exposes `SOUL.md`/`USER.md`/`MEMORY.md`/`config.yaml`/`.env`/`cron/jobs.json` read-only.
- `AgentFiles(AgentFilesReq{kind}) -> AgentFilesResult{files}` returns an agent's host-side config/memory files **read-only** for detail views. Each `AgentFile` carries `label`, absolute `path`, `content` (host-capped at 256 KiB; `truncated` flags clipping), and `missing` (file absent). Only `Hermes` exposes a set today (`SOUL.md`, `USER.md`, `MEMORY.md`, `config.yaml`, `.env`, `cron/jobs.json`); other kinds return an empty list. The client never writes these files. `.env` and other credential-bearing files are returned verbatim — acceptable because the RPC transport is end-to-end encrypted and a client only views its own host's config; treat the contents as sensitive at rest.
- `AgentSessionSummary.title` uses provider-stored titles when available, otherwise `"Unknown"`.
- `AgentSessionSummary.transcript_size_bytes` reports transcript file size when the host scan has a local file path.
- `AgentGitSummary.worktree` is populated for Claude when the encoded project path includes `--claude-worktrees-<name>`.
- Summary timestamps use `DateTime<Utc>` serialization through serde/postcard.
- Data sources are explicit through `AgentDataSource` so clients can distinguish CLI/setup checks, historical scans, terminal metadata, hook state, status lines, and provider CLI output.
- Summaries must not expose prompt text, command arguments, tool input/output, transcript bodies, or last assistant messages. Allowed fields are safe labels, ids, timestamps, counts, paths already scoped to the workspace, and provider metadata such as model, source, permission mode, CLI version, git branch, and PR link metadata.
- `AgentLifecycleStatus`, `AgentEventKind`, and `AgentActionKind` provide the cross-agent vocabulary for future hook-driven prompts and notifications.

### Async managed-agent fetching

CLI `--version` probes are slow and cached separately from the synchronous agent scan.

1. **Daemon startup:** preload scans agents into cache, then probes versions in the background.
2. **`AgentList` read:** returns cache with any known versions merged; live terminal bindings merged on the host for the response.
3. **`AgentList` with `refresh: true`:** rescans synchronous fields immediately, then starts a background version refresh.
4. **Background completion:** host pushes `HostEvent::AgentInfoChanged { info }` once per managed agent to every session with an active `Subscribe` stream. `info` is the full cached `AgentSummary` for that agent (including versions and live bindings for that session).
5. **Client update:** replace the cached row for `info.kind` (do not treat the event as a partial patch).

---

## 6) Host Events

Current host event variants:

- `TerminalCreated { id, launch_cmd }`
- `GitChanged`
- `FsChanged { path }`
- `AgentInfoChanged { info }`
- `TerminalAgentChanged { terminal_id, agent_slug }`

Client rules:

- `TerminalCreated`: attach/open terminal view if relevant.
- `GitChanged`: invalidate cached git state and refresh when appropriate.
- `FsChanged { path }`: invalidate cached file tree for the watched path and reload affected expanded nodes.
- `AgentInfoChanged`: replace cached `AgentSummary` for `info.slug`. One event per managed agent per version refresh. Requires an active `Subscribe` stream.
- `TerminalAgentChanged`: update the terminal's agent identity to `agent_slug` (`None` clears it). Emitted when the host-resolved foreground agent for a terminal changes (command start/end). Authoritative — clients render it instead of re-detecting locally. Requires an active `Subscribe` stream.

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

### 7.4 Multi-ALPN Backward Compatibility

Status: implemented. The host serves `zedra/rpc/4` and `zedra/rpc/3`.

A `ZEDRA_ALPN` bump strands shipped apps on the old ALPN (§2.4) until App Store
review clears the new build, so the host keeps serving the previous version.
This is a scoped exception to the "host does not tailor responses per client"
rule, and it **recurs on every bump** (v4 keeps v3, …; a chain if review windows
overlap). Only one previous version is kept: `zedra/rpc/2` was dropped at the
`v4` bump once its traffic reached zero. The `v3`→`v4` delta is response-only —
requests are append-only and the `v3` request set is a prefix of `v4`'s, so the
auth handshake is unchanged.

How it works:

- **Advertise both ALPNs** in `iroh_listener.rs`
  (`.alpns(vec![ZEDRA_ALPN, ZEDRA_ALPN_V3])`).
- **`read_zedra_message(conn)` is the only version seam** — it reads
  `conn.alpn()`, decodes with the matching schema, and for a legacy ALPN calls
  `into_live()` to lift the request into `ZedraMessage`. Auth and dispatch then
  run one version-agnostic path; no version flag is threaded through.
- **`proto_v3.rs`** freezes `ZedraProtoV3`/`ZEDRA_ALPN_V3` and only the diverged
  types; byte-identical messages reuse `crate::proto` (pinned by roundtrip
  tests). `into_live` rebuilds byte-identical variants via
  `(m.inner, m.tx, m.rx).into()` and wraps diverged senders with
  `with_map`/`with_filter_map` (running `From<live> for frozen`); diverged
  requests lift `From<frozen> for live`.

Behavioral degradation for `v3` clients (all non-fatal): agents moved from the
`AgentKind` enum to slug strings at `v4`, so actors outside the frozen five
(`claude`/`codex`/`opencode`/`pi`/`hermes`) are filtered out and the removed
`AgentCapabilities` is synthesized; the `agent_slug` field on `TerminalSyncEntry`,
the `extra` usage lines, and the `v4`-only `TerminalAgentChanged` event are
dropped (an unknown discriminant would be undecodable and kill the stream, §2.4).

Exit: drop a frozen module + its `alpns(...)` entry once that version's traffic
(tracked via the telemetry ALPN field) hits zero; record under §11. This is a
stopgap, not a substitute for the forward-compatible encodings in issue #140.

#### Adding the next version (recipe)

1. Before bumping `ZEDRA_ALPN`, snapshot the outgoing schema from the pre-bump
   commit into `proto_v{N}.rs`; freeze its enum, `ZEDRA_ALPN_V{N}`, and the
   diverged types only.
2. Diff old vs new by **transitive closure**, not struct decls — an unchanged
   decl can hide a diverged nested type (this caused two v2 regressions:
   `TermCreateV2` and the agent-session subtree).
3. Implement `into_live`, add a `read_zedra_message` arm + an `alpns(...)` entry.
4. Filter values unrepresentable on old clients (new enum variants/kinds) out of
   frozen responses rather than emitting an undecodable discriminant.

Frozen types `#[derive(Default)]` so synthesized defaults stay one line. Notable
irpc facts: `Connection::alpn()` gives the negotiated value post-handshake;
`Sender::with_map`/`with_filter_map` is the re-encode seam (`Receiver` has none,
so `TermAttach`'s `rx` reuses the byte-identical `proto::TermInput`);
`WithChannels` fields are public, so `into_live` re-tags a message across services
without touching irpc internals.

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

### 2026-06-29

- Bumped `ZEDRA_ALPN` to `zedra/rpc/4`. The slug-based agent rewrite (2026-06-24)
  and host-driven agent identity (2026-06-26) change how existing bytes decode
  (`AgentKind`/`AgentCapabilities` removed, `TerminalSyncEntry.agent_slug` and
  `AgentUsageSnapshot.extra` added, `HostEvent::TerminalAgentChanged` added), so
  they ship under `v4` rather than the previously documented "no bump".
- Removed `zedra/rpc/2` support (deleted `proto_v2.rs` and its `alpns(...)` entry)
  now that its traffic has reached zero. The host serves `zedra/rpc/4` and
  `zedra/rpc/3` per §7.4.
- Added `proto_v3.rs`: freezes the `v3` schema (enum-based agents) and lifts it
  to live `v4`. Agents outside the frozen five (`claude`/`codex`/`opencode`/
  `pi`/`hermes`) are filtered from `v3` responses; `agent_slug`, `usage.extra`,
  and `TerminalAgentChanged` are dropped for `v3` clients.
- Added `agent_slug` to `HostEvent::TerminalCreated`. A spawned launch command
  emits no command-line OSC, so the host now resolves the agent identity from the
  launch command and ships it with the create event; clients show the icon
  immediately instead of waiting for a reconnect. Dropped for `v3` clients in
  `host_event_v3`.

### 2026-06-26

- Host-driven agent identity. Added `TerminalSyncEntry.agent_slug` (host-resolved
  foreground agent for a terminal) and `HostEvent::TerminalAgentChanged
  { terminal_id, agent_slug }` (live identity updates). The host now resolves
  agent identity from the foreground command (with OSC 1 `icon_name` fallback)
  using the registered actors' detection rules; clients render the slug directly
  instead of re-detecting, resolving the icon locally from the slug. Additive to
  the unshipped `zedra/rpc/4` schema — no ALPN bump; the new `HostEvent` variant
  is dropped for `zedra/rpc/2` clients in `host_event_v2`.

### 2026-06-24

- Replaced the live managed-agent enum with stable slug strings in agent RPC
  requests, summaries, sessions, files, and hook events. This rewrites the
  unshipped `zedra/rpc/4` schema without changing its ALPN. Future actor slugs
  are data, not protocol variants, so adding an agent does not require another
  ALPN bump. The frozen `zedra/rpc/2` enum remains unchanged for compatibility.

### 2026-06-20

- Implemented §7.4 Multi-ALPN Backward Compatibility: the host serves both
  `zedra/rpc/3` and `zedra/rpc/2` so shipped apps survive an App Store review
  delay. New `crates/zedra-rpc/src/proto_v2.rs` freezes the old schema and lifts
  legacy requests into the live `ZedraMessage` via `into_live`; one auth+dispatch
  path serves both. `v2` clients get empty agent `live` fields and have
  `Pi`/`Hermes` + the `v3`-only `HostEvent` variants filtered out. Response-only
  delta; stopgap pending issue #140.

### 2026-06-11

- Extended `TerminalSyncEntry` with the host's full cached terminal metadata: `agent_command`, `shell_state` (new `TermShellState` enum), and `last_exit_code`. Clients seed terminal metadata from the sync snapshot on reconnect instead of re-deriving it from replayed PTY bytes.
- `agent_command` latches the foreground command at command start (or spawn launch command), survives prompt-ready between agent turns, and clears on command end — used to restore the agent icon after reconnect.
- An idle `TermAttach` preamble with a latched foreground command replays command line + start + prompt-ready instead of a stale command end, so it never clears a client's latched agent identity.

### 2026-05-29

- Added §2.4 Schema Evolution and Decode Compatibility: documents the postcard
  undecodable-unknown problem, the current ALPN version-gate discipline
  (append-only within a version, bump on any wire change), and the target
  forward-compatible encodings (open enums, key/value records, versioned
  variants) tracked in issue #140.
- Added `Hermes` managed agent kind and `AgentFiles(AgentFilesReq) ->
  AgentFilesResult`.
- Added `FsSearch(FsSearchReq) -> FsSearchResult` with
  `FS_SEARCH_DEFAULT_LIMIT` / `FS_SEARCH_MAX_LIMIT`; `limit = 0` uses the
  default, oversized limits are clamped, `truncated` reports capped results, and
  `match_indices` identify host-scored character positions in `rel_path`.
- ALPN bumped to `zedra/rpc/3`.

### 2026-04-29

- Added host-built docs tree RPC:
  - `FsDocsTree(FsDocsTreeReq) -> FsDocsTreeResult`
- Added recursive docs tree payload and typed docs-tree errors:
  - `FsDocNode`
  - `FsDocsTreeError::{InvalidPath, InvalidRequest, CacheMiss, StaleSnapshot, Busy, ScanFailed, Unsupported}`
- Added bounded manual rebuild snapshot behavior:
  - per-session in-memory cache
  - 10 minute cache TTL
  - no ALPN change

### 2026-04-28

- Aligned docs with `ZEDRA_ALPN = b"zedra/rpc/2"`.
- Documented `Authenticate` as a deprecated/reserved append-only enum variant.
- Documented `SwitchSession` as reserved/unsupported for active workspace switching because the current host dispatch loop remains bound to the originally authenticated session.

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

- Deprecated `Authenticate(AuthReq) -> AuthChallengeResult` as a reserved append-only enum variant.
- Removed the older standalone reconnect RPC from the active protocol.
- Added unified `Connect(ConnectReq) -> ConnectResult` replacing the previous challenge request and standalone reconnect paths:
  - `ConnectReq` carries optional `session_token` for 1-RTT fast path.
  - `ConnectResult::Ok(SyncSessionResult)` — token valid; session attached immediately.
  - `ConnectResult::Challenge { nonce, host_signature }` — no valid token; host embeds PKI challenge inline, saving a separate challenge request RTT.
- `AuthProveResult::Ok` now carries `SyncSessionResult` (piggybacked bootstrap); separate `SyncSession` call no longer needed at connect time.
- Renamed the bootstrap token field to `session_token`.
- Session token storage changed to single-slot (`Option<([u8; 32], SessionToken)>`) per session — only one token is valid at a time, consumed atomically on validation. No TTL.
- ALPN bumped to `zedra/rpc/2` (breaking change).

### 2026-03-26

- Added an earlier fast reconnect bootstrap RPC, later superseded by `Connect(session_token)` on 2026-03-28.
- Added canonical session bootstrap RPC:
  - `SyncSession(SyncSessionReq) -> SyncSessionResult`
- Added `TerminalSyncEntry` to return resumable terminal ids + latest backlog sequence + cached title/CWD.
- Updated connection lifecycle:
  - first pairing and PKI fallback now end with `SyncSession`
  - token resume may skip PKI challenge/AuthProve entirely when a valid host-issued session token is present
