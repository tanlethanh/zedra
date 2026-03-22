# zedra plugin

Agent plugin for starting and managing the [Zedra](https://github.com/tanlethanh/zedra) Host daemon.
Works with Claude Code, Codex, OpenCode, and any tool supporting the
[Agent Skills](https://agentskills.io) standard.

## Install

### Claude Code

```bash
# Local development
claude --plugin-dir ./plugins/zedra

# Or install from marketplace
# (inside a Claude Code session)
/plugin marketplace add tanlethanh/zedra
/plugin install zedra@zedra
```

### Codex / OpenCode

Copy or symlink the plugin directory so the tool discovers `AGENTS.md` and
`skills/` at the project root or in a recognized plugins path.

Alternatively, point the tool at the skills directory:
```bash
# Codex
codex --skills-dir ./plugins/zedra/skills

# OpenCode
opencode --add-dir ./plugins/zedra
```

## Usage

Plugin skills are namespaced by the plugin name in Claude Code:

| Command | What it does |
|---------|-------------|
| `/zedra:zedra-start` | Check install, launch daemon, print QR |
| `/zedra:zedra-status` | Show daemon status |
| `/zedra:zedra-stop` | Stop the daemon |
| `/zedra:zedra-terminal` | Open a terminal on the connected phone |

## Skills

All skills live in `skills/` as `SKILL.md` files following the Agent Skills standard:

```
plugins/zedra/
├── .claude-plugin/
│   └── plugin.json             # Claude Code manifest
├── AGENTS.md                   # Codex/OpenCode discovery
├── skills/
│   ├── zedra-start/SKILL.md    # Install + launch + QR
│   ├── zedra-status/SKILL.md   # Check daemon health
│   ├── zedra-stop/SKILL.md     # Shutdown daemon
│   └── zedra-terminal/SKILL.md # Open phone terminal
└── README.md
```
