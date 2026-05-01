# Zedra

An experimental remote code editor on mobile with GPU-accelerated rendering powered by Zed's GPUI, P2P tunnel over QUIC/UDP by Iroh. Zedra focuses on providing mobile-first experience for developers to read code, view changes, and run AI agents over a secure, P2P tunnel.

![Zedra](https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/OG.png)

## Download App

- iOS — [TestFlight](https://testflight.apple.com/join/1EWe2kRH)
- Android — coming soon

## Desktop daemon

Note: Zedra uses direct P2P connections when possible, but may fallback to relays if blocked by `Symmetric NAT` or `CGNAT` (common in home networks). Works best on LANs and supported relay regions. For high latency issues, please reach out. Learn more: [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)

**Manual**
```shell
# Install Zedra CLI
curl -fsSL zedra.dev/install.sh | sh
# Start Zedra in working directory
zedra start
# Or start in background 
zedra start --detach
```

**Claude Code**
```shell
# Config Zedra skills for Claude
zedra setup claude
# In Claude, reload plugins and start Zedra
/reload-plugins
/zedra-start
```

**Codex**

```shell
# Config Zedra skills for Codex
zedra setup codex
# In Codex, reload skills and start Zedra
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
