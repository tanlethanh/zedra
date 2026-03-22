---
layout: ../layouts/ProseLayout.astro
title: "Support — Zedra"
description: "Get help with Zedra."
canonical: "zedra.dev/support"
---

# Support

## Contact

Email us at **tanle@zedra.dev** and we will get back to you as soon as possible.

You can also open an issue on [GitHub](https://github.com/tanlethanh/zedra/issues).

## Frequently asked questions

**The app cannot connect to my desktop.**

Make sure the host daemon is running on your desktop by executing `zedra start` in your terminal.
Then tap the QR scan button in the app and scan the code shown in your terminal.
If the connection still fails, check that your desktop firewall allows outbound UDP traffic.

**The terminal appears blank after reconnecting.**

Type any key (e.g. `Enter`) to refresh the prompt, or run `clear`.
This can happen when the session backlog is too large to replay.

**I lost my pairing after restarting the daemon.**

Run `zedra qr` on your desktop to generate a new pairing QR code and scan it from the app.

**The app crashes or behaves unexpectedly.**

Please email **tanle@zedra.dev** with a brief description of what happened and your device model.
Bug reports are always welcome on [GitHub](https://github.com/tanlethanh/zedra/issues) as well.

## Privacy

See our [Privacy Policy](/privacy) for details on what data Zedra collects (very little) and how it is used.
