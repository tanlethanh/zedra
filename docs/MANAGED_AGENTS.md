# Add an Agent

Add agent support on the host first. The host actor is the source of truth for
identity, discovery, sessions, setup, account data, and usage. The app adapter
is optional and only handles local behavior such as chat paste formatting,
notifications, and icon branding.

Agents use stable slug strings over RPC. Adding an agent does not change the
ALPN version or add a protocol enum.

## Common Path

Most agents are detect-only. They appear in terminals, version probes, and the
installed-agent list, but they do not manage sessions, setup, hooks, account
state, or usage.

Create `crates/zedra-host/src/agent/<slug>.rs`:

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

Register the actor in `crates/zedra-host/src/agent/mod.rs`:

- add `mod <slug>;`
- add `&<slug>::<Name>Actor,` to `ACTORS`
- bump `static ACTORS: [&dyn AgentActor; N]`

`programs` drives the `--version` probe and installed-agent list.
`detect_aliases` matches whole words or phrases inside the foreground command,
such as `amp`, `cursor-agent`, or `npx @openai/codex`.

Use `detect_exact` for short names that can appear as normal words or flag
values, such as `pi` or `hermes`. `simple_actor!` sets `detect_aliases`; write
the `AgentActor` impl by hand when you need `detect_exact`.

## Managed Agent

Use a managed agent only when the provider supports sessions, resume,
setup/hooks, account data, or usage. Current managed agents are `claude`,
`codex`, `opencode`, `pi`, and `hermes`.

Create `crates/zedra-host/src/agent/<slug>.rs`, implement `AgentActor`, and
register it in `ACTORS`.

Override only the methods the provider supports:

- identity: `slug`, `display_name`, `icon_name`, `programs`,
  `detect_aliases`, `detect_exact`
- availability: `cli_available`, `cli_version_summary`
- sessions: `session_counts`, `sessions`, `resume_launch_command`,
  `scan_data_source`, `session_scan_cli`
- setup/hooks: `setup`, `setup_summary`, `receive_hook`, `hook_test_payload`
- account/usage: `account_fields`, `subscription_plan`, `account_usage`,
  `extra`, `config_files`
- global behavior: `is_global`
- detail screen: `shows_detail` — return `true` to list the agent on the app's
  manage screen (defaults `false`; detect-only actors stay hidden)

`setup` is the only mutable setup operation. It writes the hook runner and
provider config, then returns the written paths.

Return `true` from `is_global` only when sessions ignore the workdir. Hermes is
global.

If the actor carries a custom session-count type, add it to the
`session_counts_from!` macro near the top of `crates/zedra-host/src/agent/mod.rs`.

Do not add per-agent `match` arms to the REST API, host cache, CLI scans, hook
dispatch, or installed-agent list. Those paths resolve actors through
`ACTORS`.

## App Adapter

The app keys on the host slug. Unknown slugs use `GenericAdapter`, which
resolves `assets/icons/<slug>.svg`, falls back to `terminal.svg`, and derives a
display name from the slug.

Add a specialized `AgentAdapter` in `crates/zedra/src/agent/mod.rs` only when
the agent needs custom app behavior.

Override these methods as needed:

- `icon_path`: bundled SVG path when branding differs from `<slug>.svg`; Codex
  uses `icons/openai.svg`
- `should_notify`: hook event names that raise a notification
- `add_to_chat` / `ask`: custom paste format; Claude uses
  `@file#Lstart-Lend`

`native_image_name` derives from `icon_path`, so a branding override carries to
the native picker. Do not add a second native-image override.

App navigation and RPC calls carry the slug as a `String`. Unknown slugs must
degrade to unsupported features.

## Ownership

Keep host-owned behavior on the host:

- agent list, picker data, info, usage, account, and session history
- session resume, setup, and lifecycle hooks
- identity detection and hook-event status

Keep app-owned behavior in the app:

- add-to-chat and ask paste formatting
- `should_notify`
- icon branding overrides

Everything else shown in the app comes from host data.

## Icons

Use one icon slug on every platform: the bare
`assets/icons/<icon-slug>.svg` name.

GPUI reads that SVG directly. Native UI receives the bare slug and resolves
generated assets:

- iOS: `<icon-slug>.imageset`
- Android: `ic_<icon_slug>`, with hyphens converted to underscores

The icon slug is usually the agent slug. It can differ for branding:

- `codex` uses `openai`
- `copilot` uses `githubcopilot`
- `hermes` uses `hermesagent`

`AssetSource::load` returns nothing for a missing SVG, and GPUI renders blank.
Check asset existence before rendering:

```text
icon(slug):
    specialized adapter overrides icon_path() -> that
    else if ZedraAssets::get("icons/{slug}.svg") exists -> that
    else -> "icons/terminal.svg"
```

The host `AgentActor::icon_name` and `AgentSummary.icon_name` use the same icon
slug as native picker assets. `AgentAdapter::native_image_name` strips
`icons/<slug>.svg` to the same slug.

## Assets

Commit only `crates/zedra/assets/icons/<icon-slug>.svg`. Use lowercase
kebab-case and `currentColor`.

Do not commit generated native assets:

- `ios/Zedra/Assets.xcassets/*.imageset`
- `android/app/src/generated/res/drawable/*.xml`

Builds regenerate native assets automatically. Run this only when you need
local iOS imagesets for inspection:

```sh
scripts/generate-assets.sh
```

`bun run icons:gen` runs the same script.

## RPC Contract

The live protocol (`zedra/rpc/4`) uses slug strings in `AgentSummary`,
`AgentSessionSummary`, agent session/resume/file requests, and hook events.

Usage display lines are host-formatted into `AgentUsageSnapshot.extra` and
rendered verbatim. Keep per-agent display rules host-side.

The frozen `zedra/rpc/3` module (`proto_v3.rs`) still carries the historical
`AgentKind` enum and filters out agents it cannot represent. Do not change that
schema. Adding an actor can require icon assets and manual-test steps, but it
must not bump ALPN only because a new slug exists.

## Validation

```sh
cargo fmt
cargo check -p zedra-rpc -p zedra-session -p zedra-host
cargo check -p zedra
```
