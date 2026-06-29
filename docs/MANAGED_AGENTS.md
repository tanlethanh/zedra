# Adding an Agent

Agents are actor-based on both sides of the connection. The host side is the
**actor** (`AgentActor`, `crates/zedra-host/src/agent/`); the app side is the
**adapter** (`AgentAdapter`, `crates/zedra/src/agent/`). There is no agent enum
in the live RPC protocol — an actor is identified by its stable slug, sent over
the wire as a `String`. Adding an agent never changes the ALPN version, touches
a match table, or adds a detection word list.

Adding an agent is at most three steps:

1. **Host actor** (`crates/zedra-host/src/agent/<slug>.rs`) — identity and the
   probes the host supports. Required.
2. **App adapter** (`crates/zedra/src/agent/mod.rs`) — optional; only for custom
   in-app behavior or icon branding. A plain agent gets the generic adapter.
3. **Assets** — `icons/<slug>.svg` and the native picker image. Both fall back
   to a generic icon, so a new agent renders before assets ship.

## Ownership

Each feature is owned by the side where it must run.

**Host** (source of truth, sent to the app as data):

- agent list / picker, info, usage, account, session history
- session resume, setup, lifecycle hooks
- identity detection (foreground command → slug) and status
  (running / waiting / idle, from hook events)

**App** (needs local latency, runs in-process):

- add-to-chat / ask — normalize a file range into the agent's prompt format and
  paste it
- `should_notify` — which hook events raise a notification
- icon branding overrides

Everything else the app shows comes from host data.

## Detect-only agent

Recognized in terminals (icon, version probe, installed-agent list) but no
managed sessions, setup, or hooks. Most agents (`amp`, `cline`, `cursor`,
`gemini`, ...) are this — one macro line in its own file
`crates/zedra-host/src/agent/<slug>.rs`:

```rust
simple_actor!(
    AmpActor,           // actor type name
    "amp",              // slug: stable identity, sent over RPC
    "Amp",              // display name
    "amp",              // icon slug: assets/icons/<slug>.svg, same name on every platform
    ["amp"],            // programs: executables that launch it, preference order
    ["amp", "ampcode"]  // detect aliases: substrings matched in the foreground command
);
```

Register it in `crates/zedra-host/src/agent/mod.rs`: add `mod <slug>;`, add
`&<slug>::<Name>Actor,` to the `ACTORS` array, and bump the array length
`static ACTORS: [&dyn AgentActor; N]`.

`programs` drives the `--version` probe and the installed-agent list.
`detect_aliases` match as whole words/phrases inside the foreground command, so
they handle `amp`, `cursor-agent`, `npx @openai/codex`. For short tokens that
double as common words or flag values (`pi`, `hermes`), use `detect_exact`
instead — those match only when they are the entire command. The macro sets
`detect_aliases`; to set `detect_exact`, hand-write the `impl` instead of using
the macro.

## Fully integrated agent

Managed sessions, resume, setup/hooks, account, and usage — only `claude`,
`codex`, `opencode`, `pi`, `hermes`. Create
`crates/zedra-host/src/agent/<slug>.rs` and implement `AgentActor` by hand;
register it the same way (`mod`, `ACTORS`, length bump).

`AgentActor` defaults every optional operation to unsupported, so override only
the methods the provider actually exposes:

- identity — `slug`, `display_name`, `icon_name`, `programs`, `detect_aliases` /
  `detect_exact` (required identity; the rest are optional)
- availability — `cli_available`, `cli_version_summary`
- sessions — `session_counts`, `sessions`, `resume_launch_command`,
  `scan_data_source`, `session_scan_cli`
- setup/hooks — `setup` (the single mutable op: writes the hook runner and
  provider config, returns written paths), `setup_summary`, `receive_hook`,
  `hook_test_payload`
- account/usage — `account_fields`, `subscription_plan`, `account_usage`,
  `extra`, `config_files`
- `is_global` — return `true` only for agents whose sessions ignore the workdir
  (Hermes)

Per-agent session-count types convert into the shared `SessionCounts` via the
`session_counts_from!` macro near the top of `agent/mod.rs`; add a line for your
type if you carry one.

The local REST API, host cache, CLI scans, hook dispatch, and installed-agent
list all resolve actors through the `ACTORS` registry. Do not add per-agent
`match` arms at those call sites.

## App adapter

The app keys on the host slug and needs no per-agent code by default:
`adapter()` returns a `GenericAdapter` for any unknown slug, resolving the icon
from `assets/icons/<slug>.svg` (falling back to `terminal.svg`) and the display
name from the slug.

Add a specialized `AgentAdapter` only for custom branding or chat behavior, then
register it in the `adapter()` match. Override the relevant methods:

- `icon_path` — bundled SVG when it differs from `<slug>.svg` (Codex uses
  `openai.svg`). `native_image_name` derives from `icon_path`, so a branding
  override carries to the native picker automatically — no second override.
- `should_notify` — which provider hook event names raise a notification
- `add_to_chat` / `ask` — custom paste format (Claude uses `@file#Lstart-Lend`
  mentions instead of the generic fenced context)

App navigation and RPC calls carry the slug as a `String`; an unknown slug must
degrade to an unsupported feature, never require a new protocol variant.

## Icon resolution

One icon name on every platform: the bare `assets/icons/<slug>.svg` slug. GPUI
renders the embedded SVG; iOS/Android resolve the generated imageset/drawable of
the same name (see AGENTS.md "Icon Assets"). The icon slug is usually the agent
slug, but branding diverges it for a few (`codex` -> `openai`, `copilot` ->
`githubcopilot`, `hermes` -> `hermesagent`).

`AssetSource::load` returns nothing for a missing SVG and GPUI renders blank —
no automatic fallback — so the GPUI path checks existence at the call site:

```
icon(slug):
    specialized adapter overrides icon_path() -> that            # branding
    else if ZedraAssets::get("icons/{slug}.svg") exists -> that  # slug convention
    else -> "icons/terminal.svg"                                 # generic fallback
```

`ZedraAssets::get` (rust-embed) is a compile-time-embedded lookup, so the check
is free. Branding overrides must be struct-based: the bundle ships both
`codex.svg` and `openai.svg`, so the slug default would pick the wrong one —
Codex keeps a small adapter purely to override `icon_path()`.

The host `AgentActor::icon_name` / `AgentSummary.icon_name` is this same icon
slug, used as the native picker asset name. `AgentAdapter::native_image_name`
strips `icon_path` to the same slug, so host hint and app derivation always
agree.

## Assets

- Author one SVG: `crates/zedra/assets/icons/<slug>.svg` (lowercase slug). It is
  the single committed source on every platform; required for the generic
  adapter to show a real icon.
- iOS imagesets and Android drawables are **generated** from it at build time
  (gitignored) — do not hand-add `Agent*.imageset` or `agent_*.xml`. Run
  `scripts/generate-assets.sh` (or `bun run icons:gen`) for the iOS/GPUI preview.

## RPC contract

The live protocol (`zedra/rpc/4`) uses slug strings in `AgentSummary`,
`AgentSessionSummary`, agent session/resume/file requests, and hook events.
Usage display lines are host-formatted into `AgentUsageSnapshot.extra` and
rendered verbatim, so per-agent display rules stay host-side. The frozen
`zedra/rpc/3` module (`proto_v3.rs`) still carries the historical `AgentKind`
enum and filters out agents it cannot represent — do not change that frozen
schema. Adding an actor may need new icon assets and manual-test steps, but it
must not bump the ALPN version solely because a new slug exists.

## Validation

```sh
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-host
cargo check -p zedra
```
