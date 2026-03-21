#!/bin/sh
# Zedra — Gemini CLI skill installer
# Installs the zedra CLI and Gemini skills so /zedra-start works in any session.
#
# Usage:
#   curl -fsSL https://zedra.dev/gemini.sh | sh
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

# --- Install Gemini CLI skills ---

install_skills() {
    echo ""

    # Try native gemini skills install first
    if command -v gemini >/dev/null 2>&1; then
        echo "Installing Zedra skills via Gemini CLI..."
        if gemini skills install "https://github.com/${REPO}.git" --path plugins/zedra 2>/dev/null; then
            echo ""
            echo "Done! Open Gemini CLI and run:"
            echo ""
            echo "  /zedra-start"
            echo ""
            return
        fi
        echo "Native install failed, falling back to manual placement..."
    fi

    # Fallback: place SKILL.md files directly
    # Gemini skill discovery paths (from docs):
    #   1. ~/.gemini/skills/   or  ~/.agents/skills/   (user — all projects)
    #   2. .gemini/skills/     or  .agents/skills/      (workspace — this project)
    echo ""
    echo "Where should Zedra skills be installed?"
    echo ""
    echo "  1) User       ~/.gemini/skills/   (available in all projects)"
    echo "  2) Workspace  .gemini/skills/      (this project only)"
    echo ""
    printf "Choose [1]: "

    if [ -t 0 ]; then
        read -r choice
    else
        choice=""
    fi

    case "$choice" in
        2)  skills_dir=".gemini/skills" ;;
        *)  skills_dir="${HOME}/.gemini/skills" ;;
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
    echo "Done! Open Gemini CLI and run:"
    echo ""
    echo "  /zedra-start"
    echo ""
}

# --- Main ---

install_cli
install_skills
