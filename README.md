# Zedra

An experimental remote editor on mobile with GPU rendering powered by Zed's GPUI, P2P tunnel over QUIC/UDP by Iroh. Zedra focusses on providing mobile-first experience for developers to read code, view changes, run any AI agents on their terminals over a secure, P2P tunnel.

![Zedra](https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/OG.png)

## Get the App

- iOS — [TestFlight](https://testflight.apple.com/join/1EWe2kRH)
- Android — coming soon

## Desktop daemon

Note: Consider using Tailscale to always have direct connection between your computers. P2P connections are unreliable on home networks and may require relay service, which isn't optimized for low latency demand.

**Manual**
```bash
# Install Zedra CLI
curl -fsSL zedra.dev/install.sh | sh
# Start Zedra in working directory
zedra start
```

**Claude Code**
```bash
# Inside Claude Code session
/plugin marketplace add tanlethanh/zedra-plugin
/plugin install zedra@zedra
# Restart Claude Code session and start Zedra
/zedra:zedra-start
```

Scan the QR code with the Zedra app. That's it.

## How It Works

1. `zedra start` runs a lightweight daemon on your desktop
2. Phone and desktop discover each other automatically — direct P2P, relay fallback
3. All traffic is encrypted end-to-end with TLS 1.3. Pairing requires physical QR scan — no credentials leave your device


## Status

Zedra is under active development. Core features are stable and in use — bugs, rough edges, and breaking changes should be expected. Feedback and issues are welcome on [GitHub](https://github.com/tanlethanh/zedra/issues).

## License

MIT © [Tan Le](https://github.com/tanlethanh)
