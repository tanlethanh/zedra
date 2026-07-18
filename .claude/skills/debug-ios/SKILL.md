---
name: debug-ios
description: Self-debug iOS app end to end — verify device/simulator, devtool, log daemon work, plan root-cause investigation, drive app via synthetic taps/gestures while capture timestamped logs, report findings back to human. Use whenever user report bug, crash, hang, stuck spinner, wrong behavior in iOS app, or ask reproduce/investigate/diagnose something on iOS, check device/simulator logs, or drive UI without touch phone self — even if no say "debug" explicit (e.g. "the reconnect spinner never stops", "workspace card does nothing when tapped", "why did it crash on my phone", "check what's happening on my iPhone").
allowed-tools: Bash, Read, Edit, Grep, Glob, AskUserQuestion, Agent
---

# Debug iOS

Autonomous iOS debug loop: verify environment, plan investigation, drive app while capture logs, report back. Build on `docs/DEVTOOL.md` (in-app tap/touch driver, iOS + Android),
`scripts/ios-log.sh daemon` (background log capture queryable by time range),
and `scripts/ios-log.sh wait` (poll for expected log line, no sleep-and-hope).

Invoke with bug description + optional flags:
`/debug-ios <bug description> [--direct] [--no-devtool] [--perf]` — see
"Flags" below for what each change.

## At a glance

0. **Verify** — device connect, log daemon capture fresh timestamp output,
   devtool reachable (built `--devtool --debug`).
1. **Plan** — hypothesis from read code first + ordered action steps +
   logs to add.
2. **Loop** — action → capture → evaluate → next, until root cause confirm
   or the first failed attempt requires more information (see below).
3. **Summary** — report to human: what wrong, what tried, the fix, cleanup
   status, assumptions, recommendations, new concepts, proposals.

Each phase run as own subagent by default (`--direct` inline instead).
Iron Law + Output hygiene apply every phase — read those next.

## Iron Law

**NO FIX WITHOUT VERIFIED ROOT CAUSE. NO "DONE" WITHOUT RE-OBSERVE FIX
WORKING.** Never assume — every hypothesis check against real
log line or real screenshot/element query before act on it. Can't verify, say so and ask for the missing information; don't guess-and-ship. Token
cost not reason to stop loop — wrong fix cost user far more
than another verify pass.

## Flags

- `--direct` — run every phase (verify/plan/loop/summary) inline in main
  conversation instead of delegate to subagents. Use for quick, narrow
  bug where overhead of spawn agents not worth it. Default is
  subagent-delegated (see "Why subagents" below).
- `--no-devtool` — skip action-step loop entirely; only add logs, ask
  human reproduce manually, read back capture. Use when iOS devtool
  not built into current app binary and rebuild not wanted yet.
- `--perf` — this performance investigation, not correctness bug.
  Switch Phase 2 logging to per-frame + aggregation-key pattern instead of
  state-change logging — see `references/perf-mode.md`.

## Why subagents

Each phase below dispatch to subagent by default (`Agent` tool, one
per phase, foreground since each phase output gate next). Subagent
get **plan + expected result**, do work (shell commands, log reads,
devtool calls), return **findings + logs tried + summary** — not
transcript. Keep main conversation context to "what learn and what next," not "every curl call and grep." Main agent
(you, in parent conversation) read each subagent report, decide
next action — you accumulate cross-phase picture, not
any single subagent.

`--direct` collapse this: you do work self in main thread. Use
when spawn agent cost more than save.

## Output hygiene

Apply everywhere in this skill — subagent reports, `--direct` mode, every
phase. Subagent isolate verbose work in own context only help if
**returned summary** don't just re-paste verbosity into
parent conversation; that the actual leak, not tool calls self.

- **Never paste raw JSON into response.** `devtool.sh elements` return
  full frame dump — parse it, state 1-3 field that matter
  ("`ws-card-0` at x=51,y=345"), not payload. Default to
  `devtool.sh list` (already concise table) for routine "what's on
  screen" checks; reach for raw `elements` only when need exact
  coordinates for script, and even then quote only relevant entry.
- **Never paste more than ~10-15 raw log lines.** `ios-log.sh query`/
  `ios-log.sh wait` output real signal, but response to human
  should be "N lines matched `<pattern>`; relevant ones: ..." with just
  those lines — not full scrollback. Need scan more to find
  signal, do scanning, then report finding, not scan.
  Redirect wide captures to file if need grep repeatedly.
  `ios-log.sh wait` already return one line for exactly this reason —
  prefer over `ios-log.sh query` when have specific pattern in
  mind, not open-ended window to eyeball.
  Never `cat` raw capture file or forward unfiltered contents into
  final report; only include specific matched lines support the
  finding making.
- **Command output (build logs, `ps aux`, curl `-v`) summarize to
  pass/fail + one relevant line**, not paste wholesale. Build either
  succeed or fail at specific step with specific error — that
  report, not scrollback.
- This hard rule for subagent reports specific: "findings + logs
  tried + summary" (above) mean summary state what found, with
  short supporting quotes — not transcript replay, parent
  agent should not need re-summarize subagent already-verbose report.

## Phase 0: Verify

Dispatch (or, with `--direct`, run inline):

1. **Device connect.** `idevice_id -l` must list UDID (physical device)
   or booted simulator must exist (`xcrun simctl list devices booted`).
   Neither, STOP, ask human which target use — don't guess.

2. **Log daemon, before devtool bridge — both platforms now.** Start
   daemon there launch app via `devicectl --console` (device) or
   `simctl launch --console` (simulator) — this **replace whatever
   instance already running**, on EITHER platform, so step must come
   before devtool bridge, or you'll bridge to process about to die. Why
   this backend, not idevicesyslog/`log stream`: `references/why.md`.
   - **After any rebuild/install: `stop` → `sleep 2` → `start`** — never
     bare `start` on possibly-still-running daemon (why: `references/why.md`).
     `status` already say "not running" → plain `start` fine.
   - `./scripts/ios-log.sh daemon start` (add `--filter` if bug in
     subsystem default filter would miss — see "Log filter scope"
     below).
   - Verify it work: wait ~3-5s (device need time relaunch + log),
     `./scripts/ios-log.sh query --since 10s`, confirm non-empty output
     **with `Mon DD HH:MM:SS` timestamp actual recent, not
     stale**. Empty or stale = verify FAILURE, not shrug.

3. **Devtool reachable**, unless `--no-devtool`:
   - Check for `devtool: listening on 127.0.0.1:9777` in log capture,
     or just try ping, treat failure as "not built with --devtool."
   - `./scripts/devtool.sh bridge-ios` (or `bridge` on Android), then
     `./scripts/devtool.sh ping`.
   - Ping fails → rebuild with **`--devtool --debug` together, always**
     (`./scripts/run-ios.sh sim --devtool --debug` or
     `device --devtool --debug`) — why `--debug` always pairs with
     `--devtool`: `references/why.md`. Re-verify after rebuild (and redo
     step 2's stop→start, since a device rebuild relaunches too). Tell the
     human you're rebuilding — don't silently fall back to `--no-devtool`.

Verify subagent return: device target (UDID/sim), devtool status
(reachable / rebuilt / skipped), log daemon status (already running /
started / **failed — stop here**). Verify fail, can't self-resolve
(e.g. no device at all), STOP, ask human — don't proceed to Phase 1
on unverified environment.

### Log filter scope

Daemon pre-filter at capture time
(`Zedra|zedra|devtool|panic|PANIC|crash|CRASH|fault|error` by default) —
anything not match never write to disk, can't be recover by
`ios-log.sh query` later. Bug live in subsystem outside filter (e.g. need
raw `CommCenter` or `backboardd` lines), stop daemon, restart it with
broader `--filter` **before** start repro loop, not after. Physical-device
devtool bridging needs the `devtool` keyword specifically — that's where
`devtool: token: ...` (neither "Zedra" nor "zedra" appears in it) lives,
and `devtool.sh`'s token fallback greps this same capture file.

## Phase 1: Plan

Dispatch subagent with: bug description, verify results,
instructions produce structured plan:

1. **Hypothesis** — best guess what wrong and why, ground in read
   relevant code path first (per `AGENTS.md` — read before change
   behavior). Name specific file/function suspect.
2. **Action steps** — ordered list, each one of: navigate (devtool
   `tap`/`long-press`/`tap-xy` to specific element or coordinate), scroll,
   focus/type, or "wait for state X." Each step name:
   - devtool call (or manual action if `--no-devtool`)
   - **expected resulting state** (what element/screen should appear —
     confirm via `elements`/`list` or a screenshot)
   - **expected log signal** (what log line, if any, should show up)
   - if step target `WKWebView`, native alert/action sheet, or system
     keyboard: say so explicit, note "no `/elements`, no screenshot on
     physical device without human — success infer from logs only" (see
     `docs/DEVTOOL.md` § Limits). Not GPUI elements, never appear in
     hitbox snapshot; don't let Phase 2 discover by trial and error.
3. **Logs to add** — any `tracing::info!` instrument needed before
   repro will be diagnostic, with feature-prefix convention (see below).
   Prefer read existing logs first; only add new where existing
   signal insufficient.

Plan is contract for Phase 2 — each loop iteration check self
against "did step N produce expected state and log signal," not vibes.

## Phase 2: Loop

For each action step, dispatch subagent (or run inline with `--direct`)
that do: **action → capture → evaluate → next**.

1. **Action.** Perform step via `./scripts/devtool.sh press <leaf>` /
   `long-press <leaf>` / `tap-xy <x> <y>` / `call <name> <params>` / etc.
   (or ask human perform, if `--no-devtool`). Call block until touch/call
   itself done (`"completed":true` in response) — no fixed sleep need for
   gesture itself. Short settle wait (~0.3-0.5s) still need only if step
   trigger app-level animation (drawer slide, screen transition) whose end
   state depend on — downstream of touch, block don't cover that
   (`docs/DEVTOOL.md` § Limits). `"completed":false` = timeout, treat
   unconfirmed, not "definite fail."
2. **Capture.** `./scripts/devtool.sh list`/`elements` for UI state. For
   expected log signal, use `./scripts/ios-log.sh wait '<pattern>' --timeout
   Ns` instead of fixed `sleep` + `ios-log.sh query` — it poll until
   pattern appear or timeout elapse, return immediate either way,
   which remove sleep-N-then-query guess game (query too early and
   genuinely-absent signal look same as "hasn't happen yet"; sleep
   too long waste loop time). Timeout here (no match) itself
   evidence — mean expected signal genuine didn't fire, not
   that didn't wait long enough. Fall back to
   `./scripts/ios-log.sh query --since <step start time>` for open-ended
   review of everything in window, not single expected pattern.
3. **Evaluate — check for interruption before trust result:**
   - Does `devtool list`/`elements` show **expected screen**, not some
     other one? Wrong-screen result usually mean prior step didn't
     land, modal/permission prompt intercept focus, or user
     interact with physical device concurrent. Don't proceed
     assume step work — confirm expected element present.
   - Do logs show **expected signal**, and nothing indicate
     confound (network error, reconnect, unrelated crash)? Confound
     invalidate this iteration evidence — note it, don't build on it.
   - If interrupted/confounded: back up, re-establish expected starting
     state, retry step. Don't paper over broken assumption by
     reinterpret next step expectations.
4. **Next.** Root cause confirm (hypothesis match observed
   behavior with log evidence), move to fix it (see below). If the first
   attempt does not reproduce or explain the bug, stop and ask for the
   specific information needed to continue (see below).

### Applying a fix

Once root cause verify:

1. Make smallest change that fix it (per `AGENTS.md`).
2. Re-run exact action steps that reproduce bug, confirm
   expected (now-correct) behavior — **re-observe, don't assume fix
   work because look right in diff.**
3. Track fix in loop summary (see below) as **confirmed** — vs.
   tentative/arbitrary change that turn out not matter, should
   mark for removal, not left in diff as unexplained noise.

### Debug log conventions

- Rust: `tracing::info!("[debug:<topic>] ...")` — never `tracing::debug!`
  (project convention: `docs/CONVENTIONS.md`, `[[feedback-logging]]`).
  `<topic>` short, greppable feature slug (e.g. `workspace-terminal`,
  `drawer-anim`), consistent across all lines add for this investigation
  so `ios-log.sh query --filter '\[debug:topic\]'` isolate them.
- Log **state changes, navigation, lifecycle events** — not every frame,
  not full data dumps. Log line should answer "did X happen," not "here
  is entire state."
- **Remove all `[debug:<topic>]` lines once bug fixed and confirmed**
  (or once hypothesis ruled out, instrumentation no longer
  needed). Debug logging scaffolding, not documentation — must not
  ship. Track every add so cleanup in Summary phase checklist, not
  `git grep` fishing expedition.

### Performance instrumentation (`--perf`)

Different shape from correctness debug — per-frame logs expected here, but
only structured for aggregation. See `references/perf-mode.md`.

### When to stop and ask

Stop loop, use `AskUserQuestion` (or plain text if specifics
don't fit multiple-choice question) when:

- The first debugging attempt does not reproduce or explain the bug. State
  what the attempt established and ask for the specific missing action,
  account state, timing, or logs needed to continue.
- Form plausible root cause but fix require
  product/architecture call (e.g. "this need data-model change" vs "this
  need one-line guard") — decision belong to human, per
  `AGENTS.md`'s "ask before meaningful product or architectural decisions."

When stop: say plain what tried, what ruled out, what
need from human (specific action, or specific decision). Then
**wait** — don't keep loop on guesses, don't silently pick
architecturally bigger option to "be thorough." Human come back
with missing action or decision, resume loop from where left
off; don't restart Phase 0.

### Tracking tentative fixes

Keep running ledger (in working notes, not committed anywhere) of
every change made during investigation, each tag:

- **confirmed** — reproduce-fixed via fresh action-step pass after
  change land.
- **tentative** — made while chase hypothesis that didn't pan out, or
  made before full confirmation. Need review before fix consider
  done: either turn out matter (promote to confirmed, keep) or
  doesn't (revert it — don't leave speculative changes in diff).

Summary phase turn this ledger into human-facing report.

## Phase 3: Summary

Dispatch subagent (or write direct with `--direct`) produce report
for human — one who own design/architecture decisions and need
follow this without re-read whole transcript:

1. **What wrong** — confirmed root cause, in plain terms, with
   file/function.
2. **What tried and invalidated** — hypotheses ruled out and why (so
   next person don't re-walk same dead ends).
3. **The fix** — what change, why smallest viable change, and
   re-observe evidence it work.
4. **Cleanup status** — every `[debug:<topic>]` line add, and whether
   removed; every tentative change and confirmed/reverted
   status. Nothing left ambiguous here.
5. **Assumptions made along way** — anything treated as true
   without full verify (e.g. "assumed reconnect path behave same
   on Wi-Fi vs cellular — not independently tested").
6. **Recommendation, not decision** — if fix reveal deeper
   structural issue (e.g. "this class of bug recur until X
   refactor"), say so, give reasoning, but frame as
   recommendation for human decide on, per `AGENTS.md` — don't
   unilateral scope refactor into this fix.
7. **New concepts surfaced** — if investigation introduce
   domain/keyword/aspect not previously discussed (new subsystem,
   undocumented invariant, naming inconsistency), call out explicit
   as own bullet, separate from bug narrative — this the
   "separate mind" pass: don't let new concept get bury inside fix
   description where missed.
8. **Autonomous-flow proposals** — if, during investigation,
   found self want capability didn't exist yet (e.g. reusable
   capture-and-diff harness, specific new devtool endpoint), say
   so as proposal. Don't build it silent as part of this fix unless
   human confirm in scope.

## Notes

- This skill assume `docs/DEVTOOL.md`'s shared `gpui_devtool` crate is
  built into running app (`--devtool` feature). App predates
  that, Phase 0 catch it, trigger rebuild.
- `scripts/ios-log.sh daemon` use one fixed capture location per repo
  checkout (not session-scope) — `stop`/`status` always find daemon
  `start` create, regardless of shell context. Use `--tag <name>` on all
  three subcommand only if deliberate need more than one concurrent
  capture. Device/simulator *selection* reads one global pref file
  (`/tmp/zedra-ios-device`, no `$PPID` — dropped that scoping because a
  child script sees its invoker's PID, not the invoker's own `$PPID`,
  which let `devtool.sh` and this daemon silently diverge on target).
  `run-ios.sh`/`ios-log.sh`/`devtool.sh` all read/write the same file, so
  they agree by construction. Missing entirely (never ran `run-ios.sh`)
  → falls back to enumerating physical devices via `idevice_id -l`, which
  won't see a simulator. Daemon on the wrong target → pass
  `--select-device` or check what `run-ios.sh` actually saved.
- `devtool.sh bridge-ios` also guards against a **port collision**: the
  simulator binds `127.0.0.1:9777` directly whenever it's running
  `--devtool`, and nothing stops a physical-device `iproxy` tunnel from
  binding the same port at the same time — whichever one "localhost"
  resolves to wins silently, with `/ping` succeeding either way. Every
  devtool response now carries a `pid`; `bridge-ios` checks it before
  bridging a device and refuses if something it doesn't own already
  answers there, rather than layering a second listener on top.
- For Android, same devtool surface apply via `./scripts/devtool.sh
  bridge` and `./scripts/android-log.sh`/`perf-debug.sh`; this skill
  iOS-first per `AGENTS.md`'s platform scope, but Phase 0-3 structure
  transfer direct if ask debug Android instead.

## Reference files

Not needed for normal execution — read only when the pointer in the body
sends you here.

- `references/why.md` — console-attach-vs-unified-logging + stale-daemon +
  `--devtool`/`--debug` pairing + devtool port-collision/`pid` rationale
  (Phase 0).
- `references/perf-mode.md` — `--perf` per-frame aggregation logging
  pattern (Phase 2, `--perf` only).
