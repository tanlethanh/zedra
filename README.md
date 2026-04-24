# Zedra

An experimental remote editor on mobile with GPU rendering powered by Zed's GPUI, P2P tunnel over QUIC/UDP by Iroh. Zedra focusses on providing mobile-first experience for developers to read code, view changes, run any AI agents on their terminals over a secure, P2P tunnel.

![Zedra](https://raw.githubusercontent.com/tanlethanh/zedra/main/packages/landing/public/OG.png)

## Download App

- iOS — [TestFlight](https://testflight.apple.com/join/1EWe2kRH)
- Android — coming soon

## Desktop daemon

Note: Zedra attempts to establish a direct connection between your computers, but sometimes it may be blocked by network conditions, specifically `Symmetric NAT` or `CGNAT` (commonly seen in home networks). In such cases, the connection still needs a relay fallback path. For now, it works best on LANs and the regions supported by relay servers. If you encounter noticeable high latency, please reach out to me. For those curious about this topic, I recommend reading [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)

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
