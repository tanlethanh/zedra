# zedra-session

Client-side connection and reconnect layer for mobile. Owns endpoint binding, auth, reconnect loops, session events, host-event subscription, and remote terminal attach.

## What This Crate Owns

- connection phases and transport watchers in `src/connect.rs`
- UI-facing session orchestration in `src/session.rs`
- mutable session handle API in `src/handle.rs`
- UI-thread snapshots in `src/state.rs`
- client signing abstraction in `src/signer.rs`
- remote terminal attach and replay state in `src/terminal.rs`

## Working Rules

- Treat `Session` as the orchestrator and `SessionHandle` as the mutable shared handle for RPC access and attached terminals.
- `SessionState` is single-threaded snapshot state for the UI thread. Keep it as a reducer over `ConnectEvent`; do not move network work into `state.rs`.
- `connect.rs` emits raw lifecycle events, `state.rs` interprets them, and GPUI code applies those events on the UI thread. Preserve that layering.
- Reconnect behavior is part of the product. Be conservative when changing retry timing, idle detection, path watchers, or terminal reattachment.

## Runtime Rules

- Reusable session-layer code should use `session_runtime()` instead of bare `tokio::spawn()`, unless it is already guaranteed to run inside the session runtime.
- `Session::connect()` intentionally stores signer, endpoint, ticket, session id, and token on `SessionHandle` before the async loop starts. Keep those credentials synchronized if connection flow changes.
- Host event subscription is a sidecar task after successful connection. Changes there must respect abort signals and `closed_notify`.

## Terminal Rules

- `RemoteTerminal` owns `last_seq`, attach state, and the bridging tasks for terminal I/O.
- Preserve replay semantics: `TermAttachReq.last_seq` is how reconnect resume works.
- On reattach, abort and replace prior input/output tasks rather than stacking them.
- If terminal lifecycle changes, keep `SessionHandle`, `RemoteTerminal`, and the host-side `TermAttach` flow consistent.

## Compatibility And Error Handling

- Keep `ConnectError` labels and user messages stable unless the underlying product semantics change.
- The observer RPC downgrade in `handle.rs` is a compatibility shim for older hosts. Do not remove it casually.
- When adding new connection or transport events, wire them through both `ConnectEvent` and `SessionState::apply_event`.

## Logging And Style

- Use `tracing` for lifecycle logs. Keep logs useful for human-in-the-loop mobile debugging.
- Prefer explicit event names and structured connection-state transitions over implicit mutations.
- Keep comments minimal and focused on non-obvious lifecycle or concurrency constraints.

## Good Change Shapes

- Connection-flow changes usually touch `connect.rs`, `state.rs`, and sometimes `session.rs` together.
- RPC additions often need new `SessionHandle` methods, but avoid turning `SessionHandle` into business-logic orchestration; it should stay a thin RPC-facing handle.
- If a change affects auth or session resume, verify interaction among `pending_ticket`, `session_token`, `session_id`, and reconnect loops.

## Validation

- `cargo check -p zedra-session`
- `cargo test -p zedra-session`

## Key Files

- `src/connect.rs` — endpoint config, auth flow, watchers, reconnect loop
- `src/session.rs` — orchestrator, subscribe loop, host-event fanout
- `src/handle.rs` — shared RPC API surface and compatibility fallbacks
- `src/state.rs` — UI-thread reducer and display snapshot
- `src/terminal.rs` — remote terminal bridge and sequence tracking
