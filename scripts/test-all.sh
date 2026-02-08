#!/usr/bin/env bash
# CI-friendly test runner: runs all Rust tests, integration tests, and
# optionally the Android emulator E2E tests.
#
# Usage:
#   ./scripts/test-all.sh                # Rust tests only (fast)
#   ./scripts/test-all.sh --emulator     # + Android emulator tests
#   ./scripts/test-all.sh --verbose      # cargo test with --nocapture

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

RUN_EMULATOR=false
CARGO_FLAGS=""

for arg in "$@"; do
    case "$arg" in
        --emulator) RUN_EMULATOR=true ;;
        --verbose)  CARGO_FLAGS="-- --nocapture" ;;
        *)          echo "Unknown arg: $arg"; exit 1 ;;
    esac
done

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0

run_step() {
    local name="$1"
    shift
    echo ""
    echo -e "${CYAN}━━━ $name ━━━${NC}"
    if "$@"; then
        echo -e "${GREEN}✓ $name${NC}"
        PASS=$((PASS + 1))
    else
        echo -e "${RED}✗ $name${NC}"
        FAIL=$((FAIL + 1))
    fi
}

cd "$PROJECT_DIR"

# ─────────────────────────────────────────────────────────────────────────────
# Phase 1: Unit tests per crate (fast, no I/O)
# ─────────────────────────────────────────────────────────────────────────────

echo -e "${YELLOW}Phase 1: Unit tests${NC}"

run_step "zedra-rpc unit tests" \
    cargo test -p zedra-rpc $CARGO_FLAGS

run_step "zedra-fs unit tests" \
    cargo test -p zedra-fs $CARGO_FLAGS

run_step "zedra-git unit tests" \
    cargo test -p zedra-git $CARGO_FLAGS

run_step "zedra-host unit tests (lib)" \
    cargo test -p zedra-host --lib $CARGO_FLAGS

# ─────────────────────────────────────────────────────────────────────────────
# Phase 2: Integration tests (spawn servers, use filesystem)
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}Phase 2: Integration tests${NC}"

run_step "SSH integration tests" \
    cargo test -p zedra-host --test integration $CARGO_FLAGS

run_step "RPC E2E tests" \
    cargo test -p zedra-host --test e2e_rpc $CARGO_FLAGS

# ─────────────────────────────────────────────────────────────────────────────
# Phase 3: Android emulator (optional, slow)
# ─────────────────────────────────────────────────────────────────────────────

if [ "$RUN_EMULATOR" = true ]; then
    echo ""
    echo -e "${YELLOW}Phase 3: Android emulator E2E${NC}"
    run_step "Android emulator test" \
        "$SCRIPT_DIR/test-emulator.sh"
else
    echo ""
    echo -e "${YELLOW}Phase 3: Skipped (pass --emulator to enable)${NC}"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo -e "${CYAN}═══════════════════════════════════════${NC}"
echo -e "${CYAN}  Test Summary: ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}"
echo -e "${CYAN}═══════════════════════════════════════${NC}"

[ "$FAIL" -eq 0 ] || exit 1
