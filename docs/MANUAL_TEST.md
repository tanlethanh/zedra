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
5. Open the workspace drawer immediately after connect
6. Expected: file explorer root entries and git status are already loaded without waiting for the first drawer open to trigger them
7. Navigate to terminal — verify PTY works (shell prompt, keystrokes echo)

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
5. Expected: the workspace drawer Terminals tab shows the active remote
   terminals from the host without creating a replacement terminal

## 4. Reconnect After Host Restart

1. Connect via QR, note session ID
2. Kill the host daemon (Ctrl-C or `zedra stop`)
3. Wait 5 seconds, restart host: `zedra start --workdir .`
4. Expected: app auto-reconnects (Reconnecting badge → Connected); session
   panel shows same or new session ID depending on `sessions.json` state
5. Expected: after reconnect, file explorer root entries and git status refresh asynchronously without blocking the terminal from becoming usable

## 5. Host Unreachable → Retry

1. Connect via QR
2. Take the host machine offline (disable network interface)
3. Expected: badge shows "Reconnecting... (N)" counting up to 10
4. After 10 attempts: badge shows "Disconnected" / home screen shows "Unreachable"
5. Bring host network back up, tap "Retry"
6. Expected: reconnects successfully

## 5a. Idle Before Reconnect

1. Connect via QR and wait for the session badge to show "Connected"
2. Disable the host network interface or disconnect the client from the network without closing the app
3. Expected within about 4 seconds: session badge changes to "Idle Ns" while the session is still present
4. Keep waiting
5. Expected later: normal reconnect flow still takes over (`Idle` -> `Reconnecting` -> `Disconnected` or `Connected`, depending on recovery)
6. Restore network before reconnect exhausts
7. Expected: badge returns from `Idle` to `Connected` before or during reconnect recovery

## 5b. Path Changes Do Not False Idle

1. Connect and wait for the transport badge to settle on `Relay` or `P2P`
2. Trigger a path change without fully losing connectivity
3. Example: force a relay-to-direct upgrade, or briefly switch networks and recover before reconnect starts
4. Expected: the badge may change transport type or RTT, but it should not enter `Idle` unless connection-wide inbound traffic stalls for about 4 seconds
5. Expected: path handoff can update the displayed path metadata without changing the liveness rule

## 5c. Host macOS Sleep/Wake Direct Path Recovery

1. Start the host daemon on macOS with tracing enabled and connect from a device
2. Wait for the transport badge to show `P2P`, or confirm the daemon/client logs show a selected direct path
3. Sleep the host Mac long enough for the client connection to go idle or reconnect
4. Wake the host without restarting `zedra start`
5. Expected in host logs: `net_monitor: starting endpoint recovery` followed by fresh iroh network-change/report activity
6. Expected: the client may reconnect over relay first, then upgrades back to `P2P` without restarting the host daemon

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

## 9. Terminal Hyperlink Detection

1. Connect to a session and open the terminal view
2. Run:

```bash
printf 'src/main.rs:12:3\ngit:(refactor-app-session-architecture)\nhello\nREADME\nv0.112.0\ngpt-5.4\n/model\n'
```

3. Expected before tapping: `src/main.rs:12:3` shows a subtle underline; `git:(refactor-app-session-architecture)`, `hello`, `README`, `v0.112.0`, `gpt-5.4`, and `/model` do not
4. Tap `src/main.rs:12:3`
5. Expected: the terminal file preview opens for `src/main.rs` at line/column metadata
6. Expected: the preview header metadata and code body both render with the app monospace font rather than a proportional fallback
7. Tap `git:(refactor-app-session-architecture)`, `hello`, `README`, `v0.112.0`, `gpt-5.4`, and `/model`
8. Expected: none of those tokens are treated as hyperlinks and no preview sheet opens

## 10. Connecting Overlay Layout On Wide Screens

1. Open the app on a wide device or simulator width such as iPad or landscape iPhone
2. Start a new connection or reopen a saved workspace so the connecting overlay is visible
3. Expected: the connecting content stays horizontally centered with visible left and right padding
4. Rotate or resize while the overlay is visible
5. Expected: the title, badge, and details remain in a bounded centered column instead of stretching edge to edge

## 11. Terminal Keyboard Tap Toggle On iOS

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Tap a non-hyperlink area of the terminal once
3. Expected: the terminal becomes focused, the software keyboard appears, and terminal input works
4. Expected: the visible terminal surface does not show a native full-height UIKit caret
5. Expected: long-press on terminal content does not show native iOS text-selection handles or selection highlight
6. Tap a non-hyperlink area of the already-focused terminal again
7. Expected: the software keyboard dismisses, terminal focus is cleared, and the keyboard does not immediately reopen
8. Tap the terminal a third time
9. Expected: the terminal becomes focused again and the software keyboard reopens on that first tap after dismissal

## 11a. Terminal Scroll To Bottom Native Button On iOS

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Print enough output to create scrollback, for example `seq 1 200`
3. Drag upward in the terminal until the view is several lines away from the bottom
4. Expected: a small native circular arrow-down button materializes at the lower-right of the terminal
5. Expected on iOS 26 or newer: the button uses UIKit glass; on older iOS it falls back to a native dark material
6. Tap the arrow-down button
7. Expected: the terminal scrolls to the latest output immediately, the native press feedback is visible briefly, then the button dematerializes
8. Show the software keyboard, scroll away from the bottom again, and repeat
9. Expected: the button stays above the keyboard/home indicator and still scrolls to the bottom
10. Flick terminal scrollback so it continues moving with momentum, then tap the arrow-down button while momentum is still active
11. Expected: the terminal stays pinned to the latest output instead of drifting back upward from the remaining momentum
12. While the button is visible, switch to another terminal with enough scrollback and drag away from the bottom
13. Expected: the previous terminal's button is gone, and the newly active terminal shows its own scroll-to-bottom button

## 12. Quick Action Terminal Navigation

1. Connect to a session with at least two open terminals
2. Return to the home screen
3. Open the quick action panel and tap a terminal card under the connected workspace
4. Expected: the quick action panel closes, the app switches to the workspace screen, and the tapped terminal becomes the main view
5. Repeat from the workspace screen with a different terminal card
6. Expected: the selected terminal becomes active immediately without getting stuck on the previous screen or terminal

## 13. Drawer Close Tap During Snap

1. Open either app drawer
2. With the soft keyboard visible, start dragging the drawer or trigger an open or close snap
3. Expected: the soft keyboard hides as soon as dragging or snapping begins
4. Trigger a close and immediately tap the dimmed backdrop while the drawer is still animating closed
5. Repeat, but tap the closing drawer panel instead of the backdrop
6. Start opening the drawer with a drag, release so it continues snapping open, then tap or drag again before the snap completes
7. Compare the release-to-open snap against the release-to-close snap
8. Drag the drawer closed and start a new open drag as soon as it looks fully closed
9. Release a drag when the drawer is only slightly away from the open or closed target
10. Expected: input is ignored until the current snap animation finishes; the drawer does not reverse, restart, or jump to a new position, and close unlocks immediately when the visual close ends without an extra dead interval

## 14. Git Panel Diff Navigation

1. Connect to a workspace with at least one modified file and one untracked file
2. Open the workspace drawer and switch to the Git Diff tab
3. Tap a file entry in the git panel
4. Expected: the drawer closes and the git diff view opens for the tapped file
5. Expected: added and removed lines are indicated by full-width background color only; the diff text does not render a leading `+` or `-`
6. Expected: added and removed backgrounds stay continuous across rows without thin gaps, including after horizontal scrolling long lines
7. Expected: the workspace header subtitle shows the git filename plus added and removed totals, and long filenames truncate instead of overflowing
8. Tap the untracked file entry
9. Expected: the diff view shows the untracked file content as added lines
10. Long-press a file entry
11. Expected: the file action sheet opens for that entry instead of doing nothing

## 15. Markdown List Item Wrap In Preview

1. Connect to a session and open the terminal view
2. Run:

```bash
printf '%s\n' '- This is a very long bullet item that should wrap across multiple lines in the markdown preview without clipping or forcing horizontal overflow on a phone-sized sheet.' '1. This is a very long numbered item that should also wrap within the list text column while keeping the marker visible at the left edge.' > /tmp/zedra-markdown-wrap.md
printf '/tmp/zedra-markdown-wrap.md:1\n'
```

3. Tap `/tmp/zedra-markdown-wrap.md:1`
4. Expected: the preview opens in markdown mode
5. Expected: both the bullet item and the numbered item wrap onto multiple lines inside the text column; the marker stays visible and the wrapped lines do not overflow horizontally

## 16. iOS Native Selection In Markdown Preview

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

```bash
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
```

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

## 17. Native Confirmations For Terminal Delete And Session Disconnect

1. Connect to a session with at least two terminals open
2. Open the workspace drawer Terminals tab and tap the close affordance on one terminal card
3. Expected: a native confirmation alert appears with `Delete` and `Cancel`
4. Tap `Cancel`
5. Expected: the alert dismisses and the terminal card remains visible
6. Trigger the same delete again and tap `Delete`
7. Expected: the terminal card is removed from the drawer immediately, without waiting for the remote delete RPC to finish
8. Repeat terminal deletion from the quick action panel
9. Expected: the same native confirmation alert appears there, and confirmed deletion removes the card immediately
10. Open the Session tab and tap `Disconnect`
11. Expected: a native confirmation alert appears with `Disconnect` and `Cancel`
12. Tap `Cancel`, then retry and tap `Disconnect`
13. Expected: the session disconnects only after confirmation
## 18. Workspace Header Terminal Title + Agent Icon

Requires shell integration emitting OSC 133/633/1337 (zsh + iTerm2 integration,
or VS Code shell integration).

1. Open a workspace, open a terminal.
2. Type `claude` (or `opencode`, `codex`, `gemini`) — wait for prompt.

Expected:
- Header subtitle (below project name) updates from cwd to the terminal title.
- Agent icon appears to the left of the subtitle matching the running CLI
  (claude/opencode/openai/gemini/copilot).
- After the agent exits and shell returns to prompt, icon disappears;
  subtitle shows the last title (or falls back to cwd).
- Switching to a different terminal in the drawer updates header to the
  active terminal's title + icon.
