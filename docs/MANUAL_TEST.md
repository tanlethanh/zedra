# Manual Test Plan

## Agent Notes

- For UI, platform, and device-driven changes, agents should add or update the relevant manual verification steps in this document.
- Prefer concrete reproduction steps and expected results over vague test descriptions.
- When debugging, add targeted log instructions if the test depends on developer-run device validation.

## 1. Normal QR Scan → Connect

1. Start host daemon: `zedra start --workdir .`
2. Open app on device
3. Tap "Scan QR" — scan the terminal QR code
4. Expected: app connects, session panel shows "Connected", endpoint shown
5. Navigate to terminal — verify PTY works (shell prompt, keystrokes echo)

## 2. QR Already Consumed

1. Start host: `zedra start --workdir .`
2. Device A scans QR → connects successfully
3. Device B scans the **same** QR
4. Expected: Device B sees "Handshake already used" error (not a crash)
5. To pair Device B: restart host (or run `zedra qr` if/when implemented)

## 3. Continue Session from Saved Workspace

1. Connect via QR (test 1 above), navigate around, note session ID in panel
2. Force-close the app
3. Reopen — tap the saved workspace entry in the home screen
4. Expected: reconnects using stored session ID (no QR needed); terminal
   backlog replays any missed output

## 4. Reconnect After Host Restart

1. Connect via QR, note session ID
2. Kill the host daemon (Ctrl-C or `zedra stop`)
3. Wait 5 seconds, restart host: `zedra start --workdir .`
4. Expected: app auto-reconnects (Reconnecting badge → Connected); session
   panel shows same or new session ID depending on `sessions.json` state

## 5. Host Unreachable → Retry

1. Connect via QR
2. Take the host machine offline (disable network interface)
3. Expected: badge shows "Reconnecting... (N)" counting up to 10
4. After 10 attempts: badge shows "Disconnected" / home screen shows "Unreachable"
5. Bring host network back up, tap "Retry"
6. Expected: reconnects successfully

## 6. Session Occupied (Two Devices)

1. Pair Device A via QR → connected to session S
2. Start a new `zedra start` for the same workdir on the host (same session)
3. Pair Device B via the new QR → should attach to session S
4. Expected: Device B blocked with "Session occupied" (Device A is active)
5. Disconnect Device A → Device B can now attach

## 7. `zedra client` RTT Test

```bash
# Terminal 1
zedra start --workdir .

# Terminal 2 (same machine, same workdir)
zedra client --workdir . --count 5
```

Expected output: 5 ping rows with RLY/P2P label and RTT in ms, then statistics.

## 8. `--relay-url` Override

```bash
zedra start --workdir . --relay-url https://sg1.relay.zedra.dev
```

Expected: host connects to the specified relay; QR shows that relay URL in
`relay` field of JSON output (`--json` flag).

## 16. iOS Native Selection In Markdown Preview

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

````bash
cat >/tmp/zedra-markdown-selection.md <<'EOF'
# Selection Test

This paragraph should support native iOS selection inside the markdown preview.

- First bullet item
- Second bullet item

```rust
let answer = 42;
println!("{answer}");
```
EOF
printf '/tmp/zedra-markdown-selection.md:1\n'
````

3. Tap `/tmp/zedra-markdown-selection.md:1`
4. Expected: the preview opens in markdown mode
5. Long-press inside the heading or paragraph text
6. Expected: native iOS selection handles and the system edit menu appear without any custom app long-press UI
7. Drag a selection handle downward across the bullet list and into the code block
8. Expected: the selection can extend across markdown blocks; visible list markers and code lines participate in the selected range instead of acting like dead zones
9. With the selection still active, scroll the markdown preview vertically
10. Expected: the native selection highlight and handles move with the selected text instead of staying fixed to the viewport
11. Tap `Copy`
12. Expected: the selection menu dismisses cleanly and the preview remains responsive to scrolling and link taps afterward
