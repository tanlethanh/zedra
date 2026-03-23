# Terminal Multiplexing Fix Plan

## Problem

Multiple running terminals can block each other's I/O, causing lag when two or more
terminals are active simultaneously. The transport uses QUIC streams (one bidi stream
per `TermAttach`) which should be independent, but several shared data structures and
blocking patterns upstream of the transport serialize work across terminals.

---

## Root Causes

### 1. Shared session-level output backlog (highest impact)

`ServerSession` owns **one** `output_backlog: Mutex<VecDeque<BacklogEntry>>` and **one**
`next_output_seq: Mutex<u64>` shared by all terminals.

Every PTY chunk — from any terminal — runs this sequence inside a `spawn_blocking` thread:

```rust
let seq = rt.block_on(async {
    let s = sess.next_backlog_seq().await;        // lock 1: next_output_seq
    sess.push_backlog_entry(BacklogEntry { … }).await;  // lock 2: output_backlog
    s
});
```

When terminal A generates heavy output (e.g. `cat large_file`), terminal B's
`spawn_blocking` thread blocks on `block_on(output_backlog.lock().await)`.
**All terminals serialize through the same two async locks on every PTY chunk.**

### 2. PTY reader thread blocked on full channel (medium impact)

After the backlog, the PTY reader does:

```rust
if rt.block_on(tx.send(TermOutput { data, seq })).is_err() { … }
```

`tx` capacity is 256. Under relay congestion (~300 ms RTT) the output task drains slowly.
When the channel fills, `tx.send()` blocks, stalling the `spawn_blocking` thread
indefinitely. With N terminals under congestion, N blocking threads stall simultaneously,
exhausting Tokio's blocking thread pool.

### 3. Shared `session.terminals` mutex in the input hot path (minor)

Each `TermAttach` input loop acquires `session.terminals.lock().await` **per keystroke**,
then calls synchronous `write_all + flush` while holding the async mutex. Two terminals
receiving simultaneous input (e.g. paste in both) contend on this lock with blocking I/O
inside.

### 4. Sequential `reattach_terminals` on reconnect (UX)

Terminals are reattached one-by-one with sequential awaits. With 3 terminals over relay
(~300 ms each), this adds ~900 ms of avoidable reconnect latency.

### 5. QUIC connection-level flow control (transport, inherent)

All `TermAttach` streams share one QUIC connection and therefore one congestion window.
A bursty terminal can fill the connection-level window, back-pressuring all streams.
This is addressed indirectly by fixing #1 and #2 (less data in flight at once, less
stalling). A future improvement could use QUIC stream priorities.

---

## Fixes

### Fix 1 — Per-terminal backlog (`session_registry.rs`)

**Change**: Move `output_backlog` and `next_output_seq` from `ServerSession` into
`TermSession`. Combine both into a single `TermBacklog` struct behind one `Mutex` so
seq allocation and push are one lock acquisition.

```rust
// Before (session-level, shared):
pub output_backlog: Mutex<VecDeque<BacklogEntry>>,
pub next_output_seq: Mutex<u64>,

// After (per-terminal, in TermSession):
pub backlog: Mutex<TermBacklog>,

pub struct TermBacklog {
    pub entries: VecDeque<BacklogEntry>,
    pub next_seq: u64,
}
```

The PTY reader in `create_terminal()` takes only its own terminal's `backlog` lock.
`backlog_after()` reads from `TermSession.backlog` instead of session-level.

**Impact**: Eliminates all cross-terminal serialization on the PTY output path.

### Fix 2 — Non-blocking PTY→channel send (`rpc_daemon.rs`)

**Change**: Replace `rt.block_on(tx.send(...))` with `tx.try_send(...)`. On
`TrySendError::Full`, accumulate data into a small pending buffer local to the reader
loop and retry on the next PTY read. On `TrySendError::Closed`, clear the sender slot
(already done on error).

```rust
// Before:
if rt.block_on(tx.send(TermOutput { data, seq })).is_err() {
    output_sender.lock().unwrap().sender = None;
}

// After:
// pending_buf: Vec<u8> carried across loop iterations
pending_buf.extend_from_slice(&data);
let out = TermOutput { data: std::mem::take(&mut pending_buf), seq };
match tx.try_send(out) {
    Ok(()) => {}
    Err(TrySendError::Full(ret)) => { pending_buf = ret.data; }
    Err(TrySendError::Closed(_)) => { output_sender.lock().unwrap().sender = None; }
}
```

**Impact**: `spawn_blocking` threads never stall on QUIC back-pressure. Under congestion,
data coalesces naturally in `pending_buf` and is sent as one chunk once the channel
drains, which is consistent with the coalescing already done in the output task.

### Fix 3 — Capture PTY writer at setup, remove per-keystroke lock (`rpc_daemon.rs`)

**Change**: In the `TermAttach` handler, capture the `pty_writer` (a `Box<dyn Write +
Send>`) at setup time and move it into the input loop. Remove `session.terminals.lock()`
from the hot path entirely.

```rust
// Before (per keystroke):
let mut terms = session_for_input.terminals.lock().await;
if let Some(term) = terms.get_mut(&term_id_for_input) {
    let _ = term.writer.write_all(&term_input.data);
    let _ = term.writer.flush();
}

// After (setup once, write directly):
// writer: captured Arc<Mutex<Box<dyn Write+Send>>> extracted at TermAttach start
if writer.lock().unwrap().write_all(&term_input.data).is_ok() {
    let _ = writer.lock().unwrap().flush();
}
```

Since `TermSession.writer` is already `Box<dyn Write + Send>`, wrap it in an
`Arc<Mutex<...>>` so it can be shared between the input loop and a future resize handler.
Move `TermSession.writer` to `Arc<Mutex<Box<dyn Write + Send>>>`.

**Impact**: Keystrokes from different terminals no longer contend on a single async mutex
holding blocking I/O.

### Fix 4 — Parallel `reattach_terminals` (`handle.rs`)

**Change**: Use `futures::future::join_all` to attach all terminals concurrently.

```rust
// Before:
for terminal in &terminals {
    if let Err(e) = self.attach_terminal(client, terminal).await { … }
}

// After:
let futs = terminals.iter().map(|t| self.attach_terminal(client, t));
for result in futures::future::join_all(futs).await {
    if let Err(e) = result { … }
}
```

**Impact**: Reconnect resume time scales O(1) with terminal count instead of O(N).

---

## Files Changed

| File | Change |
|------|--------|
| `crates/zedra-host/src/session_registry.rs` | Fix 1: per-terminal backlog struct |
| `crates/zedra-host/src/rpc_daemon.rs` | Fix 1 call-sites, Fix 2, Fix 3 |
| `crates/zedra-session/src/handle.rs` | Fix 4 |

---

## Non-Goals

- QUIC stream priorities (Fix 5): requires evaluating iroh 0.96 API; deferred.
- Per-session QUIC connections per terminal: too invasive; deferred.
