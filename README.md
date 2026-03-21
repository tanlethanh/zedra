# Zedra

**Remote editor on mobile. Code from anywhere.**

One QR scan connects you to your desktop. Full terminal, file browser, git, and AI agents over an encrypted P2P tunnel. No port forwarding. No cloud.

![Zedra](https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/OG.png)

<video src="https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/zedra-demo.mp4" autoplay muted loop playsinline width="100%"></video>

## Quick Start

**curl**
```bash
curl -fsSL zedra.dev/install.sh | sh
zedra start
```

**Claude Code**
```
/plugin marketplace add tanlethanh/zedra
/plugin install claude-code@zedra
/zedra-start
```

**Codex**
```bash
curl -fsSL zedra.dev/codex.sh | sh
# then in Codex:
/zedra-start
```

**OpenCode**
```bash
curl -fsSL zedra.dev/opencode.sh | sh
# then in OpenCode:
/zedra-start
```

**Gemini CLI**
```bash
gemini skills install https://github.com/tanlethanh/zedra.git --path plugins/zedra
/zedra-start
```

Scan the QR code with the Zedra app. That's it.

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

- Android — coming soon
- iOS — [TestFlight](https://testflight.apple.com/join/1EWe2kRH)

## Status

Zedra is in active development. I need your feedbacks.

## License

MIT
