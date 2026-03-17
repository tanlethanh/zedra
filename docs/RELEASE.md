# Releasing zedra-host

## How to cut a release

1. Bump the version in `Cargo.toml` (workspace root):
   ```toml
   [workspace.package]
   version = "0.2.0"
   ```

2. Commit, tag, and push:
   ```bash
   git add Cargo.toml
   git commit -m "chore: release v0.2.0"
   git push origin main
   git tag v0.2.0
   git push origin v0.2.0
   ```

   > **Do not use `git push --tags`** — this repo has 400+ tags pulled from the
   > upstream zed remote. Pushing them all would fail with HTTP 400.

Pushing the tag triggers `.github/workflows/release.yml`, which:
- Builds `zedra` for macOS arm64/x86_64 and Linux x86_64/aarch64
- Packages each binary as `zedra-<target>.tar.gz` with a `.sha256` checksum
- Creates a GitHub Release with all artifacts and auto-generated notes

## How users install / update

**First install:**
```bash
curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh
```

**Specific version:**
```bash
curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh -s -- --version v0.2.0
```

**Update** — run the install script again; it overwrites the existing binary.

The script installs to `~/.local/bin/zedra` by default. Override with `--prefix /usr/local/bin` or `ZEDRA_PREFIX`.

## Verify a release locally

```bash
cargo build -p zedra-host --release
./target/release/zedra --help
```
