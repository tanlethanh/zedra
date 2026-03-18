# Zedra

**Your desktop, in your pocket.**

Run Claude Code from your phone. One QR scan connects you to your desktop — full terminal, file browser, git, and AI coding agents, all over an encrypted P2P tunnel. No port forwarding. No cloud. Just your machine.

<!-- TODO: demo GIF or screenshot -->

## Quick Start

```bash
# On your desktop
curl -fsSL https://zedra.dev/install.sh | sh
zedra start
```

Scan the QR code with the Zedra app. That's it.

> Using Claude Code? Install the plugin instead:
>
> ```
> /plugin marketplace add tanlethanh/zedra
> /plugin install claude-code@zedra
> /zedra-start
> ```

## What You Get

|                 |                                                            |
| --------------- | ---------------------------------------------------------- |
| **AI Agents**   | Claude Code, Codex, Open Code — run and interact on mobile |
| **Terminal**    | Full shell on your desktop, from your phone                |
| **Files**       | Browse, open, and edit with syntax highlighting            |
| **Git**         | Status, diff, log, commit — all from the palm of your hand |
| **60 FPS**      | GPU-accelerated native rendering, not a web view           |
| **Zero Config** | QR pairing, auto LAN/relay discovery, no setup             |

## How It Works

```
Phone ←──── encrypted QUIC tunnel ────→ Desktop
       (LAN direct or relay fallback)
```

1. `zedra start` runs a lightweight daemon on your desktop
2. Phone and desktop discover each other automatically — direct on LAN, relay when remote
3. All traffic is encrypted end-to-end with TLS 1.3. Pairing requires physical QR scan — no credentials leave your device

## Get the App

- [Android](https://play.google.com/store/apps/details?id=dev.zedra.app)
- iOS — coming soon

## Status

Zedra is in active development. I need your feedbacks.

## License

MIT
