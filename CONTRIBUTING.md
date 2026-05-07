# Contributing to Zedra

Zedra is experimental, but it is being built as a long-term, high-quality
developer tool. Contributions are welcome when the problem is clear, the change
is scoped, and the implementation matches the existing design and engineering
standard.

AI tools are allowed, but you must understand and own the code you submit.

Quick links: [Website](https://zedra.dev) · [Discord](https://discord.gg/39MmkSS8sc)

## Start With An Issue

Open an issue before starting most work. State the problem, why it matters, the
platform you tested, and the direction you want to take. Include reproduction
steps, logs, screenshots, or screen recordings when they help.

Small typo fixes and narrow documentation corrections can go straight to a PR.
For anything larger, discuss it first.

Good first issues are small, well-described, and easy to validate. Documentation
fixes, host CLI polish, focused tests, and small bug fixes are good candidates.
Important changes must follow the related project documents. PRs that do not
match the current quality bar, architecture, or design direction may be closed.

## Project Scope

Zedra is a mobile remote editor with a desktop host daemon. The main crates are:

- `zedra`: mobile app shell and workspace UI.
- `zedra-host`: desktop daemon and CLI.
- `zedra-session`: client connection, auth, reconnect, and RPC flow.
- `zedra-terminal`: remote terminal rendering and input.
- `zedra-rpc`: protocol types and pairing.
- `zedra-telemetry`: typed telemetry events and backend interface.

iOS is the primary development target today. Zedra uses `vendor/zed`, a
submodule from `tanlethanh/zed`, for experimental GPUI mobile support. Treat it
as part of the project and discuss `vendor/zed` changes before opening a PR.

Android and Windows support are planned, but current public contribution paths
are macOS/iOS and Linux host-daemon work.

## Development Setup

Clone your fork, initialize submodules, and install Rust targets:

```sh
git clone <your-fork-url> zedra
cd zedra
git submodule update --init --recursive
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

Use current stable Rust. The workspace uses the Rust 2024 edition.

For iOS development on macOS, use Xcode 26 or newer and install:

```sh
sudo xcodebuild -license accept
xcodebuild -downloadComponent MetalToolchain
brew install xcodegen libimobiledevice
sudo gem install cocoapods
cd ios && pod install
```

A personal Apple developer account is enough for local iOS builds. Firebase
credentials are not required. See `docs/GET_STARTED.md` for the full setup.

## Host-Only Development

You can work on the desktop daemon without an iPhone. Important commands:

```sh
cargo run -p zedra-host -- --verbose start --workdir .
cargo run -p zedra-host -- list
cargo run -p zedra-host -- status --workdir .
cargo run -p zedra-host -- logs --workdir .
cargo run -p zedra-host -- client --workdir . --count 5
cargo run -p zedra-host -- qr --workdir . --json
cargo run -p zedra-host -- start --workdir . --relay-only
cargo run -p zedra-host -- stop --workdir .
```

Use Linux or a Linux container for platform-sensitive host-daemon work.

## iOS Development

Open `ios/Zedra.xcworkspace` and build from Xcode for normal iOS work. Prefer a
physical device when changing app behavior. Helper scripts are mainly for logs,
quick rebuilds, and simulator deeplinks:

```sh
./scripts/log-ios.sh --filter zedra
./scripts/run-ios.sh sim --no-build --launch-url 'zedra://connect?ticket=...'
```

After editing `ios/project.yml`, regenerate the Xcode project:

```sh
cd ios && xcodegen generate
```

Use the pairing URL printed by `zedra start` for simulator deeplink testing.

## Code Conventions

Read `docs/CONVENTIONS.md` before writing code. In short:

- Keep changes small and consistent with nearby code.
- Use explicit error handling. Avoid normal-path `unwrap()`, unchecked indexing,
  and silent error drops.
- Use `tracing`, keep GPUI `render()` pure, and keep side effects in handlers,
  subscriptions, or tasks.
- For protocol, transport, telemetry, and UI work, read the related docs first.
- UI changes should match the current design and include screenshots or screen
  recordings in the PR.

## Validation

Initialize submodules first:

```sh
git submodule update --init --recursive
```

Run the broad checks before opening a PR:

```sh
cargo fmt
cargo check --workspace
```

Use focused checks for the area you touched:

```sh
cargo check -p zedra-host
cargo test -p zedra-host
cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host
cargo check -p zedra --features ios-platform --target aarch64-apple-ios
cargo check --manifest-path vendor/zed/Cargo.toml -p gpui_ios -p gpui --target aarch64-apple-ios
```

All PRs must be validated as working. Test iOS changes on iOS. For UI,
platform, reconnect, terminal input, pairing, or gesture changes, update
`docs/MANUAL_TEST.md` with concrete manual verification steps.

## Pull Requests

Use a fork and open PRs against `main` unless a maintainer asks for another base.
Keep PRs small and focused; avoid unrelated cleanup, generated churn, and
dependency updates.

```text
feat|fix|chore|docs: <description>
fix(host): handle stale daemon lock
```

Use the same format for commit subjects and PR titles. PR bodies should use:

```md
## Summary

- ...

## Notes

- ...
```

Add `## Breaking Changes` only when needed.

## Dependencies And Generated Files

Discuss new dependencies before adding them.

Avoid committing generated files unless the repository workflow expects them.
Do not include incidental generated project, lockfile, or header churn in an
unrelated PR.

## Vendor Zed

`vendor/zed` is a git submodule and a maintained GPUI fork for Zedra's mobile
work. Always initialize it before Cargo validation.

Discuss `vendor/zed` changes before implementation. In most cases, GPUI mobile
work should be handled in the `tanlethanh/zed` repository on the
`feat/gpui-mobile` branch, with the Zedra repository updated only after the
submodule change is ready.

## Security

Do not open public issues for vulnerabilities. Email security reports to
`tanle@zedra.dev`.

## Conduct

Be respectful, concrete, and concise. Maintainers may close issues or PRs that
are unclear, spammy, hostile, or repeatedly ignore this guide.
