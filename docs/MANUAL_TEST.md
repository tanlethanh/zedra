# Manual Test Plan

## Agent Notes

- For UI, platform, and device-driven changes, agents should add or update the relevant manual verification steps in this document.
- Prefer concrete reproduction steps and expected results over vague test descriptions.
- When debugging, add targeted log instructions if the test depends on developer-run device validation.

## 0. Mobile Hover Styling

1. Open the app on iOS or Android
2. Tap an outline button, drawer tab icon, the Session direct-address row, and the Session Disconnect button
3. Expected: each tap still triggers its action
4. Expected: no hover background remains stuck after tap, drag, or scroll interactions
5. Expected: active, selected, destructive, and disabled states remain readable without hover styling

## 0a. Home Install Guide Tabs

1. Open the app with no saved workspaces visible on the Home screen
2. Switch between the `curl`, `claude`, `codex`, `opencode`, and `gemini` guide tabs
3. Expected: each tab shows the same install commands as the landing page
4. Tap a command line in each tab
5. Expected: the tapped command line is copied to the system clipboard without navigating away from Home
6. Long-press and drag across guide text
7. Expected: native text selection handles appear, command and comment lines are selectable, and `Copy` copies the selected text
8. Expected: switching tabs or scrolling the guide does not leave stale selection handles on screen

## 0b. Home Settings Button

1. Run a Debug build and open the Home screen
2. Tap the top-right settings icon
3. Expected: a light haptic feedback fires and the Settings screen opens
4. Run a Release build and open the Home screen
5. Expected: the settings icon is not visible and the developer Settings screen is not reachable from Home

## 0c. Developer Native Notification

1. Run a Debug build and open Settings
2. Tap `Native Notification` in the Developer section
3. Expected: two compact in-app notification bubbles slide down near the top safe area, with the newest expanded in front and the older one peeking above it as a smaller glass bubble
4. Expected: the front bubble uses the app asset icon `AgentCodex` tinted like the notification title; the upper peek bubble uses an SF Symbol fallback icon
5. Expected: each new bubble fades in faster than it scales, then continues sliding down and settling into place with a subtle spring
6. Tap the front `Agent completed` bubble
7. Expected: it fades out faster than it scales, then finishes scaling back toward zero while sliding upward; a callback notification appears
8. Tap `Native Notification` again, then swipe the front bubble downward
9. Expected: all pending notification items expand into full bubbles, with the oldest at the top and the newest at the bottom
10. Swipe any expanded bubble upward
11. Expected: only that bubble dismisses, and the remaining bubbles move up smoothly
12. Tap `Native Notification` repeatedly
13. Expected: multiple notifications collect into the same bubble stack and all auto-close after their configured durations by default

## 0d. Firebase GPUI Screen Views

1. Run an iOS build with Firebase Analytics enabled and open Firebase DebugView or a build with `debug-telemetry`
2. Open Home, Settings, Quick Actions, then connect to a workspace
3. Open the workspace drawer and switch through Files, Documents, Git Diff, Terminals, and Session
4. Open a non-markdown file, a markdown file, a git diff, and a terminal as the main workspace view
5. Tap terminal file links for both a source file and a markdown file so the native custom sheet opens
6. Expected: manual `screen_view` events include `screen_name` and `screen_class` for `Home`, `Settings`, `Quick Actions`, `Workspace Connecting`, `Workspace Editor`, `Workspace Markdown`, `Workspace Git Diff`, `Workspace Terminal`, each drawer tab, `Custom Sheet Editor`, and `Custom Sheet Markdown`
7. Expected: native automatic rows such as `UIViewController`, `CustomSheetViewController`, `UIAlertController`, and `ZedraQRScannerVC` are still present because native Firebase screen reporting remains enabled

## 1. Normal QR Scan → Connect

1. Start host daemon: `zedra start --workdir .`
2. Open app on device
3. Tap "Scan QR" — scan the terminal QR code
4. Expected: app connects, session panel shows "Connected", endpoint shown
5. Open the workspace drawer immediately after connect
6. Switch to the Session tab and confirm the `Connection` section has a right chevron indicator and wraps long status text, then tap it
7. Expected: the drawer closes and the connecting view opens for the active session with a top-right close button
8. Tap the top-right close button
9. Expected: the connecting view closes and the previous workspace content is visible
10. Expected: closing the connecting view does not fire haptic feedback
11. Reopen the connecting view, then open a file, git diff, or terminal from the workspace drawer
12. Expected: the connecting view closes and the selected workspace content is visible
13. Expected: file explorer root entries and git status are already loaded without waiting for the first drawer open to trigger them
14. Navigate to terminal — verify PTY works (shell prompt, keystrokes echo)

## 1a. Host Info Subscription

1. Start host daemon: `zedra start --workdir .`
2. Connect from the app and open the workspace drawer
3. Switch to the Session tab
4. Expected within 5 seconds: CPU, RAM, uptime, and battery rows appear when the host exposes battery data
5. Leave the Session tab open for at least 15 seconds while running a CPU or memory load on the host
6. Expected: CPU/RAM values update roughly every 5 seconds without reconnecting or refreshing the drawer
7. Disconnect the app
8. Expected: host logs show no repeated host-info send errors after the stream closes

## 1b. Large File Explorer Responsiveness

1. Start host daemon in a large repository: `zedra start --workdir /path/to/large/repo`
2. Connect from the app and open the workspace drawer
3. In the File Explorer tab, expand several directories and use "Load more" until the visible tree contains hundreds of rows
4. Scroll the file explorer and repeatedly expand/collapse directories with already-loaded children
5. Expected: scrolling and toggles stay responsive, without long UI stalls or accidental file opens from loading rows
6. Tap a file row nested at least four levels deep
7. Expected: the drawer starts closing immediately without stuttering while the file loads, and file explorer rows use the same height as Git panel file rows
8. Reopen the drawer while the file remains the main workspace view
9. Expected: only the file row for the active main workspace view is highlighted, and the highlight spans the full file explorer width
10. Open a git diff or terminal as the main workspace view, then reopen the file explorer
11. Expected: the file row highlight clears because the active main workspace view is no longer that file
12. Expected: before syntax highlighting appears, code text uses a subtly dim foreground; when highlighting applies, text rows do not jump, reorder, or visibly reload

## 1c. Docs Tree Display Mode

1. Start host daemon in a repository with markdown files under both root and nested paths, including a `.git` directory
2. Connect from the app and open the workspace drawer
3. In the File Explorer tab, tap the top-right docs-tree display mode toggle
4. Expected: the docs tree builds from the host and shows markdown files with compact nested paths like `vendor/zed/docs/`
5. Collapse a nested docs directory, leave and reopen the workspace, then return to docs-tree mode
6. Expected: the same directory remains collapsed until manually expanded
7. Tap a markdown row
8. Expected: the drawer closes and the main workspace renders the selected markdown file
9. Reopen the drawer and return to docs-tree mode
10. Expected: only the active markdown file row is highlighted
11. Open a git diff or terminal as the main workspace view, then return to docs-tree mode
12. Expected: the docs-tree file highlight clears because the active main workspace view is no longer that markdown file
13. If `Load more docs` appears, tap it
14. Expected: another page is merged into the same tree without duplicating existing markdown rows
15. Scroll and collapse directories in a large docs tree
16. Expected: scrolling and toggles stay responsive without rendering the full tree at once
17. Tap the refresh icon in the docs tree footer
18. Expected: a native alert says Zedra will scan Markdown files and large workspaces may slow briefly
19. Tap `Refresh`
20. Expected: the refresh icon rotates while the current tree stays visible, then the tree is replaced after the refresh finishes
21. Expected: files inside dot-prefixed, gitignored, and common generated/dependency directories are not shown
22. Connect to an older host that does not support docs-tree RPCs
23. Expected: the docs tree shows an unsupported-host message and the refresh icon no longer stays in the building state

## 2. QR Already Consumed

1. Start host: `zedra start --workdir .`
2. Device A scans QR → connects successfully
3. Device B scans the **same** QR
4. Expected: Device B sees "The QR code was used. Refresh it and scan again." (not a crash)
5. To pair Device B: refresh the QR code and scan again

## 2a. Protocol Version Mismatch

1. Run an app build and CLI/host build that use different `ZEDRA_ALPN` versions
2. Scan the host QR from the app
3. Expected: connect view shows "Protocol mismatch, Update App or CLI"

## 3. Continue Session from Saved Workspace

1. Connect via QR (test 1 above), create at least three terminals, and note their order in the drawer Terminals tab
2. Force-close the app
3. Reopen — tap the saved workspace entry in the home screen
4. Expected: reconnects using stored session ID (no QR needed); terminal
   backlog replays any missed output
5. Expected: the workspace drawer Terminals tab shows the active remote
   terminals from the host without creating a replacement terminal
6. Expected: terminal cards appear in the same order they had before force-close

## 3a. Remove Saved Workspace From Home

1. Connect via QR so a workspace card appears on Home
2. Return to Home and long-press the workspace card
3. Tap `Delete` in the native confirmation alert
4. Expected: the workspace card disappears from Home immediately
5. Force-close and relaunch the app
6. Expected: the deleted workspace card does not reappear

## 3b. Terminal Reattach Resize

1. Connect on Device A, open a terminal, and run:
   ```sh
   sh -c 'show(){ set -- $(stty size); printf "WINCH %sx%s\n" "$2" "$1"; }; trap show WINCH; show; while sleep 1; do :; done'
   ```
2. Disconnect or force-close Device A while the process keeps running
3. Reattach from Device B with a different screen ratio, or relaunch after changing simulator/device orientation
4. Expected: the terminal reattaches and prints a new `WINCH <cols>x<rows>` matching the current device viewport
5. Repeat with a non-alt AI CLI session such as `claude`, `codex`, or a `/zedra-start` resumed session
6. Expected: resumed output uses the current device width without stale wrapping or dumped resize artifacts

## 4. Reconnect After Host Restart

1. Connect via QR, note session ID
2. Kill the host daemon (Ctrl-C or `zedra stop`)
3. Wait 5 seconds, restart host: `zedra start --workdir .`
4. Expected: app auto-reconnects (Reconnecting badge → Connected); session
   panel shows same or new session ID depending on `sessions.json` state
5. Expected: after reconnect, file explorer root entries and git status refresh asynchronously without blocking the terminal from becoming usable
6. Expected: if the restarted host syncs zero terminals, the app creates and opens a fresh terminal instead of leaving the main view on `Loading ...`

## 5. Host Unreachable → Retry

1. Connect via QR and create at least three terminals
2. Take the host machine offline (disable network interface)
3. Expected: badge shows `Reconnecting (N) · Ns` with a per-second countdown, up to 3 attempts
4. After 3 attempts: connect view shows "Host unreachable. Check network and host."
5. Bring host network back up, tap "Retry"
6. Expected: reconnects successfully
7. Expected: the workspace drawer Terminals tab preserves the pre-reconnect terminal order
8. If the host was restarted and no remote terminals remain, expected: a fresh terminal is created and opened

## 5a. Idle Before Reconnect

1. Connect via QR and wait for the session badge to show "Connected"
2. Disable the host network interface or disconnect the client from the network without closing the app
3. Expected within about 4 seconds: session badge changes to "Idle Ns" while the session is still present; workspace status dots turn yellow and blink
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
4. Expected while Device A is still live: Device B is blocked with "Host occupied. Disconnect other device and retry."
5. Quit or network-disconnect Device A, then retry Device B after the active-client stale window
6. Expected: the host evicts the stale Device A connection and Device B attaches without waiting for the full transport timeout

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
printf '\033]8;;file://%s/src/main.rs:12:3\033\\src/main.rs:12:3\033]8;;\033\\\n' "$PWD"
printf '\033]8;;https://zedra.dev\033\\zedra.dev\033]8;;\033\\\n'
```

3. Expected before tapping: only the OSC 8 `src/main.rs:12:3` and `zedra.dev` rows show a subtle underline; the plain `src/main.rs:12:3`, `git:(refactor-app-session-architecture)`, `hello`, `README`, `v0.112.0`, `gpt-5.4`, and `/model` rows do not
4. Tap the underlined OSC 8 `src/main.rs:12:3`
5. Expected: the terminal file preview opens for `src/main.rs` at line/column metadata
6. Expected: the preview header metadata and code body both render with the app monospace font rather than a proportional fallback
7. Tap the underlined OSC 8 `zedra.dev`
8. Expected: the URL opens externally
9. Tap the plain `src/main.rs:12:3`, `git:(refactor-app-session-architecture)`, `hello`, `README`, `v0.112.0`, `gpt-5.4`, and `/model`
10. Expected: none of those tokens are treated as hyperlinks and no preview sheet opens

## 9b. Terminal Preview Sheet Gesture Ownership

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

```bash
cat > /tmp/zedra-long-code.rs <<'EOF'
fn main() {
    let message = "this line is intentionally very long so the terminal preview code editor needs horizontal scrolling inside the native custom sheet without moving the sheet detent or dismissing the sheet while the drag is horizontal";
    println!("{message}");
}
EOF
printf '\033]8;;file:///tmp/zedra-long-code.rs:1:1\033\\/tmp/zedra-long-code.rs:1\033]8;;\033\\\n'
```

3. Tap `/tmp/zedra-long-code.rs:1`
4. Expected: the preview opens in code editor mode inside the native custom sheet
5. Expected: Rust keywords and string tokens gain syntax colors after the preview finishes parsing
6. Swipe horizontally across the long string line
7. Expected: the code scrolls sideways and the native sheet does not move or dismiss
8. Swipe mostly vertically inside the preview body
9. Expected: the preview content scrolls vertically
10. Scroll to the top of the preview body, then drag downward
11. Expected: the native sheet moves or dismisses normally from the top edge

## 10. Connecting Overlay Layout On Wide Screens

1. Open the app on a wide device or simulator width such as iPad or landscape iPhone
2. Start a new connection or reopen a saved workspace so the connecting overlay is visible
3. Expected: the connecting content stays horizontally centered with visible left and right padding
4. Expected: a restart connection icon appears immediately next to the title
5. Tap the restart connection icon
6. Expected: the icon rotates once, light haptic feedback fires, the current connection attempt restarts, and the overlay remains visible
7. Rotate or resize while the overlay is visible
8. Expected: the title, restart icon, badge, and details remain in a bounded centered column instead of stretching edge to edge
9. Tap `View Details`, then `Hide Details`
10. Expected: the subtitle stays horizontally centered and does not jump when details expand or collapse

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
10. Dismiss the keyboard through a platform control or hardware-keyboard state while the terminal remains focused, then tap the terminal again
11. Expected: the terminal stays focused and the software keyboard reopens
12. With the keyboard visible, drag vertically in terminal content to scroll scrollback or a terminal app that handles touch scroll
13. Expected: the terminal scrolls without dismissing the keyboard or clearing terminal focus
14. In a fresh non-alt terminal with the keyboard visible, run a slow stream such as `for i in $(seq 1 20); do echo "line $i"; sleep .2; done`
15. Expected: early output continues from the top without being pushed upward into an empty lower gap; once the occupied rows reach the keyboard edge, the terminal lifts gradually and never more than the keyboard height
16. With retained scrollback, clear the terminal using `printf '\033[2J\033[Htop\n'`
17. Expected: the cleared content stays top-aligned instead of inheriting a full keyboard lift from old scrollback; TUI-authored blank layout rows still count as occupied space
18. With the keyboard still visible and enough scrollback to reach history top, drag upward until scrolling stops, then keep dragging slightly
19. Expected: the oldest scrollback rows can be revealed and are not clipped above the terminal viewport

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

## 11b. Terminal Native Dictation On iOS

1. Connect to a session on a physical iPhone and open the terminal view
2. Tap the terminal so the software keyboard appears
3. Tap the keyboard dictation microphone and dictate a short command fragment such as `echo hello`
4. Expected: a compact native glass/material preview appears above the keyboard and updates with the live dictated text
5. Stop dictation
6. Expected: the preview hides, and the dictated text stays inserted in the PTY once, without being removed, without a stuck marked-text underline, and without duplicate characters
7. Repeat with a longer phrase and stop immediately after the last words
8. Expected: the final committed PTY text includes the last words, with no `UIDictationController` hypothesis-cancel log
9. Repeat with a dictated phrase that includes a newline or return command
10. Expected: newline input is routed as terminal enter rather than leaving literal marked text behind
11. Start dictation again, then cancel or force recognition failure by stopping before speech is recognized
12. Expected: the preview hides, the terminal remains focused, no stale dictation text is committed, and normal keyboard typing still reaches the PTY

## 11c. Native Keyboard Suggestions On iOS

1. On iPhone or iOS simulator, disable the simulator hardware keyboard so the software keyboard is visible
2. Connect to a session and tap the terminal
3. Type a partial word such as `hell`
4. Expected: iOS native inline predictions or suggestion candidates appear when supported by the OS and keyboard settings
5. Accept a suggestion
6. Expected: the accepted text is inserted into the PTY once, without enabling a native caret, edit menu, or terminal text-selection handles
7. Resume or reconnect to an existing terminal with text already at the prompt, then press and long-press software-keyboard backspace
8. Expected: each repeated backspace is routed to the PTY and can continuously delete existing prompt text rather than stopping after the synthetic prediction context is empty
9. Dictate a short fragment, stop dictation so it commits, then press backspace
10. Expected: the dictated characters can be deleted from the PTY one character at a time
11. Type command-like text with lowercase letters, straight quotes, hyphens, or double spaces
12. Expected: autocapitalization, smart quotes, smart dashes, and smart insert/delete do not rewrite the command text
13. Open the workspace Git sidebar and focus the Commit message input
14. Type prose and accept an available native suggestion
15. Expected: the suggestion inserts into the commit message, while smart punctuation and autocapitalization remain disabled
16. Switch the software keyboard to Vietnamese Telex and type `lee`, `chaf`, `toois`, `vois`, `ddungs`, and `uw ` in the terminal
17. Expected: the PTY receives `lê`, `chà`, `tối`, `với`, `đúng`, and `ư `, without duplicate base consonants such as `llê` or `chhà`, without placing the tone on the final vowel as `tôí`, without duplicating a replayed composed cluster as `vơới`, without dropping preserved prefixes such as `đ`, and without dropping the standalone composed character before the space
18. Switch to a Japanese keyboard, type a short marked composition, and accept a candidate
19. Expected: the marked text commits once to the PTY, the candidate UI anchors near the terminal input area, and there is no repeated `variant selector cell index number could not be found` warning

## 11d. iOS Native Text Input Regression Matrix

1. Terminal, normal IME marked text: switch to Japanese, type a multi-character composition, move between candidates, then accept one candidate
2. Expected: preedit text is visible only as native marked text, the accepted candidate is inserted once, and cancelling composition restores the previously committed terminal input
3. Terminal, Vietnamese Telex: type `lee`, `chaf`, `toois`, `vois`, `ddungs`, and `uw ` without pausing between keys
4. Expected: output is `lê`, `chà`, `tối`, `với`, `đúng`, and `ư ` immediately when the keyboard commits each rewrite, with no duplicate replayed consonants, dropped prefixes, or composed clusters
5. Terminal, native suggestion: type `teh`, accept the keyboard suggestion `the`, then type `hel` and accept `hello`
6. Expected: replacements are sent as minimal PTY diffs, with no extra backspaces, no duplicate prefix, and no stuck native marked range
7. Terminal, dictation: dictate a phrase, stop immediately after the final word, then wait for any late final transcript update
8. Expected: the preview hides on recording stop, the late hypothesis stays in the dictation store, final text includes the last words, and there is no `UIDictationController` hypothesis-cancel log
9. Terminal, native dictation stream: dictate `hello, how's it going` and watch the first `insertText` word plus following `replaceRange` rewrites
10. Expected: the first word moves directly into dictation preview without streaming to the PTY first, and finalization commits once without a hypothesis-cancel log
11. Cross-flow: after a dictation commit, press backspace repeatedly, then type `hel` and accept `hello`
12. Expected: dictation cleanup does not delete already-committed terminal text, repeated backspace continues through the PTY, and the following suggestion starts from a fresh keyboard context
13. Cross-flow: start a Japanese marked composition, cancel it, then accept a native suggestion in the same terminal focus session
14. Expected: cancelled marked text does not poison the following suggestion replacement or leave stale candidate UI
15. Commit message input: type Japanese marked text, accept a candidate, then accept a native suggestion
16. Expected: normal input uses the same native IME protocol correctly, with marked text committed once and suggestions replacing only the requested range
17. Commit message input, dictation: tap the microphone, dictate a short phrase, then stop dictation
18. Expected: the dictated phrase remains in the input after UIKit commits, the final cleanup delete does not clear the field, and any late `insertDictationResult` does not duplicate the phrase

## 11e. Terminal Keyboard Accessory Arrow Repeat On iOS

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Tap each arrow button in the keyboard accessory once
3. Expected: each tap sends exactly one corresponding arrow keystroke
4. Press and hold each arrow button, then release it
5. Expected: the corresponding arrow input repeats continuously while held and stops immediately on release
6. Start holding an arrow button, then dismiss the keyboard or background the app
7. Expected: repeat stops and does not resume when the keyboard or app returns

## 12. Quick Action Terminal Navigation

1. Connect to a session with at least two open terminals
2. Return to the home screen
3. Open the quick action panel and tap the add icon in the connected workspace header
4. Expected: the quick action panel closes, the app switches to the workspace screen, and a new terminal becomes the main view
5. Return to the home screen
6. Open the quick action panel and tap a terminal card under the connected workspace
7. Expected: the quick action panel closes, the app switches to the workspace screen, and the tapped terminal becomes the main view
8. Repeat from the workspace screen with a different terminal card
9. Expected: the selected terminal becomes active immediately without getting stuck on the previous screen or terminal

## 12a. Drawer Terminal List Stability During Network Reports

1. Connect to a session with at least two open terminals
2. Open the workspace drawer and switch to the Terminals tab
3. Leave the drawer open until the client logs `net report changed`
4. Expected: terminal cards remain visible and stable; the list does not disappear and reappear
5. Switch to the Session tab while network/path details update, then switch back to Terminals
6. Expected: the same terminal cards are still visible and ordered consistently

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
10. In a long drawer tab, try a mostly vertical swipe inside the tab content, then a mostly horizontal drawer drag from the same scrollable area
11. Expected: vertical swipes scroll the tab content without dragging the drawer; once the drawer claims a horizontal drag, the tab content does not scroll under the gesture
12. Expected: input is ignored until the current snap animation finishes; the drawer does not reverse, restart, or jump to a new position, and close unlocks immediately when the visual close ends without an extra dead interval

## 14. Git Panel Diff Navigation

1. Connect to a workspace with at least one modified file and one untracked file
2. Open the workspace drawer and switch to the Git Diff tab
3. Tap a file entry in the git panel
4. Expected: the drawer closes and the git diff view opens for the tapped file
5. Reopen the drawer and return to the Git Diff tab
6. Expected: the file entry for the currently opened diff is highlighted
7. Open a normal file or terminal as the main workspace view, then return to the Git Diff tab
8. Expected: the git diff highlight clears because the active main workspace view is no longer a git diff
9. Expected: added and removed lines are indicated by full-width background color only; the diff text does not render a leading `+` or `-`
10. Expected: added and removed backgrounds stay continuous across rows without thin gaps, including after horizontal scrolling long lines
11. Expected: the workspace header subtitle shows the git filename plus added and removed totals, and long filenames truncate instead of overflowing
12. Tap the untracked file entry
13. Expected: the diff view shows the untracked file content as added lines
14. Long-press a file entry
15. Expected: the file action sheet opens for that entry instead of doing nothing
16. Tap the dimmed backdrop outside the action sheet
17. Expected: the native action sheet dismisses without staging or unstaging the file

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

## 15a. Markdown Code Block Overflow In Preview

1. Connect to a session and open the terminal view
2. Run:

````bash
cat >/tmp/zedra-markdown-codeblock.md <<'EOF'
# Code Block Test

```bash
printf '%s\n' 'this-is-a-very-long-command-that-should-stay-on-one-line-and-scroll-horizontally-instead-of-wrapping-or-clipping-on-mobile'
```
EOF
printf '/tmp/zedra-markdown-codeblock.md:1\n'
````

3. Tap `/tmp/zedra-markdown-codeblock.md:1`
4. Expected: the preview opens in markdown mode
5. Expected: the fenced code block does not render a visible `bash` language label
6. Expected: code block padding and line height are compact enough for a phone-sized sheet
7. Swipe vertically starting inside the code block
8. Expected: the markdown preview scrolls vertically and the code line does not drift horizontally
9. Swipe horizontally inside the code block
10. Expected: the long command scrolls horizontally without changing the surrounding vertical scroll position

## 15b. Markdown Table Header And Overflow In Preview

1. Connect to a session and open the terminal view
2. Run:

```bash
cat >/tmp/zedra-markdown-table.md <<'EOF'
# Table Test

| Command | Description | Status |
| - | - | - |
| `printf '%s\n' value` | this is a very long description that should wrap inside the capped table column on mobile | ready |
EOF
printf '/tmp/zedra-markdown-table.md:1\n'
```

3. Tap `/tmp/zedra-markdown-table.md:1`
4. Expected: the preview opens in markdown mode
5. Expected: the table header row renders `Command`, `Description`, and `Status`
6. Expected: table cell padding is compact like the code block padding
7. Expected: the long description wraps inside the capped column instead of making the table extremely wide
8. Swipe horizontally inside the table
9. Expected: the table scrolls horizontally without changing the surrounding vertical scroll position
10. Swipe vertically starting inside the table
11. Expected: the markdown preview scrolls vertically and the table does not drift horizontally

## 15c. Markdown Bottom Padding And Link Hit Slop

1. Connect to a session and open the terminal view
2. Run:

```bash
cat >/tmp/zedra-markdown-links.md <<'EOF'
# Link Test

Tap near [Zed](https://zed.dev), including slightly outside the blue text.

Last visible line.
EOF
printf '/tmp/zedra-markdown-links.md:1\n'
```

3. Tap `/tmp/zedra-markdown-links.md:1`
4. Expected: the preview opens in markdown mode
5. Scroll to the bottom
6. Expected: there is enough bottom padding to keep `Last visible line.` above the home indicator or sheet edge
7. Tap slightly outside the visible `Zed` link text
8. Expected: the link still opens, without starting text selection

## 16. iOS Native Selection In Markdown Preview

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

```bash
cat >/tmp/zedra-markdown-selection.md <<'EOF'
# Selection Test

This paragraph should support native iOS selection inside the markdown preview.

[Open Zedra](https://zedra.dev)

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
11. With the selection still active, tap empty space below the short markdown content inside the main view
12. Expected: the native selection handles dismiss and the markdown preview remains focused for normal scrolling
13. Repeat the selection, then tap `Copy`
14. Expected: the selection menu dismisses cleanly and the preview remains responsive to scrolling and link taps afterward
15. Tap `Open Zedra`
16. Expected: `https://zedra.dev` opens externally

## 16a. iOS Native Selection In Code Preview

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

```bash
cat >/tmp/zedra-code-selection.rs <<'EOF'
fn main() {
    let answer = 42;
    println!("{answer}");
}
EOF
printf '\033]8;;file:///tmp/zedra-code-selection.rs:1:1\033\\/tmp/zedra-code-selection.rs:1\033]8;;\033\\\n'
```

3. Tap `/tmp/zedra-code-selection.rs:1`
4. Expected: the preview opens in code editor mode
5. Expected: the code view does not show the old blue logical caret
6. Long-press inside a code line
7. Expected: native iOS selection handles and the system edit menu appear for the code text
8. Open a small code file from the workspace drawer so the code editor is visible as the main workspace view
9. Tap the workspace header drawer and quick-action buttons
10. Expected: the header buttons open their overlays immediately instead of waiting for selection recognition or starting selection in the code editor below
11. With the workspace drawer open over the code editor, tap drawer tabs and controls
12. Expected: drawer touches interact with the drawer immediately, including active tab changes, and do not start selection in the editor behind it
13. With a code selection active, tap the workspace header or open drawer controls
14. Expected: the selection dismisses and the tapped control responds immediately
15. With a code selection active, tap empty space inside the editor but outside any code text
16. Expected: the native selection dismisses instead of leaving stuck handles
17. Drag a selection handle into empty horizontal or vertical space inside the editor
18. Expected: the handle tracks to the nearest sensible text boundary instead of jumping to select all text before or after the handle
19. Tap the code view and type with a hardware keyboard
20. Expected: the file content remains unchanged because the editor is read-only

## 16b. Mention Editor Selection In Agent Terminal

1. Connect to a workspace with terminals running `claude`, `codex`, `opencode`, or another detected AI agent CLI such as `gemini`
2. Open a source file from the workspace drawer so the main workspace code editor is visible
3. Long-press inside a code line and drag the selection handles across multiple lines
4. Tap `Add to Chat` in the native selection menu next to `Copy`
5. Expected: `Add to Chat` shows the Zedra icon in the selection menu
6. Expected: a native selection sheet lists detected supported AI-agent terminals by terminal title without a `Terminal N` prefix; emoji and spinner glyphs are omitted from the picker labels, and iOS shows the bundled agent SVG icon in both light and dark appearance
7. Pick the Claude terminal
8. Expected: the main view switches to the selected terminal first, then the selected range is pasted into Claude as an `@file#Lx-Ly` mention after a short delay; it is not submitted automatically
9. Repeat for Codex, opencode, Gemini, or another detected non-shell agent
10. Expected: the main view switches to the selected terminal first, then the selected range is pasted as fenced context with the source range after a short delay; it is not submitted automatically
11. Open a markdown file and select text in a paragraph or code block
12. Drag the markdown selection handles to extend and shrink the selected range before opening the menu
13. Tap `Add to Chat`, pick an agent terminal, and verify the selected source lines are pasted into that terminal
14. Exit all supported AI-agent CLIs, select editor text, and tap `Add to Chat`
15. Expected: the native selection sheet shows `No AI agent detected` and no text is inserted

## 16c. Workspace Markdown File Rendering

1. Connect to a workspace with a `README.md` or another markdown file
2. Open the workspace drawer file list and tap the markdown file
3. Expected: the drawer closes and the main workspace editor renders the file with markdown formatting instead of the code editor line gutter
4. Expected: the workspace header subtitle shows the file's relative path, not the workspace cwd or an absolute path
5. Open a non-markdown source file from the same file list
6. Expected: the main workspace editor still opens the code editor with syntax highlighting and line numbers
7. Open a large markdown file with many headings, paragraphs, lists, and fenced code blocks
8. Expected: the rendered markdown has horizontal padding on both sides, including while scrolling
9. Expected: the workspace remains responsive while the file loads and scrolling does not repeatedly reparse or rebuild every markdown block

## 17. Native Confirmations For Terminal Delete And Session Disconnect

1. Connect to a session with at least two terminals open
2. Open the workspace drawer Terminals tab and tap the close affordance on one terminal card
3. Expected: a native confirmation alert appears with `Delete` and `Cancel`
4. Tap outside the alert
5. Expected: the alert dismisses and the terminal card remains visible
6. Trigger the same delete again and tap `Cancel`
7. Expected: the alert dismisses and the terminal card remains visible
8. Trigger the same delete again and tap `Delete`
9. Expected: the terminal card is removed from the drawer immediately, without waiting for the remote delete RPC to finish
10. Expected: if the deleted terminal was the active main view, the main view switches to another terminal
11. Delete the remaining terminals one by one
12. Expected: after the last terminal is deleted, the main view shows `No active terminal`
13. Repeat terminal deletion from the quick action panel
14. Expected: the same native confirmation alert appears there, and confirmed deletion removes the card immediately
15. Open the Session tab and tap `Disconnect`
16. Expected: a native confirmation alert appears with `Disconnect` and `Cancel`
17. Tap outside the alert
18. Expected: the alert dismisses and the session remains connected
19. Tap `Cancel`, then retry and tap `Disconnect`
20. Expected: the session disconnects only after confirmation and the app returns to Home
21. Expected: the home workspace card immediately shows the disconnected/reconnect state instead of the old connected state
22. Tap the disconnected workspace card
23. Expected: the connect view title is `Disconnected` and the subtitle is `Tap refresh to reconnect.`

## 18. Workspace Header Terminal Title + Terminal Agent Icon

Zedra currently uses OSC 1 icon-name updates as the primary active-agent signal.
Shells should emit the command name as OSC 1 when an AI CLI starts, then a
non-agent prompt/path icon name when the shell returns to prompt.
Launch-command terminals seed identity from the known command until shell OSC
metadata arrives.

1. Open a workspace, open a terminal.
2. Type `claude` (or another supported AI CLI such as `opencode`, `codex`,
   `gemini`, `amp`, `cline`, `cursor-agent`, `goose`, `hermes-agent`,
   `junie`, `kilo`, `openclaw`, `openhands`, `pi`, `qodercli`, `qwen`, or
   `trae-cli`) and observe it while running, then exit and wait for the shell
   prompt.

Expected:
- Header subtitle (below project name) updates from cwd to the terminal title.
- Header subtitle does not show an agent icon.
- Agent icon appears in the terminal card matching the running CLI, based on OSC
  1 icon-name updates rather than terminal title text.
- If the agent changes the terminal title while running, the icon remains
  stable in the terminal card.
- After the agent exits and shell returns to prompt, terminal card icon disappears;
  subtitle shows the last title (or falls back to cwd), even if the title still
  mentions the agent.
- Switching to a different terminal in the drawer updates header to the
  active terminal's title and updates the terminal card icon.
- Relaunch the client app while an AI CLI command is still running in the
  remote terminal. Expected: after reconnect/reattach, the terminal card
  restores the agent icon from host-persisted OSC 1 metadata without waiting for
  a new command to start or a new OSC 1 event to arrive.
- Start a Claude resume terminal through `/zedra-start` or another
  `launch_cmd` path such as `claude --resume <session_id>`. Expected: while
  Claude is running, the terminal card shows the Claude icon even before a shell
  prompt emits fresh OSC metadata.

## 19. Xcode Rust Build Target

1. Open `ios/Zedra.xcworkspace` in Xcode
2. Select an iOS simulator destination and press Build
3. Expected: the build log shows `ZedraRustFFI`, then `Building Rust for Xcode (..., iphonesimulator, ...)`, before `ProcessXCFramework`
4. Select a connected iOS device destination and press Build
5. Expected: the build log shows `ZedraRustFFI`, then `Building Rust for Xcode (..., iphoneos, ...)`, before `ProcessXCFramework`
6. Select the `Zedra Release` scheme with the connected iOS device destination and press Run
7. Expected: the Rust build log includes `Release mode enabled`, and Xcode installs and launches the production `Zedra` app
8. Run `./scripts/build-ios.sh --device --release --debug`
9. Expected: the command fails before compiling and says iOS release builds cannot enable debug flags
10. Archive the app with the Release configuration
11. Expected: the archive uses the production bundle id `dev.zedra.app`, generates a dSYM, and has Xcode Release strip/validation settings enabled

## 20. iOS Orientation Support

1. Launch the app on an iPhone or iOS simulator in portrait orientation
2. Rotate the device or simulator to landscape left and landscape right
3. Expected: Zedra remains in upright portrait orientation and does not relayout into a landscape window
4. Archive the Release build and inspect the generated `Info.plist`
5. Expected: the base `UISupportedInterfaceOrientations` key contains `UIInterfaceOrientationPortrait`
6. Expected: the iPad-specific `UISupportedInterfaceOrientations~ipad` key contains portrait, portrait upside down, landscape left, and landscape right
