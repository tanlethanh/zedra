# Zedra Plugin

This plugin provides skills to manage the Zedra Host daemon — a desktop companion
that lets you control your development environment from the Zedra mobile app.

## Available Skills

| Skill | Description |
|-------|-------------|
| `zedra:zedra-start` | Check/install zedra CLI, launch daemon, print QR for mobile pairing |
| `zedra:zedra-status` | Show daemon status and active sessions |
| `zedra:zedra-stop` | Stop the running daemon |
| `zedra:zedra-terminal` | Open a new terminal on the connected phone |

## Prerequisites

- **zedra CLI**: Installed automatically by the `start` skill, or manually via:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/tanlethanh/zedra/main/scripts/install.sh | sh
  ```

## Claude Code Installation

```
/plugin marketplace add tanlethanh/zedra
/plugin install zedra@zedra
```
- **Zedra mobile app**: Install on your Android/iOS device to scan the pairing QR code

## How It Works

1. Run `/zedra-start` to launch the Zedra Host daemon
2. The daemon prints an ASCII QR code in the terminal
3. Scan the QR with the Zedra mobile app to pair
4. Your phone gets a remote terminal, file explorer, and git tools for this workspace
5. Use `/zedra-terminal` to open additional terminals on the phone
6. Use `/zedra-stop` to shut down when done
