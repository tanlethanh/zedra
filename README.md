# Zedra

An experimental remote code editor on mobile with GPU-accelerated rendering powered by Zed's GPUI, P2P tunnel over QUIC/UDP by Iroh. Zedra focuses on providing mobile-first experience for developers to read code, view changes, and run AI agents over a secure, P2P tunnel.

![Zedra](https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/OG.png)

## Security Note

Zedra is early, but it provides the most secure approach with e2e encryption, direct connection, and zero-trust model in a near future. If you have any concern about security, email me at [tanle@zedra.dev](mailto:tanle@zedra.dev), I will support to bring Zedra to your workflow with trust.

## Download App

- iOS — [AppStore](https://apps.apple.com/vn/app/zedra-code-from-anywhere/id6760534630) or [TestFlight](https://testflight.apple.com/join/1EWe2kRH)
- Android — Closed testing. Join via [Google Groups](https://groups.google.com/g/zedra-beta)
 
## Desktop daemon

Note: Zedra uses direct P2P connections when possible, but may fallback to relays if blocked by `Symmetric NAT` or `CGNAT` (common in home networks). Works best on LANs and supported relay regions. For high latency issues, please reach out. Learn more: [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)

**MacOS/Linux**
```shell
curl -fsSL zedra.dev/install.sh | sh
# Install agent hooks for notification
zedra setup
# Start daemon in working directory
zedra start --detach
```

**Windows**

```powershell
irm https://zedra.dev/install.ps1 | iex
zedra start --detach
```

**Claude Code**
```shell
# Config Zedra skills, hooks for Claude
zedra setup claude
# In Claude Code, reload plugins and start Zedra
/zedra-start
```

**Codex**

```shell
# Config Zedra skills, hooks for Codex
zedra setup codex
# In Codex, reload plugins and start Zedra
$zedra-start
```

Scan the QR code with the Zedra app. That's it.

## How It Works

1. `zedra start` runs a lightweight daemon on your desktop
2. Phone and desktop discover each other automatically — direct P2P, relay fallback
3. All traffic is encrypted end-to-end with TLS 1.3. No credentials leave your device


## Status

Zedra is under active development. Core features are stable and in use — bugs, rough edges, and breaking changes should be expected. Feedback and issues are welcome on [GitHub](https://github.com/tanlethanh/zedra/issues).

## License

MIT © [Tan Le](https://github.com/tanlethanh)
