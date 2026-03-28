---
layout: ../layouts/ProseLayout.astro
title: "Privacy — Zedra"
description: "Zedra never sees your code, terminal sessions, or files. All data travels over an encrypted P2P tunnel between your devices."
canonical: "zedra.dev/privacy"
---

# Privacy Policy

<span class="updated">Last updated: March 2026</span>

> **Zedra never sees your code, terminal sessions, file contents, or commands.**
> All data between the mobile app and your machine travels over a direct, end-to-end
> encrypted P2P tunnel that bypasses any Zedra infrastructure. We cannot intercept it,
> and we do not try to.

## How Zedra works

Zedra connects your phone directly to your desktop over an encrypted **iroh QUIC/TLS 1.3**
tunnel. Your terminal output, keystrokes, file contents, and git diffs are transmitted only
between your two devices.

**Direct connection (preferred).** iroh performs NAT traversal using STUN-style hole-punching.
When successful, traffic flows directly device-to-device with no intermediary.

**Relay fallback.** When a direct path cannot be established — for example, behind a
symmetric NAT or a restrictive firewall — iroh transparently falls back to routing traffic
through an iroh relay server. The relay only forwards encrypted QUIC packets; it has no
ability to decrypt or inspect the content. Relay servers are a transport mechanism, not a
data store.

**End-to-end encryption.** All data transmitted between iroh endpoints is protected with
end-to-end encryption. Data is encrypted on the sender's device and can only be decrypted
by the intended recipient. This applies to both the direct and relay paths — even relay
servers that facilitate the connection cannot read the data being transmitted. iroh uses
**Ed25519 keys** for endpoint identity and encryption by default. No Zedra server ever sees
the plaintext of your session.

## What we do not collect

- **Code and files.** File contents, directory listings, and editor state never leave your machines.
- **Terminal sessions.** All PTY input and output is end-to-end encrypted between your devices. Zedra servers see none of it.
- **Git history and diffs.** Your repository data stays on your machine.
- **Authentication credentials.** The pairing key is stored locally on your device. We never receive it.
- **IP addresses or location data.** We do not log connection metadata.
- **Personal information.** Zedra requires no account, email address, or identity.

## Anonymous usage telemetry

We collect a small amount of anonymous, aggregated telemetry to understand how the product
is used. This data contains **no personal information and no content from your sessions.**

**Landing page (zedra.dev).** We use Firebase Analytics to track anonymous page interactions:
which install method is selected, whether the App Store button is clicked, and which social
links are followed. No cookies identifying you personally are set. No browsing history
outside this site is collected.

**Desktop daemon (`zedra start`).** The daemon reports anonymous usage events — daemon start,
session open, session end, and aggregate bandwidth samples — to Google Analytics via the
Measurement Protocol. Events are tagged with a randomly generated machine UUID stored at
`~/.config/zedra/telemetry_id`. This ID is not linked to your identity. It contains no
hostname, username, file paths, or terminal content.

If the daemon binary is built without the telemetry credentials embedded, all telemetry is
silently disabled.

## Third-party services

- **Firebase / Google Analytics.** Used for anonymous landing page and daemon usage events. Subject to [Google's Privacy Policy](https://policies.google.com/privacy).
- **Apple TestFlight.** Distribution of the iOS app. Subject to [Apple's Privacy Policy](https://www.apple.com/legal/privacy/).
- **iroh relay servers.** Used as a fallback when a direct P2P connection cannot be established. Zedra self-hosts the upstream [`iroh-relay`](https://github.com/n0-computer/iroh) binary — unmodified open source — with no database and no logging of connection content. The deployment configuration is available in [`deploy/relay/`](https://github.com/tanlethanh/zedra/tree/main/deploy/relay) in the public repository. Relay servers forward encrypted QUIC packets only and cannot decrypt them. A direct P2P connection is always attempted first; relay is only used when NAT traversal fails.

## Data retention

Anonymous analytics events are retained by Google Analytics under Google's standard data
retention policy (default 14 months). Zedra does not maintain its own database of user
records.

## Changes to this policy

If we materially change what data is collected, we will update this page and note the date
at the top. We will not begin collecting new categories of personal data without updating
this policy first.

## License

Zedra is open source software released under the [MIT License](https://github.com/tanlethanh/zedra/blob/main/LICENSE).
Copyright &copy; 2026 Tan Le.

## Contact

Questions or concerns? Open an issue on [GitHub](https://github.com/tanlethanh/zedra).
