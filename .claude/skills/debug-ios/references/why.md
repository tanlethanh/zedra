# Why: devtool/log-daemon design decisions

Supporting rationale for Phase 0 steps — read when something doesn't work as
expected or need justify a deviation. Not needed for normal execution.

## Why console-attach, not idevicesyslog/`log stream` (both platforms)

idevicesyslog can't locally decode third-party binary compact log entries —
even with privacy fix in `crates/zedra/src/ios/logger.rs`, shows
`<decode: missing data>` for our messages on real devices.
`xcrun simctl spawn log stream` (simulator) hits the exact same decode
failure — confirmed live, not just device. `devicectl device process launch
--console` / `simctl launch --console` sidestep this entirely (no sudo, no
pymobiledevice3) since they capture raw stdout/stderr, not unified logging —
Zedra's iOS logger writes to stderr for exactly this reason. Both relaunch
the app fresh each `start` — that's why Phase 0 step 2 must run before
devtool bridge, on either platform.

## Why devtool responses carry a `pid`, and bridge-ios can refuse to bridge

The devtool port (9777) is a single fixed number. A simulator running
`--devtool` binds it directly; nothing stops a physical-device `iproxy`
tunnel from binding the same port at the same time. When that happens,
whichever one "localhost" resolves to answers — silently, with `/ping`
succeeding and `/elements` returning valid-looking JSON either way. This
was reproduced live during a self-review: `/elements` returned a different
device's screen resolution while driving the simulator, no error anywhere.
Fix: every devtool response includes `pid` (`std::process::id()`), and
`bridge-ios` checks it before starting a new device tunnel — if something
it doesn't own already answers on the port, it refuses instead of layering
a second listener on top. It also drops its own tracked tunnel when the
target switches (sim → device or device → device), so a stale tunnel from
a previous target can't linger and collide with the new one.

## Why stale daemon looks like real signal

Daemon attached to process a rebuild just killed (or about to kill) silently
freeze on last-captured line. Query it reads old cached content that look
like real signal but isn't — a confound you don't want mid-investigation.
Hence: stop → sleep 2 → start after any rebuild, never bare start.

## Why --devtool always pairs with --debug

`--debug` enables `debug-logs` feature (`crates/zedra/src/ios/logger.rs`) —
what makes `tracing::info!`/`warn!` calls emit at all. Without it, every
`[debug:<topic>]` line this skill tells you add is silently dropped, not
just filtered by log daemon. No realistic case where this skill wants
`--devtool` without `--debug`.
