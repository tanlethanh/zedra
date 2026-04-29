#!/bin/sh
# Zedra — Codex setup compatibility wrapper
#
# Usage:
#   curl -fsSL https://zedra.dev/codex.sh | sh
set -eu

REPO="tanlethanh/zedra"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/main"

install_cli() {
    if command -v zedra >/dev/null 2>&1; then
        echo "zedra CLI already installed: $(command -v zedra)"
    else
        echo "Installing zedra CLI..."
        curl -fsSL "${RAW_BASE}/scripts/install.sh" | sh
    fi
}

zedra_bin() {
    if command -v zedra >/dev/null 2>&1; then
        command -v zedra
    elif [ -x "${HOME}/.local/bin/zedra" ]; then
        printf "%s\n" "${HOME}/.local/bin/zedra"
    else
        echo "zedra CLI was installed, but it is not in PATH." >&2
        echo "Add ~/.local/bin to PATH, then run: zedra setup codex" >&2
        exit 1
    fi
}

install_cli
"$(zedra_bin)" setup codex
