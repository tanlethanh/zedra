# Task Live Activity

Glanceable progress for agent/terminal tasks across all workspaces, shown in the
Dynamic Island and on the lock screen. Distinct from push notifications: a Live
Activity is a long-lived, updatable surface with its own lifecycle.

## Model

One **aggregate** Live Activity per app install. It rolls up every active task
across all workspaces and sessions. Zedra Delta is the stateful aggregator and the
single source of truth.

```
Workspace (many)  ──emit task events──▶  Delta (aggregate + tokens + timers)
  └─ Session                                  │ push full content-state via APNs
       └─ Task (time-bound, many)             ▼
                                         iOS app: one Activity, renders state
```

Why aggregate, not one-per-task: iOS soft-caps concurrent activities, the Dynamic
Island shows one at a time, and a background APNs update **replaces** content-state
wholesale (app code does not run to merge). So the merge must happen server-side,
and only Delta sees all workspaces.

## Content State (MVP)

Tiny by design — the compact island is the only surface in the MVP.

```swift
struct ContentState {
    enum Trailing { case loading, needsAction, done }
    var trailing: Trailing   // resolved aggregate glyph
    var doneUntil: Date?     // when a `done` tick should revert (≈ now + 20s)
    var pending: Int         // tasks still running/queued
}
```

### Compact rendering

- **Leading**: Zedra icon (static).
- **Trailing**: one glyph, resolved by priority:
  1. any task `needsInput` → `!`
  2. else a task finished `< doneUntil` → ✓ (system-drawn 20s countdown)
  3. else `pending > 0` → spinner
  4. else → end the activity

`!` outranks ✓ outranks spinner. The done tick holds 20s; if other tasks are still
pending when it expires, trailing reverts to spinner, otherwise the activity ends.

## Lifecycle

- **Start**: first task appears and no activity is live. Delta starts it (push-to-start
  token, iOS 17.2+) or the app starts it locally when foregrounded.
- **Update**: any task event → Delta re-resolves → debounced APNs push
  (≈1/s; `priority 10` for `needsInput`/`failed`, `priority 5` for routine progress).
- **The 20s tick is a Delta-scheduled push.** A backgrounded app cannot self-transition,
  so on `done` Delta schedules a follow-up push at `doneUntil` to revert or end.
- **End**: last task terminal and the final tick drained → Delta sends APNs `end`
  with a dismissal date. User-swipe or system staleness also end it; the app reports
  that back so Delta stops pushing a dead token.
- **Staleness guard**: every push sets `staleDate`; the system 8h/12h caps backstop.

## State sync (app → Delta)

Reported over the app's Delta device channel:

| Event | Reported |
|---|---|
| Activity start | LA push token + `activity_id` |
| `pushTokenUpdates` | rotated token (else stale → APNs 410) |
| `pushToStartTokenUpdates` | push-to-start token (remote restart while app asleep) |
| App launch | `Activity.activities` snapshot — reattach + reconcile |
| User/system dismiss | dismissed/ended → Delta stops pushing |
| Authorization change | `areActivitiesEnabled` |

**Reattach on launch is required**: the activity outlives app kills and the token may
have rotated while the app was down. The app re-enumerates live activities and
re-reports tokens so Delta resyncs.

## Storage (Delta backend)

- `iOSTaskLiveActivity` is a **capability flag** on the device node — advertises that
  the device wants task Live Activities.
- The aggregate **state, tokens, and scheduled timers live in their own Delta store**,
  keyed by node id — not embedded in the node record (high churn), and never on the
  workspace hosts (they cannot aggregate across workspaces).

## Status

- [x] iOS rendering foundation: widget extension + compact/expanded Dynamic Island.
- [x] Local test path: `zedra://la-test/{start,done,needs,end}` deeplinks (no backend).
- [ ] App → Delta token reporting + reattach (needs Delta device channel).
- [ ] Delta aggregator: task-event ingest, resolver, debounce, 20s scheduled push,
  push-to-start, 410 purge (Delta backend repo).
- [ ] Host task-event emission (`task_started/needs_input/task_done/task_ended`).
