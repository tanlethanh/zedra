# Add an Agent

Add agent support on the host first: the actor is the source of truth for
identity, discovery, sessions, setup, account data, and usage. The app adapter
is optional and only covers local behavior (paste formatting, notifications,
icon branding). Agents are stable slug strings over RPC — adding one never
bumps ALPN or adds a protocol enum.

## Host Actor

Every agent is one `AgentActor` implementation in
`crates/zedra-host/src/agent/<slug>.rs`. Register it in `agent/mod.rs`: add
`mod <slug>;`, append `&<slug>::<Name>Actor,` to `ACTORS`, bump the array
size. Every host feature — detection, discovery, sessions, setup, account
data — resolves through that registry; never add per-agent `match` arms to
the REST API, host cache, CLI scans, hook dispatch, or installed-agent list.

`ACTORS` order is the order the app's agent picker shows — put new agents where
they belong, not at the end. Users reorder with `agents.order` in the config
(listed slugs first, the rest keep registry order); `enabled_actors()` is the
single seam that applies both `order` and `disabled`.

Most agents are detect-only: they show up in terminals, version probes, and
the installed-agent list, nothing more. `simple_actor!` is all they need:

```rust
simple_actor!(
    AmpActor,           // actor type name
    "amp",              // slug sent over RPC
    "Amp",              // display name
    "amp",              // icon slug: assets/icons/amp.svg
    ["amp"],            // executables, in preference order
    ["amp", "ampcode"]  // foreground-command aliases
);
```

`programs` drives the `--version` probe and installed-agent list.
`detect_aliases` matches whole words inside the foreground command
(`cursor-agent`, `npx @openai/codex`). For short names that double as normal
words or flag values (`pi`, `hermes`), write the `AgentActor` impl by hand and
use `detect_exact` instead.

Go managed only when the provider supports sessions, resume, setup/hooks,
account data, or usage; the registry in `agent/mod.rs` is the authoritative
list. Override only what the provider supports:

| Feature | Methods | Notes |
| --- | --- | --- |
| Identity & detection | `slug`, `display_name`, `icon_name`, `programs`, `detect_aliases`, `detect_exact` | `slug` is the wire identity; the icon slug may differ for branding |
| Availability | `cli_available`, `cli_version_summary` | Defaults probe `programs()` on PATH |
| Sessions & resume | `session_counts`, `sessions`, `resume_launch_command`, `scan_data_source`, `session_scan_cli` | Custom session-count types register in `session_counts_from!` (`agent/mod.rs`) |
| Setup & hooks | `setup`, `setup_summary`, `supports_setup_cli`, `setup_cli`, `receive_hook`, `hook_test_payload` | `setup` is the only mutable op: writes the hook runner + provider config, returns the paths |
| Account & usage | `account_fields`, `subscription_plan`, `account_usage`, `extra`, `config_files` | Async plan/usage default to `None`; skip the overrides when local-only |
| Behavior flags | `is_global`, `shows_detail` | `is_global`: sessions ignore the workdir (Hermes); `shows_detail`: listed on the app's manage screen |

### Launch commands

`default_launch_command` is the shell command the app runs to start a fresh
session; `resume_launch_command` resumes one. Both default to the resolved
program name, and both are overridden by `agents.overrides.<slug>.launch_cmd` /
`resume_cmd` in the global config. They run as shell script lines, so a
`KEY=value` prefix sets environment for the agent.

Zedra launches agents with permission prompts bypassed by default. A phone is
a poor place to answer approval dialogs — the session stalls until the user
comes back, which defeats the point of supervising agents from mobile. The host
is the user's own machine and the pairing is authenticated, so the trust
boundary is the host, not the prompt. Users who want prompts back set
`launch_cmd`/`resume_cmd` overrides in the global config.

Detect-only agents get their flags from the trailing `simple_actor!` argument;
managed agents override `default_launch_command` (and put the flag in
`resume_launch_command` too).

| Agent | Flag | Notes |
| --- | --- | --- |
| claude | `--dangerously-skip-permissions` | Also `CLAUDE_CODE_NO_FLICKER=0` to stay off the alt screen, so the mobile terminal keeps native scrollback |
| codex | `--dangerously-bypass-approvals-and-sandbox` | Applies to `codex` and `codex resume` |
| opencode | `--auto` | Auto-approves permissions that are not explicitly denied |
| hermes | `--yolo` | Bypasses dangerous-command approval |
| antigravity | `--dangerously-skip-permissions` | CLI binary is `agy` |
| cursor | `--force` | `--yolo` is an alias for it |
| copilot | `--allow-all` | Equals `--allow-all-tools --allow-all-paths --allow-all-urls` |
| grok | `--always-approve` | Also `--no-alt-screen` |
| gemini | `--yolo` | Downgraded to prompting in an untrusted folder; trust it once interactively or set `GEMINI_CLI_TRUST_WORKSPACE=true` |
| amp | none | Bypass is a config setting (`amp.dangerouslyAllowAll`), not a flag |
| openclaw, pi, maki | none | No such flag in their CLIs |
| fx | `FX_PERMISSION_MODE=yolo` | Env var, not a flag; fx reads the permission mode from the environment |
| omp | none | `tools.approvalMode: yolo` is the default; critical destructive patterns and pending provider safety checks still prompt |

### Alt screen

The alternate screen destroys the mobile terminal's native scrollback: the app
can only scroll what the TUI redraws. Agents that expose an inline mode get it
in their launch command; the rest keep their default.

| Agent | Inline mode | Default behavior |
| --- | --- | --- |
| claude | `CLAUDE_CODE_NO_FLICKER=0` | Inline |
| codex | `--no-alt-screen` | Inline |
| grok | `--no-alt-screen` (also `--minimal`) | Inline |
| pi, cursor | none needed | Already inline |
| opencode | `--mini` only | Alt screen — `--mini` is a reduced interface, not a rendering toggle, so it stays opt-in via `launch_cmd` |
| amp, copilot | none | Alt screen |
| hermes | `--cli` (classic REPL) | Config-driven (`display.interface`) |
| omp | none needed | Already inline (pty capture shows no `\e[?1049h`) |
| fx | none needed | Already inline (pty capture shows no `\e[?1049h`) |

Verify a CLI's behavior by running it under a pty and looking for the
`\e[?1049h` (alt screen enter) sequence rather than trusting its help text.

Bypassing approvals also means the agent's approval hook never fires, so
`WaitingApproval` will not appear for these sessions.

Provider-specific hook templates live in the actor's file; `agent/cli.rs`
keeps only shared plumbing (workdir hook script, checked file writers).

### Setup flow (`zedra setup`)

Override `supports_setup_cli()` and `setup_cli(action, ctx)` on the actor; the
command discovers actors through the registry. Handle `Install` and `Remove`
idempotently — agents with nothing to install still explain what setup
provides and state that remove is a no-op. Do everything through the
`SetupCliCtx` — no `println!`, no reaching into `agent::setup`; the ctx
carries the user's flags (`full_bin_path`, `quiet`).

| Function | Use |
| --- | --- |
| `ctx.section(title)` | Heading opening a setup phase |
| `ctx.step(label)` | Step label; follow with `detail` lines |
| `ctx.detail(text)` | Dim outcome line (paths written, commands run) |
| `ctx.message(text)` | Plain line; `""` for a blank separator |
| `ctx.suggest_command(cmd)` | Highlighted command for the user to run next |
| `ctx.require_command(program)` | Error when a provider CLI is not on PATH |
| `ctx.run_step(label, program, args)` | Run a command; error on non-zero exit |
| `ctx.try_step(label, program, args)` | Run a command; `Ok(false)` on failure |
| `ctx.install_plugin_and_hooks(spec)` / `ctx.remove_plugin_and_hooks(spec)` | Whole marketplace-plugin flow from a `PluginSetup` spec (Claude/Codex) |
| `ctx.merge_command_hooks(path, events, agent)` | Upsert Zedra entries in a Claude/Codex-style JSON hooks file |
| `ctx.remove_command_hooks(path, agent)` | Delete only Zedra-owned entries from that file |
| `ctx.install_skills(name, dir)` / `ctx.remove_skills(name, dir)` | Manage Zedra skills under a skills dir |
| `ctx.remove_path(path)` | Remove a file or dir; `Ok(false)` when absent |
| `ctx.hook_binary()` | Binary to embed in hook scripts (`zedra`, or absolute with `--full-bin-path`) |
| `ctx.home_dir()` | User home; errors when `$HOME` is unset |

### Web client (optional)

An agent with a web UI (e.g. `opencode serve`) can expose it as a card the app
opens over the web tunnel in an in-app webview. Override two actor methods:

- `has_web_client() -> true` advertises the affordance (sets
  `InstalledAgentEntry.web_client`, shows the picker globe). Nothing else keys on
  the slug.
- `web_client_open(ctx) -> WebClientOpened { port, path }` opens one card. Launch
  or reuse a server through `ctx.pool.acquire(key, spec)` — the pool is
  agent-agnostic and refcounts by key, so **the actor alone decides process
  topology**: a constant key shares one server across cards, a unique key spawns
  one per card. Do any per-card setup (opencode creates a fresh session), keep
  `ctx.sink` to push the card's live title/state, and return the port plus the
  URL `path` the app should open. `web_client_close(ctx)` releases the pool key.

The manager owns the card registry, ordering, broadcast, and process pool; it
never learns the agent's API, event format, or whether a server is shared. Keep
all of that in the actor — see `agent/opencode.rs` (one shared server, fresh
session per card, `/global/event` demuxed per session id). Protocol contract:
`docs/PROTOCOL_SPECS.md` §5.9.

## App Adapter

The app keys on the host slug. Unknown slugs get `GenericAdapter`: icon from
`assets/icons/<slug>.svg` (fallback `terminal.svg`), display name derived from
the slug, unsupported features degraded.

Add a specialized `AgentAdapter` in `crates/zedra/src/agent/mod.rs` only for
custom app behavior:

- `icon_path`: branding override (Codex uses `icons/openai.svg`)
- `should_notify`: hook event names that raise a notification
- `add_to_chat` / `ask`: custom paste format (Claude uses `@file#Lstart-Lend`)

`native_image_name` derives from `icon_path`, so a branding override carries
to the native picker — never add a second native-image override.

## Ownership

Host owns: agent list, picker data, info, usage, account, session history,
resume, setup, lifecycle hooks, identity detection.

App owns: paste formatting, `should_notify`, icon branding overrides.
Everything else shown in the app comes from host data.

## Icons

One icon slug on every platform: the bare `assets/icons/<icon-slug>.svg` name.
GPUI reads the SVG directly; native UI resolves generated assets from the same
slug (iOS `<icon-slug>.imageset`, Android `ic_<icon_slug>` with hyphens as
underscores).

The icon slug is usually the agent slug; branding exceptions: `codex` →
`openai`, `copilot` → `githubcopilot`, `hermes` → `hermesagent`.

GPUI renders blank for a missing SVG, so check existence before rendering:

```text
icon(slug):
    specialized adapter overrides icon_path() -> that
    else if ZedraAssets::get("icons/{slug}.svg") exists -> that
    else -> "icons/terminal.svg"
```

## Assets

Commit only `crates/zedra/assets/icons/<icon-slug>.svg` (lowercase kebab-case,
`currentColor`). Generated native assets (`ios/Zedra/Assets.xcassets/*.imageset`,
`android/app/src/generated/res/drawable/*.xml`) are gitignored; builds
regenerate them. To inspect iOS imagesets locally:

```sh
scripts/generate-assets.sh
```

## RPC Contract

The live protocol (`zedra/rpc/4`) uses slug strings in `AgentSummary`,
`AgentSessionSummary`, agent session/resume/file requests, and hook events.
Usage display lines are host-formatted into `AgentUsageSnapshot.extra` and
rendered verbatim — keep per-agent display rules host-side.

The frozen `zedra/rpc/3` module (`proto_v3.rs`) still carries the historical
`AgentKind` enum and filters out agents it cannot represent. Do not change
that schema; a new slug alone never bumps ALPN.

## Validation

```sh
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-host
cargo check -p zedra
```

To verify a setup flow end-to-end without touching the real machine, use the
sandbox harness (macOS, `sandbox-exec`):

```sh
scripts/setup-sandbox.sh zedra setup claude
```

It runs the command with a throwaway `HOME`/XDG/`HERMES_HOME`, shim provider
CLIs that log every invocation to `calls.log`, and a Seatbelt profile denying
network and writes outside the sandbox dir (kept and printed for inspection).
Flags: `--shell` for an interactive sandboxed shell, `--no-shims` to keep the
real `PATH`, `--allow-net` to permit network (needed for opencode's skills
download; the run is then no longer hermetic).
