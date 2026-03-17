#!/bin/sh
# Xcode Cloud — ci_post_clone.sh
#
# Runs AFTER the repository is cloned but BEFORE Xcode Cloud resolves
# dependencies (CocoaPods, SPM). This is the correct place to:
#   1. Generate Zedra.xcodeproj from ios/project.yml via xcodegen
#      (pod install, which Xcode Cloud runs automatically, requires an
#       existing .xcodeproj — it will fail without this step)
#   2. Install Rust and the iOS device target
#      (installed here so the cargo cache is warm before ci_pre_xcodebuild.sh)
#
# Xcode Cloud build order:
#   ci_post_clone.sh  ← this file
#   pod install       ← automatic, needs .xcodeproj
#   ci_pre_xcodebuild.sh
#   xcodebuild archive / test
#
# Environment notes:
#   - $CI_PRIMARY_REPOSITORY_PATH  path where the repo was cloned
#   - $CI_DERIVED_DATA_PATH         DerivedData for this build
#   - $HOME/.cargo                  persists across scripts within the same build
#   - Homebrew is pre-installed at /opt/homebrew (Apple Silicon runners)

set -euo pipefail

echo "==> ci_post_clone: working directory: $CI_PRIMARY_REPOSITORY_PATH"
cd "$CI_PRIMARY_REPOSITORY_PATH"

# ── xcodegen ────────────────────────────────────────────────────────────────
# Generate Zedra.xcodeproj so that Xcode Cloud's automatic pod install succeeds.
# Xcode Cloud's CocoaPods support runs `pod install` automatically after
# ci_post_clone.sh if a Podfile is found; it needs a valid .xcodeproj.
echo "==> Installing xcodegen..."
brew install xcodegen

echo "==> Generating Xcode project from ios/project.yml..."
cd ios
xcodegen generate
cd ..

# ── Rust toolchain ───────────────────────────────────────────────────────────
# Install Rust using the minimal profile (no docs / clippy / etc.) to keep the
# install fast. The iOS device target is required by build-ios.sh.
# $HOME/.cargo/env is sourced in ci_pre_xcodebuild.sh to pick up the toolchain.
echo "==> Installing Rust (minimal profile)..."
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --no-modify-path

# shellcheck source=/dev/null
. "$HOME/.cargo/env"

echo "==> Adding aarch64-apple-ios target..."
rustup target add aarch64-apple-ios

echo "==> Rust version: $(rustc --version)"
echo "==> ci_post_clone done."
