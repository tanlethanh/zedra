# Threading & Async Patterns

This document maps every thread boundary and async pattern in Zedra, explains why each exists, and lists known improvement opportunities.

---

## Threads in a Running Session

```
┌─────────────────────────────────────────────────────────────────────┐
│ Main / UI thread  (GPUI, single-threaded, Rc-typed)                 │
│   AndroidApp::process_command()  — drains command queue each frame  │
│   ZedraApp::render()             — reads WorkspaceState, pending slots│
│   Entity tasks                   — await futures channels → cx.notify()│
└───────────────────────┬─────────────────────────────────────────────┘
                        │  futures unbounded channel / PendingSlot.set()
                        │  (no global main-thread callback queue)
                        │
┌───────────────────────┴─────────────────────────────────────────────┐
│ Session runtime  (Tokio, 2 worker threads, "zedra-session")         │
│   SessionHandle::connect()       — QUIC + PKI auth + fetch info     │
│   spawn_reconnect_with_reason()  — exponential back-off loop        │
│   path watcher task              — updates TransportSnapshot        │
│   keep-alive ping task           — detects silent network drops     │
│   closed-watcher task            — fires on clean QUIC FIN          │
│   terminal pump task (×N)        — drains TermAttach bidi stream    │
│   event subscriber task          — receives HostEvent stream        │
└───────────────────────┬─────────────────────────────────────────────┘
                        │  iroh QUIC / TLS 1.3 (relay or P2P)
                        │
┌───────────────────────┴─────────────────────────────────────────────┐
│ Android JNI threads  (JVM-managed, any number)                      │
│   surfaceCreated / surfaceChanged / surfaceDestroyed                 │
│   onTouchEvent, onKeyDown, dispatchKeyEvent                         │
│   onResume / onPause / onDestroy                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Pattern 1 — Android JNI Command Queue

**Files**: `android/command_queue.rs`, `android/jni.rs`, `android/app.rs`

**Problem**: GPUI's `App` contains `Rc`-typed state and must run on a single thread.
Android JNI callbacks can arrive on any JVM thread.

**Solution**: A **crossbeam bounded channel** (cap 512) as a strict boundary:

```
JNI thread (any)
  └─ COMMAND_QUEUE.send(AndroidCommand)   [try_send — drops if full]
Main thread (Choreographer 60 FPS)
  └─ drain_commands()                     [try_recv loop, non-blocking]
     └─ AndroidApp::process_command(cmd)
```

Key properties:
- `try_send` never blocks the JNI thread; full queue → logged warn + drop.
- `drain_commands` is a snapshot: all commands that arrived since the last frame are processed in order before the next render.
- The global `Lazy<AndroidCommandQueue>` is the only shared state between JNI and main.

**iOS equivalent**: No queue needed. All Obj-C app delegate calls are already on the main thread. State is held in `thread_local! { RefCell<Option<Rc<…>>> }` — no `Send` required.

---

## Pattern 2 — Channel-Based UI Wake (Cross-Runtime Wakeup)

**Files**: `zedra-session/src/terminal.rs`, `zedra-session/src/state.rs`,
`crates/zedra/src/workspaces.rs`, `zedra-terminal/src/view.rs`

**Problem**: Tokio tasks on the session runtime must not touch GPUI, but the UI must
re-render as soon as terminal output or session state changes.

**Solution**: `futures::channel::mpsc::unbounded` — producers send `()` from tokio;
GPUI `cx.spawn` tasks await the receiver and call `cx.notify()` on the entity.

```
Tokio (pump / SessionState listener)
  └─ UnboundedSender::unbounded_send(())
GPUI entity task
  └─ while rx.next().await → weak.update(cx, |_, cx| cx.notify())
```

Key properties:
- No global static callback queue; each subscription is owned by the view or `Workspaces`.
- `RemoteTerminal::signal_needs_render` still sets `needs_render` for VTE gating in `render()`.
- `Workspaces` uses one shared receiver task plus per-workspace `state.subscribe()` forwarders.
- See `docs/CONVENTIONS.md` — "Channel-Based Notifications (Zero-Latency)".

---

## Pattern 3 — PendingSlot (Async One-Shot → Main Thread)

**File**: `pending.rs`

**Problem**: Native callbacks (alert button tap, QR scan result) arrive on a platform
thread outside both the session runtime and the GPUI main thread, and deliver a single
value that the render loop should consume once.

**Solution**: `PendingSlot<T>` — a `Mutex<Option<T>>` with `set` / `take`:

```rust
// Platform callback (any thread):
PENDING_QR_TICKET.set(ticket);

// GPUI render loop (main thread, each frame):
if let Some(ticket) = PENDING_QR_TICKET.take() { ... }
```

Key properties:
- `set` is last-write-wins; suited for one-shot events where duplicate delivery is safe.
- No wakeup signal — the render loop polls each frame. Acceptable because frames run at 60 FPS.
- `SharedPendingSlot<T>` = `Arc<PendingSlot<T>>` for non-static use cases.

---

## Pattern 4 — Session Runtime (Isolated Tokio Runtime)

**File**: `zedra-session/src/lib.rs`

**Problem**: GPUI runs its own async executor. Mixing GPUI tasks with Tokio tasks would
require GPUI to be Tokio-aware. Long-running I/O (QUIC, reconnect loops) would also
compete with GPUI's frame-pacing.

**Solution**: A dedicated `tokio::runtime::Runtime` (`multi_thread`, 2 workers):

```rust
static SESSION_RUNTIME: OnceLock<Runtime> = OnceLock::new();
session_runtime().spawn(async move { ... });   // from sync context
tokio::spawn(async move { ... });              // from within a session task
```

Key properties:
- Entirely separate from any GPUI executor.
- `session_runtime().spawn()` is used to start the reconnect loop (sync call site).
- `tokio::spawn()` is used inside async contexts already on this runtime (sub-tasks).

---

## Pattern 5 — SessionHandle (Arc + Per-Field Locks + Generation Counter)

**File**: `zedra-session/src/handle.rs`

**Problem**: Many independent tasks (path watcher, ping, closed-watcher, reconnect, terminal
pumps) all need read/write access to shared connection state without a single coarse lock
causing contention or deadlocks.

**Solution**: `SessionHandle(Arc<SessionHandleInner>)` with **one lock per logical concern**:

| Field | Sync primitive | Rationale |
|---|---|---|
| `connect_state` | `Mutex<ConnectState>` | Composite; phase + snapshot written together |
| `client` | `Mutex<Option<irpc::Client>>` | Replaced atomically on reconnect |
| `workdir`, `hostname`, … | `Mutex<String>` | Written once on connect, read often |
| `conn_generation` | `AtomicU64` | CAS-free stale-task guard |
| `reconnect_running` | `AtomicBool` | CAS guard against concurrent reconnect loops |
| `user_disconnect` | `AtomicBool` | Written by UI, polled by reconnect loop |
| `skip_next_backoff` | `AtomicBool` | Written by UI, consumed by reconnect loop |

**Stale-task guard**: Every long-lived task captures `my_gen` at spawn time and checks
`conn_generation.load(Acquire) != my_gen` at each loop iteration. When a new `connect()`
call increments the generation, old tasks see the mismatch and exit cleanly without
being explicitly cancelled.

---

## Pattern 6 — RemoteTerminal (Fine-Grained Shared Buffer)

**File**: `zedra-session/src/terminal.rs`

**Problem**: PTY bytes arrive in a Tokio pump task at high frequency. The GPUI render
thread must drain them each frame without blocking. No lock should be held across an
`await`.

**Solution**: `Arc<RemoteTerminal>` with independent locks per data stream:

```
Tokio pump task                         GPUI render thread (each frame)
───────────────                         ──────────────────────────────
push_output(bytes)                      if needs_render.swap(false) {
  osc_scanner.lock() → scan               output.lock() → drain → VTE
  meta.lock() + osc_events.lock()         meta.lock() → read title/cwd
  output.lock() → push_back              osc_events.lock() → drain
needs_render.store(true, Release)       }
notify_tx.unbounded_send(())  →  TerminalView listener → cx.notify()
```

Key properties:
- Each `lock()` is held only for the duration of one push/drain — never across an `await`.
- `needs_render: AtomicBool` gates VTE processing so frames with no new data skip it cheaply;
  the unbounded channel schedules frames when the pump has bytes.
- `input_tx: Mutex<Option<mpsc::Sender>>` is swapped out on reconnect while the
  render thread may hold a reference; the `Mutex` serialises the swap safely.

---

## Pattern 7 — Host RPC Dispatch (Per-Request Task Spawn)

**File**: `zedra-host/src/rpc_daemon.rs`

**Problem**: The irpc dispatch loop must not block on slow RPC handlers (e.g. a git diff
over a large repo) while new requests keep arriving.

**Solution**: Spawn one Tokio task per incoming message:

```rust
loop {
    match irpc_iroh::read_request::<ZedraProto>(&conn).await {
        Ok(Some(msg)) => { tokio::spawn(async move { dispatch(msg, …).await }); }
        Ok(None) => break,
    }
}
```

**TermAttach split-task**: For terminal I/O, a slow relay (e.g. 300 ms RTT) would cause
`irpc_tx.send().await` to stall under QUIC flow control, blocking keystroke delivery.
The solution is to decouple output from input with a dedicated output task:

```
PTY reader task
  └─ mpsc::Sender<TermOutput>  (cap 256)
Output task (tokio::spawn)
  └─ drain mpsc, coalesce chunks, irpc_tx.send().await
Input loop (inline)
  └─ irpc_rx.recv().await → write to PTY pty_fd
```

Coalescing: while the previous QUIC send is in flight, `try_recv` merges any accumulated
PTY chunks. This reduces irpc framing overhead without adding interactive latency (single
keystrokes never accumulate).

---

## Improvement Opportunities

### I1 — Reduce PendingSlot polling for platform one-shots

**Current**: Some views poll `SharedPendingSlot` on a timer; `ZedraApp::tick()` handles
deeplinks and similar.

**Done for high-frequency paths**: Terminal output and session state use channel →
`cx.notify()` (see Pattern 2 and CONVENTIONS.md).

**Remaining**: Platform callbacks (QR, alerts) could use a dedicated channel into a
GPUI entity or `cx.spawn` instead of timer polling, if latency matters.

---

### I2 — `connect_state` lock is held too broadly in the reconnect countdown

**Current**: The reconnect loop acquires `connect_state.lock()` inside a `for remaining in … { sleep(1s) }` loop, updating `next_retry_secs` each second. The lock is grabbed and released once per second but the `ConnectPhase::Reconnecting { next_retry_secs }` value is only meaningful for display.

**Problem**: Any other task reading `connect_state` during the countdown spins through
`lock()` acquisitions. Minor now; more noticeable if more readers are added.

**Fix**: Store `next_retry_at: Instant` in `ConnectState` instead of `next_retry_secs`.
The render loop derives the countdown by computing `next_retry_at.saturating_duration_since(Instant::now()).as_secs()` directly, eliminating the per-second lock-and-notify cycle entirely.

---

### I3 — Session runtime worker count is fixed at 2

**Current**: `worker_threads(2)` — hardcoded.

**Problem**: On devices with 8+ cores this under-utilises the hardware. On very low-end
devices (2 cores shared with GPUI) it may cause scheduling contention. Terminal pump
tasks and the reconnect loop are mostly I/O-bound but a burst of concurrent RPC calls
(file tree + git status + terminal attach simultaneously) can queue behind each other.

**Fix**: Use `worker_threads(available_parallelism().map(|n| n.get().min(4)).unwrap_or(2))`.
Cap at 4 so the session runtime never crowds out GPUI's main thread on mobile.

---

### I4 — `output` ring buffer has no size cap

**Current**: `RemoteTerminal.output: Arc<Mutex<VecDeque<Vec<u8>>>>` grows unbounded if
the render thread falls behind the pump (e.g. app backgrounded, render paused).

**Problem**: A high-throughput PTY (e.g. `cat` of a large file) while the app is
backgrounded can accumulate hundreds of MB of unprocessed output.

**Fix**: Cap the ring buffer at a fixed byte budget (e.g. 4 MB). When the cap is exceeded,
drop the oldest chunks and inject a `\x1bc` reset before the first new chunk so the VTE
state machine doesn't process a partial sequence. This mirrors the existing backlog-gap
reset logic in the pump task.

---

### I5 — Command queue drops silently under load

**Current**: `COMMAND_QUEUE` is bounded at 512. `try_send` logs a warning and drops when
full.

**Problem**: Under sustained touch input (fast scrolling) it is possible to drop scroll
events. The warning is logged but the user sees a stutter with no diagnostic.

**Fix**: Track a `dropped_commands: AtomicU64` counter on `AndroidCommandQueue`. Expose
it in the session panel debug view and include it in the perf-test output. Additionally,
consider raising the bound to 1024 for touch events specifically, or applying token-bucket
rate-limiting to `ScrollWheel` commands before they reach the queue.

---

### I6 — `state_notifier` closure is called while holding `connect_state` lock

**Current**: `notify_state_change()` acquires `state_notifier.lock()` after releasing
`connect_state.lock()`. This is correct today, but several callers set the phase and then
call `notify_state_change()` in sequence, meaning two separate lock acquisitions per phase
transition.

**Problem**: The pattern is fragile. If a future caller forgets to call `notify_state_change()`
after mutating `connect_state`, the UI will silently stale.

**Fix**: Add a `set_phase(phase, cx)` helper that atomically updates the phase and fires
the notifier, making it impossible to mutate phase without notifying:

```rust
fn set_phase(&self, phase: ConnectPhase) {
    if let Ok(mut cs) = self.0.connect_state.lock() {
        cs.phase = phase;
    }
    self.notify_state_change();
}
```

Replace all direct `cs.phase = …` + `notify_state_change()` pairs with `self.set_phase(…)`.

---

### I7 — Android command queue has no back-pressure to Java

**Current**: When the Rust queue is full, commands are dropped. The Java side has no
visibility into this.

**Problem**: Java continues producing at full speed. A persistent overload (e.g. very fast
scrolling on a slow device) produces a stream of dropped-command warnings with no recovery.

**Fix**: Expose a JNI method `isCommandQueueNearFull() -> boolean`. Java's touch handler
checks this before enqueuing non-critical events (scroll deltas, fling updates) and skips
if true. Critical events (surface lifecycle, key events) are always enqueued.

---

### Priority Summary

| # | Impact | Effort | Priority |
|---|---|---|---|
| I1 — Push-based platform callbacks | Medium (latency + code clarity) | Low | High |
| I6 — `set_phase` helper | High (correctness/safety) | Low | High |
| I4 — Output buffer cap | High (OOM prevention) | Medium | High |
| I3 — Runtime worker count | Low (performance) | Low | Medium |
| I2 — Countdown lock reduction | Low (minor contention) | Medium | Low |
| I5 — Command drop counter | Low (observability) | Low | Low |
| I7 — Java back-pressure | Low (edge case) | Medium | Low |
