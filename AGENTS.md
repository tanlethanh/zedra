# AGENTS.md

## Cursor Cloud specific instructions

### Services overview

| Service | Description | Run command |
|---|---|---|
| **zedra-host** (Rust binary) | Desktop companion daemon — iroh P2P, RPC, PTY, git/fs ops | `cargo run -p zedra-host -- start` |
| **Landing page** (Astro) | Marketing site at `packages/landing/` | `bun run --cwd packages/landing astro dev` |
| **Relay monitor** (Bun/TS) | Docker sidecar at `packages/relay-monitor/` | `deploy/relay/` |
| **Relay check** (Bun/TS) | Local SSH health CLI at `packages/relay-check/` | run on your machine |

The Android app (`crates/zedra/`) and iOS build require NDK/Xcode and a physical device — not buildable in Cloud Agent VMs.

### Pre-commit checks

See `CLAUDE.md` → "Pre-Commit Checks" for the canonical commands. Summary:

- **Rust**: `cargo fmt --check` then `cargo check -p zedra-rpc -p zedra-session -p zedra-terminal -p zedra-host`
- **JS/TS**: `bun run check` (runs `biome ci packages/`)

### Testing

- `cargo test -p zedra-host` — 41 unit tests (1 pre-existing failure: `git::tests::checkout_branch`)
- No integration test binary is built by default; integration tests live under `tests/integration.rs` and require `--test integration`

### Gotchas

- **Rust edition 2024**: workspace requires Rust >= 1.85. The update script ensures `rustup update stable`.
- **`libssl-dev`**: required for `openssl-sys` crate (dependency of iroh). Must be installed as a system package.
- **Bun** (not npm/yarn/pnpm): JS/TS workspace uses `bun.lock`. Install via `bun install` at repo root.
- **Git submodule `vendor/zed`**: must be initialized (`git submodule update --init --recursive`) — required even for host-only crates because `zedra-terminal` depends on `gpui` from the submodule.
- **`zedra status` / `zedra list`** subcommands have a pre-existing Tokio runtime panic — use `zedra start` and `zedra stop` instead.
- The binary name is `zedra` (not `zedra-host`): `./target/debug/zedra --help`.
