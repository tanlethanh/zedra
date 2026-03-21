#!/bin/sh
# Zedra — Codex CLI skill installer
# Installs the zedra CLI and Codex skills so /zedra-start works in any session.
#
# Usage:
#   curl -fsSL https://zedra.dev/codex.sh | sh
set -eu

REPO="tanlethanh/zedra"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/main"
SKILL_NAMES="zedra-start zedra-status zedra-stop zedra-terminal"

# --- Install zedra CLI ---

install_cli() {
    if command -v zedra >/dev/null 2>&1; then
        echo "zedra CLI already installed: $(command -v zedra)"
    else
        echo "Installing zedra CLI..."
        curl -fsSL "${RAW_BASE}/scripts/install.sh" | sh
    fi
}

# --- Install Codex skills ---

install_skills() {
    # Codex skill discovery paths (from docs):
    #   1. $HOME/.agents/skills/    (user — all projects)
    #   2. .agents/skills/          (repo — this project only)
    echo ""
    echo "Where should Zedra skills be installed?"
    echo ""
    echo "  1) User     ~/.agents/skills/   (available in all projects)"
    echo "  2) Project  .agents/skills/      (this project only)"
    echo ""
    printf "Choose [1]: "

    if [ -t 0 ]; then
        read -r choice
    else
        choice=""
    fi

    case "$choice" in
        2)  skills_dir=".agents/skills" ;;
        *)  skills_dir="${HOME}/.agents/skills" ;;
    esac

    echo ""
    echo "Installing skills to ${skills_dir}/..."

    for skill in $SKILL_NAMES; do
        target="${skills_dir}/${skill}/SKILL.md"
        mkdir -p "$(dirname "$target")"
        if curl -fsSL -o "$target" "${RAW_BASE}/plugins/zedra/skills/${skill}/SKILL.md" 2>/dev/null; then
            echo "  ${skill}"
        else
            echo "  ${skill} (skipped — download failed)"
            rm -f "$target"
        fi
    done

    echo ""
    echo "Done! Open Codex and run:"
    echo ""
    echo "  /zedra-start"
    echo ""
}

# --- Main ---

install_cli
install_skills
