# zedra-host

Desktop daemon and CLI for serving filesystem, git, PTY, auth, and telemetry over iroh + irpc.

## What This Crate Owns

- CLI entrypoints in `src/main.rs`
- iroh endpoint setup and accept loop
- PKI auth and RPC dispatch in `src/rpc_daemon.rs`
- persistent server sessions in `src/session_registry.rs`
- host-side PTY, filesystem, git, telemetry, and local REST helpers

## Working Rules

- Keep request-serving logic in `rpc_daemon.rs` thin when possible. Push reusable stateful behavior into `session_registry.rs`, `git.rs`, `fs.rs`, `pty.rs`, or other focused modules.
- Preserve the auth flow exactly unless the protocol is intentionally changing: `Register -> Connect -> Challenge -> AuthProve`, with the session-token fast path inside `Connect`.
- Treat `SessionRegistry` as the authority for session lifetime, ACLs, active attachment, pairing slots, watched paths, and terminal ownership.
- Sessions survive transport disconnects. Do not couple connection teardown to session destruction.
- Maintain the one-active-client-per-session rule unless the product requirement changes explicitly.

## Safety Boundaries

- All client paths must stay jailed to `workdir`. Reuse the `resolve_path` and observer-path normalization patterns instead of introducing ad hoc path handling.
- Keep git execution non-shell and argument-safe. Use `Command::new("git")`, validate refs, and insert `--` before user-controlled paths.
- PTY shells must keep the sanitized environment from `pty.rs`. Do not leak daemon secrets or arbitrary inherited env into spawned shells.
- Telemetry must remain privacy-safe. Do not emit usernames, raw file paths, or IPs through `zedra_telemetry`.

## Concurrency Patterns

- Use `tokio::spawn` for connection-scoped async work.
- Use `tokio::task::spawn_blocking` for blocking PTY reads and filesystem or git fingerprinting.
- Be careful around mixed async and sync locks. `session_registry.rs` and terminal streaming intentionally use `std::sync::Mutex` in a few hot paths so blocking threads do not need `block_on`.
- In terminal streaming, preserve the generation-guard pattern on `OutputSenderSlot`. New attaches must not be clobbered by stale cleanup.
- When editing terminal attach or backlog behavior, keep all three pieces aligned: host meta preamble, backlog replay, and live bridge send path.

## Logging And Metrics

- Follow the existing `component: detail` tracing style in lowercase.
- Keep operator-facing `eprintln!` output sparse and high-signal. Most detail belongs in `tracing`.
- Preserve or extend the existing counters and telemetry events when behavior changes affect auth, sessions, bandwidth, filesystem, git, or AI RPC usage.

## Good Change Shapes

- New RPC behavior usually requires updates in three places: protocol types in `zedra-rpc`, handler logic in `rpc_daemon.rs`, and any persistence or state rules in `session_registry.rs`.
- If a change affects reconnect or replay behavior, verify the interaction between `TermBacklog`, session tokens, active-client attachment, and host events.
- Prefer targeted unit tests in the local module, plus integration coverage in `tests/integration.rs` for transport or auth flows.

## Validation

- `cargo check -p zedra-host`
- `cargo test -p zedra-host`

## Key Files

- `src/main.rs` — CLI orchestration, daemon startup, telemetry init
- `src/rpc_daemon.rs` — auth, dispatch, terminal attach, observer loop
- `src/session_registry.rs` — persistent session state and attachment rules
- `src/iroh_listener.rs` — endpoint creation and accept loop
- `src/identity.rs` — per-workspace identity and secret-file persistence
