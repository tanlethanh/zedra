#!/bin/sh
# Zedra — OpenCode plugin installer
# Installs the zedra CLI and OpenCode skills so /zedra-start works in any session.
#
# Usage:
#   curl -fsSL https://zedra.dev/opencode.sh | sh
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

# --- Install OpenCode skills ---

install_skills() {
    # OpenCode skill discovery paths (from docs):
    #   1. ~/.config/opencode/skills/<name>/SKILL.md   (global)
    #   2. .opencode/skills/<name>/SKILL.md             (project)
    echo ""
    echo "Where should Zedra skills be installed?"
    echo ""
    echo "  1) Global   ~/.config/opencode/skills/  (available in all projects)"
    echo "  2) Project  .opencode/skills/            (this project only)"
    echo ""
    printf "Choose [1]: "

    # Default to global if non-interactive (piped)
    if [ -t 0 ]; then
        read -r choice
    else
        choice=""
    fi

    case "$choice" in
        2)  skills_dir=".opencode/skills" ;;
        *)  skills_dir="${HOME}/.config/opencode/skills" ;;
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
    echo "Done! Open OpenCode and run:"
    echo ""
    echo "  /zedra-start"
    echo ""
}

# --- Main ---

install_cli
install_skills
