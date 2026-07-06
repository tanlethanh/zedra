# Manual Test Plan

## Agent Notes

- For UI, platform, and device-driven changes, agents should add or update the relevant manual verification steps in this document.
- Prefer concrete reproduction steps and expected results over vague test descriptions.
- When debugging, add targeted log instructions if the test depends on developer-run device validation.

## 0-Icons. Shared Icon Pipeline (iOS + Android)

1. Open Settings and tap the sign-in entry to present the account picker sheet
2. Expected (iOS): the `Sign in with Google` button shows the Google icon and, on iOS, `Sign in with Apple` shows the Apple icon — both resolved from `assets/icons/{google,apple}.svg`
3. Repeat on Android (`./scripts/run-android.sh`); expected: both buttons render the same icons (Android drawable looked up by slug)
4. Open the agent/target picker so agent icons render; expected: each agent shows its icon (e.g. Codex shows the OpenAI mark) identically to the in-app GPUI list
5. Regression check: delete `ios/Zedra/Assets.xcassets/*.imageset` and `android/app/src/generated/res`, then run a normal app build; expected: the build regenerates them (Xcode pre-build script / Gradle `generateIconDrawables` task) and icons still render

## 0-Compat. Legacy ALPN (`zedra/rpc/2`) Client

Verifies a `zedra/rpc/3` host still serves a pre-bump app.

1. Run the host from this branch; connect a `zedra/rpc/2` app (a `v0.2.5` build or
   a client pinned to `ZEDRA_ALPN_V2`).
2. Expected: host log shows `alpn=zedra/rpc/2`, auth succeeds (no `InvalidData`),
   and terminals (open/type/resize/reconnect) and the agent list all work.
   Claude/Codex/OpenCode render; agent live badges may be blank and `Pi`/`Hermes`
   are absent (expected).
3. Connect a current `zedra/rpc/3` app in parallel — both work concurrently, and
   the `v3` client still shows full terminal/agent metadata.

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

## 0b-Status. Connection Status Indicator

1. From Home, tap the status dot on a saved workspace card without tapping the card body
2. Expected: light haptic, the app navigates to that workspace, and the connecting view opens
3. Tap the card body outside the dot
4. Expected: the workspace still opens, but the card tap path is unchanged from before
5. In a connected workspace, tap the status dot beside the header title
6. Expected: the connecting view opens and any open workspace drawer closes
7. Open the workspace drawer and tap the status dot in the drawer header
8. Expected: the connecting view opens and the drawer closes
9. Open Quick Actions and tap the status dot on a workspace row, not the row body or `+` button
10. Expected: Quick Actions closes, that workspace becomes active, and the connecting view opens
11. While a workspace is idle or reconnecting, confirm the status dot pulses opacity and scale without shifting header layout

## 0b. Home Settings Button

1. Run a Debug build and open the Home screen
2. Tap the top-right settings icon
3. Expected: a light haptic feedback fires and the Settings screen opens
4. Run a Release build and open the Home screen
5. Expected: the settings icon is not visible and the developer Settings screen is not reachable from Home

## 0c. Developer Native Notification (iOS)

1. Run a Debug build and open Settings
2. Tap `Native Notification` in the Developer section
3. Expected: two compact in-app notification bubbles slide down near the top safe area, with the newest expanded in front and the older one peeking above it as a smaller glass bubble
4. Expected: the front bubble uses the app asset icon `openai` (the Codex agent icon slug) tinted like the notification title; the upper peek bubble uses an SF Symbol fallback icon
5. Expected: each new bubble fades in faster than it scales, then continues sliding down and settling into place with a subtle spring
6. Tap the front `Agent completed` bubble
7. Expected: it fades out faster than it scales, then finishes scaling back toward zero while sliding upward; a callback notification appears
8. Tap `Native Notification` again, then swipe the front bubble downward
9. Expected: all pending notification items expand into full bubbles, with the oldest at the top and the newest at the bottom
10. Swipe any expanded bubble upward
11. Expected: only that bubble dismisses, and the remaining bubbles move up smoothly
12. Tap `Native Notification` repeatedly
13. Expected: multiple notifications collect into the same bubble stack and all auto-close after their configured durations by default

## 0c-Settings. Delta Settings Flow

1. Run a Debug build and open Settings
2. Expected: `Profile` and `Notifications` appear above `Appearance`
3. Start sign-in or notification registration
4. Expected: progress or error status replaces the relevant row description, with no separate `Status` row and no combined status/description text
5. Sign in to Delta, then tap the logout icon in the profile row
6. Expected: a native confirmation alert appears; cancelling keeps the profile signed in, and confirming returns the profile row to `Sign In`

### Mobile node metadata reconciliation

1. Sign in to Delta on iOS or Android and confirm the mobile node appears in the stack.
2. Close and relaunch the app.
3. Expected: logs contain `Delta mobile node reconciliation completed` with `Unchanged`.
4. Change the app version, build number, OS version, or device name, then relaunch the app.
5. Expected: reconciliation reports `Updated`, and the stack node contains the new metadata.
6. Delete the mobile node from the Delta stack, then relaunch the app.
7. Expected: reconciliation reports `SignedOut`, and Settings shows Delta as signed out.

### Workspace host binding reconciliation

1. Connect to a workspace while Delta is signed out.
2. Sign in to Delta after the workspace is already connected.
3. Expected: the app registers or reuses the workspace's host Delta node, then updates the host-side `DeltaClient` without requiring a full reconnect.
4. Quit and relaunch the app with the workspace still saved.
5. Expected: the workspace reloads its persisted host pubkey and host node id, and the host binding is restored once Delta sign-in is available again.
6. If the host node is deleted from Delta, reconnect the workspace.
7. Expected: the app registers a fresh host node id for that workspace and persists the new binding.
8. Sign out of Delta while the workspace remains connected.
9. Expected: the host-side in-memory Delta binding is cleared, and new agent-hook notifications stop until the workspace binding is replayed after sign-in.

## 0c-Android. Native Presentations And Embedded Sheet

1. Run a Debug Android build and open Settings
2. Tap the developer alert and selection presentation actions
3. Expected: native Material dialogs appear; the selection dialog shows `First Action`, `Second Action`, `Destructive Action`, and `Cancel` rows with visible row separation, not just the title
4. Expected: button callbacks fire once, and dismissing the selection reports a dismiss rather than choosing the last item
5. Tap `Native Notification`
6. Expected: native notification banners appear near the top safe area, auto-close by default, and tapping the action banner triggers the callback notification
7. Trigger the text input dialog from an existing call site
8. Expected: the native text field shows the initial value, `OK` returns the entered value, and `Cancel`/outside dismissal returns no value
9. Open a terminal file link so the native custom sheet opens
10. Expected: a Material bottom sheet appears with a grabber when requested and GPUI-rendered preview content inside the embedded sheet surface
11. From a fresh sheet open, swipe the preview content upward and downward before it reaches top, then drag downward from the top of the preview
12. Expected: inner content scrolls while not at top; when it is at top, the bottom sheet can take the downward drag for detent/dismiss handoff without preview repaint glitches during the drag or dismissal
13. Long press selectable text inside the embedded sheet (markdown preview / editor content)
14. Expected: native selection highlight, handles, and the floating `Copy`/`Share`/`Search` toolbar appear over the sheet; dragging handles extends the selection and `Copy` copies the sheet's GPUI text. A normal tap still scrolls and does not start selection
14a. Drag a selection handle up and down across the sheet (and continue the same long-press gesture into a drag-to-extend)
14b. Expected: only the selection endpoint moves — the bottom sheet does not drag, change detent, or dismiss during the handle/extend drag, and sheet dragging works normally again after the selection is dismissed
15. Expected: dismissing or closing the sheet leaves no stale selection overlay, and root-window text selection still works after the sheet has been opened and closed
16. Trigger the scroll-to-bottom floating button
17. Expected: the native floating button appears at the GPUI wrapper bounds and pressing it runs the Rust callback
18. Trigger dictation preview events if the call site is available
19. Expected: Android displays the preview overlay and dismiss callback, without attempting iOS-specific dictation stream interpretation

## 0c-Android-Renderer. GPUI Surface Lifecycle

1. Run a Debug Android build on a physical device
2. Connect via QR and open a terminal
3. Expected: after returning from the scanner, logcat shows `ZedraApp: window activated`, the pending ticket is processed, and connect succeeds
4. Type, scroll, and fling terminal output for at least 30 seconds
5. Expected: scrolling remains smooth and fling momentum does not continue after a drawer drag calls `prevent_default()`
5a. Slowly drag-scroll the terminal through scrollback a fraction of a row at a time
5b. Expected: the row entering at the top edge (and leaving at the bottom edge) slides in pixel by pixel; rows do not pop in only once fully visible
6. Open and close the workspace drawer while nested terminal/editor content is scrollable
7. Expected: drawer drags do not scroll the inner content, and inner vertical scroll does not move the drawer once the drawer gesture is not claimed
8. Background the app, wait 5 seconds, then foreground it
9. Expected: the existing GPUI window resumes without recreating all renderer state, and no surface validation or device-lost crash appears in logcat
10. Rotate the device to landscape left and landscape right
11. Expected: the app remains in portrait orientation and the existing workspace/session state stays visible instead of returning to the initial launch view
12. Expected: the app redraws at full physical surface resolution with `scale_factor = density`, with no fixed 0.75 render scale
13. On an Android 15 device, confirm content is not obscured by the status bar, gesture navigation handle, 3-button navigation bar, or display cutout
14. Confirm outlined buttons, cards, and input borders are visible even when their background is transparent
15. In the terminal, render `✔ ✘ ⚠ ⏺ ⏹ ⏸` and a real emoji such as `😀`
16. Expected: terminal/UI symbols render as monochrome symbol glyphs, while the real emoji renders through Android color emoji fallback before and after attempting rotation

## 0c-Android-Selection. Native Text Selection

1. Open the Home install guide and long press selectable command or comment text
2. Expected: Android-native selection highlights, handles, and floating `Copy` toolbar appear without opening the software keyboard
3. Drag both handles across multiple lines
4. Expected: highlights and handles follow the selected GPUI text without scrolling the content underneath the dragged handle
5. Tap `Copy`
6. Expected: the exact selected text is written to the Android clipboard and the native selection UI dismisses
7. Open a terminal with visible output and tap normally
8. Expected: existing terminal focus and keyboard behavior remains unchanged
9. Long press terminal output, then drag both handles across rows
10. Expected: native selection starts only after long press and terminal output selection follows the handles
11. Tap outside an active selection, scroll selected content, then switch views
12. Expected: selection dismisses or refreshes cleanly without stale highlights, handles, or toolbar

## 0c-Android-AppIds. Debug And Release Application IDs

1. Run `./scripts/run-android.sh --target arm64-v8a`
2. Expected: Android installs and launches the debug app id `dev.zedra.app.debug` with the launcher label `Zedra Dev`
3. Expected: the launcher, app info, and recents icons show the black Zedra lightning icon instead of the default Android robot, including on round-icon launchers
4. Expected: startup logcat has no `getAppVersion` / `getAppBuildNumber` JavaException and no GPUI atlas panic during the first surface draw
5. Confirm Android release signing properties are present in global Gradle config: `ZEDRA_KEYSTORE`, `ZEDRA_KEYSTORE_ALIAS`, and `ZEDRA_KEYSTORE_PASSWORD`
6. Run `./scripts/run-android.sh --release --target arm64-v8a`
7. Expected: Android installs and launches the release app id `dev.zedra.app` with the normal app label, and it can coexist with the debug build

## 0d. Firebase GPUI Screen Views

1. Run an iOS build with Firebase Analytics enabled, or add `android/google-services.json` with the `dev.zedra.app` client and run `./scripts/run-android.sh --release --target arm64-v8a`
2. Open Home, Settings, Quick Actions, then connect to a workspace
3. Open the workspace drawer and switch through Files, Documents, Git Diff, Terminals, and Session
4. Open a non-markdown file, a markdown file, a git diff, a terminal, and the managed-agent view as the main workspace view
5. Tap terminal file links for both a source file and a markdown file so the native custom sheet opens
6. Expected: manual `screen_view` events include `screen_name` and `screen_class` for `Home`, `Settings`, `Quick Actions`, `Workspace Connecting`, `Workspace Editor`, `Workspace Markdown`, `Workspace Git Diff`, `Workspace Terminal`, each drawer tab, `Custom Sheet Editor`, and `Custom Sheet Markdown`
7. Expected: no native automatic screen rows (e.g. `UIViewController`, `CustomSheetViewController`, Android activity rows). Automatic screen reporting is disabled (`FirebaseAutomaticScreenReportingEnabled`/`firebaseAutomaticScreenReportingEnabled`) because the UI is GPUI, not native view controllers; screen tracking comes solely from the manual `screen_view` events above

## 0d-Telemetry. Persisted Telemetry Opt-Out

Use a `debug-telemetry` build so every event prints `[telemetry] >> <name>` to stderr
(iOS: `./scripts/ios-log.sh`; Android: `./scripts/android-log.sh`).

1. Fresh install (or clear app data), then launch. Expected: `[telemetry] >> app_open` appears
   (default is opted-in), and `[debug:telemetry] applied persisted opt-out enabled=true`.
2. Open Settings → Privacy and set "Share usage data" to **Off**. Expected: a selection haptic
   fires and the toggle moves to Off.
3. Stay in the app and navigate to another screen. Expected: **no** `[telemetry] >>` lines.
4. Set "Share usage data" back to **On**, then navigate again. Expected: subsequent events
   resume; events from the disabled interval are not backfilled.
5. Set "Share usage data" back to **Off**, fully quit, and relaunch. Expected: **no**
   `[telemetry] >>` lines at all, including no
   `app_open`, and `[debug:telemetry] applied persisted opt-out enabled=false`.
6. Open Settings → Privacy and set "Share usage data" back to **On**, then relaunch.
   Expected: `[telemetry] >> app_open` resumes and events fire again.
7. Tap Settings → Privacy → "Telemetry docs". Expected: the system browser opens
   `zedra.dev/docs/telemetry`.

## 0d-Telemetry. Compile-Time Mobile Opt-Out

1. Build, install, and launch with `./scripts/run-ios.sh sim --no-telemetry` or
   `./scripts/run-android.sh device --no-telemetry`.
2. Expected: Settings shows a muted Privacy telemetry row with a non-interactive Off
   control and the description "Telemetry disabled when this app was built". No Firebase
   Analytics or Crashlytics calls are made. On Android, Delta push notifications remain
   available.

## 0e. Developer Native Selection

1. Run a Debug iOS build and open Settings
2. Tap `Native Selection` in the Developer section
3. Expected: a native action sheet opens without crashing and shows `First Action`, `Second Action`, `Destructive Action`, and `Cancel`
4. Tap `Cancel`
5. Expected: the action sheet dismisses without running another action
6. Open `Native Selection` again, then tap outside the sheet
7. Expected: the sheet dismisses without crashing

## 0f. Delta Agent Hooks

1. Sign in the host with Delta and confirm `zedra stack list` lists at least one push-enabled mobile node
2. Run `zedra setup claude`, `zedra setup codex`, `zedra setup opencode`, and `zedra setup pi`
3. Start a new Claude, Codex, opencode, and pi session
4. Submit a prompt, trigger a tool approval, use Add to Chat from a Zedra editor selection when an agent terminal is active, and let the task finish
5. Expected: each agent event produces a Delta notification only on the previous signed-in mobile client, without notifying other devices in the stack or exposing prompt, tool output, diff, file path, or terminal contents
6. Expected for pi: the working indicator turns on when a prompt is submitted and a `Pi completed` notification fires on turn end; pi has no approval event, so no waiting/approval notification is expected
7. If an iOS Live Activity token is registered for the same `activity_id`, expected: the Live Activity changes to working, waiting, selection, and done/end states as the hooks fire
8. Run each setup command again
9. Expected: hook installation remains idempotent and does not duplicate hook entries (for pi, `~/.pi/agent/extensions/zedra-agent-hooks.ts` is rewritten in place)

### Pi hook smoke test (no live model)

1. Run `zedra setup pi`, then start any pi session inside a Zedra terminal
2. From another shell on the host, run `zedra agent hook test --agent pi --event UserPromptSubmit --terminal-id <id>` then `... --event Stop --terminal-id <id>`
3. Expected: the agent state transitions Running → Completed, and a `Pi completed` Delta notification fires when the app is backgrounded

## 0f-1. Agent Hook Notification Deeplink — App In Background

Requires Delta sign-in, a registered push-enabled device, and hook setup
(`zedra setup claude` or equivalent).

**Connected workspace, app in background:**
1. Connect to a workspace and open a Claude terminal with a running session
2. Background the app (home button / swipe up, do not force-quit)
3. From the Claude session, submit a prompt or trigger a tool approval so a hook fires
4. Expected: a push notification arrives with a title such as `Claude requires approval` or `Claude turn finished`
5. Tap the notification
6. Expected: the app foregrounds and navigates directly to the terminal that generated the hook event, without requiring any manual workspace selection or terminal tap
7. Expected: no duplicate or blank terminal view appears

**Saved (disconnected) workspace, app in foreground or background:**
1. Disconnect the workspace or force-quit and relaunch the app (workspace card visible on Home but not connected)
2. From the host, trigger a hook event in a previously-known terminal
3. Tap the push notification
4. Expected: the app begins reconnecting to the workspace; the connecting view appears
5. Expected: after sync completes, the app navigates automatically to the terminal from the deeplink without user interaction
6. Expected: if the terminal from the deeplink no longer exists on the host after sync (stale terminal), the app falls back to the first available terminal or creates a new one

**Validation guards (must not notify):**
1. On the host, run `claude` in a plain shell that was **not** started from Zedra (no `ZEDRA_TERMINAL_ID` set); trigger a hook event
2. Expected: no push notification is sent — hooks from terminals not registered in the daemon's session registry are silently dropped
3. Confirm in daemon logs that the hook was received and discarded without an error

## 0f-2. Delta Integration — Anonymous Host Registration

Tests the path where the host has **not** run `zedra auth login` but the mobile
app is signed in. On connect the app registers the host's public key with Delta
and reports the result back to the daemon via `SetClientDeltaInfo`, enabling
agent-hook notifications without host credentials.

### Setup

- Host daemon running with no `~/.config/zedra/delta.json` (or remove it with
  `zedra auth logout` if previously signed in).
- Mobile app signed in to Delta (`Settings → Delta → Sign In`).
- At least one push-enabled mobile node registered (`zedra stack list` shows it
  on the *mobile* app — host CLI doesn't need to be authed for this check).

### 1. Dedicated Delta host key registered on connect

1. Start the daemon: `zedra start --workdir .`
2. Confirm `~/.config/zedra/delta.key` exists and is distinct from the
   workspace `identity.key`.
3. Scan QR from the mobile app and let the connection reach `Connected`.
4. Expected: host daemon log contains a line matching
   `Delta host node registered  created=true host_node_id=…` (or `created=false`
   if the same key was registered before).
5. Expected: host daemon log contains `Delta client updated from connected mobile client`
   with `stack_id`, `client_node_id`, and `host_node_id`.
6. Check that the host node appears in the Delta stack from the mobile app's
   `Settings → Delta → Stack Nodes`.
7. Expected: the host node is listed with kind `host` and matches the machine
   hostname.

### 2. Agent-hook notification without host sign-in

1. With the daemon running and the signed-in mobile app connected, background
   the app and trigger an agent completion hook.
2. Expected: the daemon sends a notification to that mobile client's
   `client_node_id`, not every device in the stack.
3. Expected: the push notification arrives on that mobile device.
4. Stop and restart the daemon:
   ```sh
   zedra stop --workdir .
   zedra start --workdir .
   ```
5. Reconnect the signed-in mobile app, background it, and trigger another agent
   completion hook.
6. Expected: the app restores the daemon's in-memory client Delta info and the
   notification arrives on that mobile device.

### 3. Re-registration is idempotent (`created` flag)

1. While the daemon is running and the app is connected, force a reconnect
   (background and foreground the app, or run `zedra client --workdir . --count 1`
   to verify the session is live).
2. Expected: daemon log shows `created=false` on the second sync-complete after
   the same mobile client reconnects — the existing host node is returned, not
   duplicated.
3. Pair a **new** device (different mobile) and let it connect.
4. Expected: daemon log shows `created=false` because both mobile clients
   register the same host `delta.key` public key. No duplicate host node is
   created.

### 4. Signed-in host CLI config

1. Sign in the host: `zedra auth login`
2. Run `zedra send` and `zedra send --live-activity`.
3. Expected: both commands use the signed-in host config, confirmed by
   `zedra auth status` showing the active config.

### 5. Error message without host sign-in

1. Remove `delta.json` with `zedra auth logout`.
2. Run `zedra send any --workdir . --title test`.
3. Expected: command exits non-zero with the message:
   `Delta not configured. Sign in with \`zedra auth login\`.`

## 0g. Android Delta Push + In-App Banner

Requires `android/google-services.json` from the Firebase project. Without it the
app still builds, but push registration reports an error instead of a token.

1. Drop `google-services.json` into `android/`, then build and install: `./scripts/build-android.sh && cd android && ./gradlew installDebug`
2. Open Settings → Delta and tap the push token row
3. On Android 13+, expected: the system prompts for the notification permission; grant it
4. Expected: registration succeeds, the row shows provider `fcm`, and `zedra stack list` lists the device (labeled with its `Build.MODEL`) as push-enabled
5. From the host, trigger a Delta notification while the app is backgrounded
6. Expected: a system notification appears in the `Delta notifications` channel; tapping it opens the app (and follows the `deeplink` data field when present)
7. Trigger a notification (or tap a Developer in-app notification action) while the app is foregrounded
8. Expected: an in-app banner slides up from the bottom, tinted by kind, auto-closing for transient banners; tapping it fires the action and suppresses the dismiss callback
9. Build without `google-services.json` and repeat step 2
10. Expected: the push row reports a configuration error and does not crash

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

## 1a-Android. System Back Navigation

1. On Android, connect to a workspace and open Quick Actions from the workspace header
2. Press the system Back button or gesture
3. Expected: Quick Actions closes and the app remains on the workspace
4. Open Settings from Home, then press system Back
5. Expected: Settings returns to Home
6. Connect to a workspace, open the workspace drawer, then press system Back
7. Expected: the workspace drawer closes
8. Open the connecting overlay from the Session tab, then press system Back
9. Expected: the connecting overlay closes and the workspace content remains visible
10. Open a terminal, then a file, then a git diff, then reopen the same terminal
11. Press system Back repeatedly
12. Expected: Back visits the previous distinct main content views in order, without duplicate entries for the reopened terminal

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
12. Open the floating file search and type part of a file or folder name that is not currently loaded in the expanded File Explorer tree
13. Expected: the host searches recursively and the floating results show fuzzy matching files and folders case-insensitively, without expanding or collapsing the underlying tree
14. Clear the floating search with its clear control or `Esc`
15. Expected: the previous browsing context returns with the same expanded directories, loaded rows, and active file highlight
16. Expected: before syntax highlighting appears, code text uses a subtly dim foreground; when highlighting applies, text rows do not jump, reorder, or visibly reload

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

## 1d. Windows Host CLI

1. On an x86_64 Windows machine, run `powershell -c "irm https://zedra.dev/install.ps1 | iex"` from Command Prompt or Windows Terminal
2. Expected: `zedra.exe` is installed under `%LOCALAPPDATA%\Programs\zedra\bin`, the directory is added to the user `Path`, and `zedra --help` works from the current shell
3. Start the daemon from PowerShell: `zedra start --workdir C:\path\to\repo --detach`
4. Expected: startup succeeds, Windows may show a firewall prompt, and `daemon.lock` plus `daemon.log` are written under `%APPDATA%\zedra\workspaces\` using their respective workspace hashes
5. Run `zedra status --workdir C:\path\to\repo`
6. Expected: status shows the running daemon, endpoint, workspace path, sessions, and terminal count without Unix path assumptions
7. Run `zedra qr --workdir C:\path\to\repo`, scan from the mobile app, then disconnect and reconnect the app without scanning again
8. Expected: QR pairing succeeds, reconnect uses the saved session identity, and relay fallback still works if direct P2P is unavailable
9. Open a terminal from the app
10. Expected: a Windows PTY opens with the shell that launched the host, keystrokes echo, resize works, and commands available on `PATH` run normally
11. Stop the daemon, set `$env:ZEDRA_SHELL = "cmd.exe"` in PowerShell, restart the daemon, and open another terminal from the app
12. Expected: the terminal opens in `cmd.exe`; clear `ZEDRA_SHELL`, restart from PowerShell, and launch commands still leave an interactive PowerShell after they run
13. Run `zedra client --workdir C:\path\to\repo --count 3`
14. Expected: the CLI client authenticates without QR and prints three RTT samples
15. Run `zedra logs --workdir C:\path\to\repo`
16. Expected: recent daemon log lines are printed from the AppData workspace directory
17. Run `zedra update --version <current-release-tag> --yes` while the daemon is still running
18. Expected: the update succeeds and warns that running daemons keep using the old version until restarted
19. Run `zedra stop --workdir C:\path\to\repo`
20. Expected: the daemon exits, the lock file is removed, and a follow-up `status` reports no running daemon
21. Run `zedra update --version <current-release-tag> --yes`
22. Expected: the update downloads the Windows release asset, verifies the checksum when available, and reports that `zedra.exe` will be replaced after the command exits
23. Open a new PowerShell window and run `zedra --version`
24. Expected: the command prints the release version

## 2. QR Already Consumed

1. Start host: `zedra start --workdir .`
2. Device A scans QR → connects successfully
3. Device B scans the **same** QR
4. Expected: Device B sees "The QR code was used. Refresh it and scan again." (not a crash)
5. To pair Device B: refresh the QR code and scan again

### Static QR

1. Start host: `zedra start --workdir .`
2. Generate a static QR: `zedra qr --workdir . --static`
3. Scan it from Device A and confirm the workspace connects
4. Disconnect Device A from the host session
5. Scan the same static QR from a second clean device
6. Expected: Device B connects without "used" or "expired" QR errors
7. Stop and restart the daemon
8. Expected: the old static QR no longer works; the app says the QR expired or was replaced
9. Restart the daemon and generate a fresh static QR if needed

## 2b. QR Rescan Restarts Existing Entry

Covers the rescan path through `Workspaces::connect_ticket` →
`Workspace::restart_with_ticket`. Every scenario must end with the same
entry index (no duplicate) and the entry transitioning into `BindingEndpoint`
or beyond.

1. Start host: `zedra start --workdir .`
2. Scan QR, let StaleTimestamp or any auth failure land the entry in `Failed`
3. Refresh the QR on the host, rescan from the same device
4. Expected: existing entry restarts and reaches `Connected`; no new entry is created
5. Repeat with a deliberately stuck Authenticating phase (e.g. pause network for
   ~10s mid-auth, then rescan a fresh QR before the entry times out)
6. Expected: in-flight attempt is aborted and the fresh ticket drives a new
   Register/Authenticate round on the same entry
7. While the entry is `Connected`, rescan a still-valid QR for the same endpoint
8. Expected: just switches to the entry; no restart, no flicker of the connecting view
9. Rescan twice in rapid succession (two fresh QRs back-to-back)
10. Expected: only the last attempt is alive; no overlapping connect loops
    (`grep` logs for `start connect to` — two entries are fine, but the first
    one must be followed by abort/cancel before the second's `Connected`)

## 2a. Protocol Version Mismatch

1. Run an app build and CLI/host build that use different `ZEDRA_ALPN` versions
2. Scan the host QR from the app
3. Expected: connect view shows "Protocol mismatch, Update App or CLI"

## 3. Continue Session from Saved Workspace

1. Connect via QR (test 1 above), create at least three terminals, and note their order in the drawer Terminals tab
2. Force-close the app
3. On another client, immediately connect to the same host session using a saved workspace or a fresh host QR
4. Expected: the second client connects without several `Host occupied` / retry attempts
5. Disconnect the second client, then reopen the original app and tap the saved workspace entry in the home screen
6. Expected: reconnects using stored session ID (no QR needed); terminal
   backlog replays any missed output
7. Expected: the workspace drawer Terminals tab shows the active remote
   terminals from the host without creating a replacement terminal
8. Expected: terminal cards appear in the same order they had before force-close

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

## 3b-1. Agent Icon Survives Reconnect

1. Connect, open a terminal, and start an agent that uses shell integration for inner commands (`codex` or `pi`)
2. While the agent is mid-task (running a tool command), background the app or toggle network to force a reconnect
3. Expected: after reattach, the terminal card keeps the agent icon and the agent state dot
4. Repeat while the agent is idle between turns (waiting at its prompt) and reconnect again
5. Expected: the agent icon is still shown; it only clears after the agent process actually exits
6. Repeat with `claude` as a control — icon persists in both cases

## 3c. Terminal Smooth Scroll Edge Rendering

1. Connect via QR and open a terminal with enough output to fill the scrollback
2. Slowly drag-scroll the terminal through scrollback a fraction of a row at a time
3. Expected: the row entering at the top edge (and the row leaving at the bottom edge) slides in pixel by pixel
4. Expected: rows do not pop in only once fully visible, and the edge gap never shows a blank partial row

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
2. In one terminal, run:
   ```sh
   sh -c 'i=0; while true; do echo "tick:$i"; i=$((i+1)); sleep 1; done'
   ```
3. In another terminal, run `cat`
4. Take the host machine offline (disable network interface)
5. Expected: badge shows `Reconnecting (N) · Ns` with a per-second countdown, up to 3 attempts
6. After 3 attempts: connect view shows "Host unreachable. Check network and host."
7. Bring host network back up, tap "Retry"
8. Expected: reconnects successfully
9. Expected: the ticking terminal continues receiving new `tick:<n>` output after reconnect, not only replayed backlog
10. In the `cat` terminal, type `after-reconnect` and press Enter
11. Expected: `after-reconnect` echoes once, proving the reattached input stream still reaches the PTY
12. Expected: the workspace drawer Terminals tab preserves the pre-reconnect terminal order
13. If the host was restarted and no remote terminals remain, expected: a fresh terminal is created and opened
14. Expected: host logs do not report reconnect teardown as `ERROR zedra_host::rpc_daemon: TermAttach: input receiver error: Io error`

## 5a. Idle Before Reconnect

1. Connect via QR and wait for the session badge to show "Connected"
2. Disable the host network interface or disconnect the client from the network without closing the app
3. Expected within about 4 seconds: session badge changes to "Idle Ns" while the session is still present; workspace status dots turn yellow and blink
4. Keep waiting
5. Background the app, then bring it back to the foreground while the badge is still `Idle`
6. Expected: reconnect starts immediately on resume (`Idle` -> `Reconnecting` -> `Connected` or `Disconnected`), without waiting for the idle loop to time out
7. Repeat with a shorter background interval where the badge still shows `Connected`
8. Expected: foreground resume probes liveness; if the host is unreachable, reconnect starts instead of leaving the active indicator stuck
9. Restore network before reconnect exhausts
10. Expected: badge returns from `Idle` or `Connected` to `Connected` during reconnect recovery

## 5a.1. Foreground Resume Respects User Disconnect

1. Connect via QR and wait for the session badge to show "Connected"
2. Tap the Session Disconnect button (the intentional disconnect path, not backgrounding)
3. Expected: badge goes to `Disconnected`/home and `SessionHandle::user_disconnect()` is set
4. Background the app for a few seconds, then bring it back to the foreground
5. Expected: the workspace is **not** resurrected — no liveness probe fires, no reconnect/restart starts, and the home/disconnected state remains
6. Tap the workspace again to reconnect explicitly and confirm recovery still works

## 5a.2. Foreground Resume Skips Probe When Transport Already Gone

1. Connect via QR and wait for "Connected"
2. Background the app long enough that the iOS lifecycle takes the active QUIC connection (`close_transport_for_lifecycle`) without the phase advancing past `Connected`/`Idle`
3. Bring the app back to the foreground
4. Expected: resume does **not** spend ~`FOREGROUND_LIVENESS_TIMEOUT` (2s) probing the stale RPC client; reconnect/restart begins promptly because `active_connection_id()` is `None` while the phase still reads `Connected`/`Idle`

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
5. Manually disconnect Device A, then immediately retry Device B
6. Expected: Device B attaches on the first retry
7. Reconnect Device A, then background it without quitting
8. Immediately retry Device B
9. Expected: Device B is still blocked because backgrounding alone keeps Device A's session active
10. Force-quit Device A from the app switcher, then immediately retry Device B
11. Expected: Device B attaches without several "Host occupied" retry attempts
12. Reconnect Device A again, then network-disconnect Device A without a graceful client close
13. Retry Device B after the active-client stale window
14. Expected: the host evicts the stale Device A connection without waiting for the full transport timeout

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
5. Expected: the terminal file preview opens for `src/main.rs` at line 12/column metadata and does not reuse any previous preview scroll position
6. Expected: the preview header metadata and code body both render with the app monospace font rather than a proportional fallback
7. Tap the underlined OSC 8 `zedra.dev`
8. Expected: the URL opens externally
9. Tap the plain `src/main.rs:12:3`, `git:(refactor-app-session-architecture)`, `hello`, `README`, `v0.112.0`, `gpt-5.4`, and `/model`
10. Expected: none of those tokens are treated as hyperlinks and no preview sheet opens

## 9b. Terminal Preview Sheet Gesture Ownership

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Run:

```bash
{
  printf 'fn main() {\n'
  for i in $(seq 1 80); do
    if [ "$i" = 40 ]; then
      printf '    let message = "this line is intentionally very long so the terminal preview code editor needs horizontal scrolling inside the native custom sheet without moving the sheet detent or dismissing the sheet while the drag is horizontal";\n'
    else
      printf '    println!("line %02d");\n' "$i"
    fi
  done
  printf '}\n'
} > /tmp/zedra-long-code.rs
printf '\033]8;;file:///tmp/zedra-long-code.rs:41:1\033\\/tmp/zedra-long-code.rs:41\033]8;;\033\\\n'
```

3. Tap `/tmp/zedra-long-code.rs:41`
4. Expected: the preview opens in code editor mode inside the native custom sheet with line 41 at the top of the code body
5. Expected: Rust keywords and string tokens gain syntax colors after the preview finishes parsing
6. Swipe horizontally across the long string line
7. Expected: the code scrolls sideways and the native sheet does not move or dismiss
8. Swipe mostly vertically inside the preview body
9. Expected: the preview content scrolls vertically
10. Dismiss the sheet, tap `/tmp/zedra-long-code.rs:41` again, then drag downward before line 1 is visible
11. Expected: the code scrolls toward line 1 first; the opened line is not treated as the file top
12. Scroll to the top of the preview body, then drag downward
13. Expected: the native sheet moves or dismisses normally from the top edge

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

## 11. Terminal Keyboard And Native Selection On iOS

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Tap a non-hyperlink area of the terminal once
3. Expected: the terminal becomes focused, the software keyboard appears, and terminal input works
4. Expected: the visible terminal surface does not show a native full-height UIKit caret
5. Hide the keyboard so the terminal is unfocused, then double-tap visible terminal output
6. Expected: double tap behaves like normal tap input: the terminal focuses and requests the keyboard, and no terminal output selection starts
7. Tap a non-hyperlink area of the terminal once
8. Expected: because the terminal is focused and the keyboard is visible, the keyboard hides and terminal focus clears
9. Tap the terminal again, then long-press visible terminal output while the keyboard is visible
10. Expected: terminal enters terminal-owned output-selection mode without dismissing the keyboard or showing an editable caret
11. Drag the native selection handles to extend and shrink the output selection
12. Expected: selection remains active, terminal output is not replaceable, and keyboard/IME state is unchanged
13. Long-press hard-wrapped output, soft-wrapped output, emoji, and CJK text, then tap `Copy`
14. Expected: copied text preserves visible hard newlines, omits soft-wrap newlines, preserves non-ASCII text, and trims trailing blank cells
15. Tap visible terminal output outside the active selection once
16. Expected: terminal output selection clears, and that same dismiss tap does not toggle terminal focus or keyboard visibility
17. Long-press an empty terminal cell
18. Expected: a custom native edit menu appears slightly above the touch point with a `Paste` action even though no output text is under the finger
19. Tap `Paste`
20. Expected: if the clipboard has text, it is sent to the PTY through terminal paste handling; if the clipboard is empty, the menu dismisses without changing terminal focus or keyboard state
21. Tap a non-hyperlink area of the already-focused terminal again
22. Expected: the keyboard hides and terminal focus clears
23. Dismiss the keyboard through a platform control or hardware-keyboard state while the terminal remains focused
24. Expected: tapping terminal text again keeps focus and requests the software keyboard
25. With the keyboard visible, drag vertically in terminal content to scroll scrollback or a terminal app that handles touch scroll
26. Expected: the terminal scrolls without dismissing the keyboard or clearing terminal focus
27. In a fresh non-alt terminal with the keyboard visible, run a slow stream such as `for i in $(seq 1 20); do echo "line $i"; sleep .2; done`
28. Expected: early output continues from the top without being pushed upward into an empty lower gap; once the occupied rows reach the keyboard edge, the terminal lifts gradually and never more than the keyboard height
29. With retained scrollback, clear the terminal using `printf '\033[2J\033[Htop\n'`
30. Expected: the cleared content stays top-aligned instead of inheriting a full keyboard lift from old scrollback; TUI-authored blank layout rows still count as occupied space
31. In a fresh non-alt terminal with no scrollback, drag upward repeatedly past the prompt
32. Expected: the prompt does not drift downward into unbounded empty space, and the scroll-to-bottom button does not appear
33. With the keyboard still visible and enough scrollback to reach history top, drag upward until scrolling stops, then keep dragging slightly
34. Expected: the oldest scrollback rows can be revealed and are not clipped above the terminal viewport

## 11-Android. Terminal Keyboard And IME

1. Connect to a session on an Android device and open the terminal view
2. Tap a non-hyperlink area of the terminal once
3. Expected: the terminal becomes focused, the software keyboard appears, and terminal input works
4. Type plain text, press backspace, press enter, and type another command
5. Expected: committed text, delete, and enter reach the PTY exactly once without opening the dictation preview
6. Use an IME that composes text, such as Vietnamese Telex or Japanese, type a short composition, and accept it
7. Expected: composing text updates without duplicating committed characters, and the accepted text reaches the PTY once
8. With the keyboard visible, tap `Esc`, `Tab`, `Enter`, and each arrow in the accessory bar
9. Expected: each accessory key reaches the PTY exactly once
10. Press and hold each accessory arrow, then release it
11. Expected: the corresponding arrow input repeats continuously while held and stops immediately on release
12. Dismiss the keyboard or background the app while holding an accessory arrow
13. Expected: repeat stops and does not resume when the keyboard or app returns
14. Open floating file search or the git commit message input while a terminal is visible behind it
15. Expected: the software keyboard appears without the terminal accessory bar, typing goes to the focused input, and the visible terminal does not resize or shift for that keyboard
16. Tap the already-focused terminal while the keyboard is visible
17. Expected: the software keyboard dismisses and the next terminal tap reopens it
18. With terminal output filling the screen, tap the terminal so the keyboard appears
19. Expected: the bottom terminal content lifts above the keyboard and accessory bar instead of staying hidden behind them

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
4. Expected: recognized text appears in the native dictation preview while the terminal output stays stable during live hypothesis rewrites
5. Stop dictation
6. Expected: the dictated text commits to the PTY once when dictation finalizes, without being removed, without a stuck marked-text underline, without duplicate characters, and without a `UIDictationController` hypothesis-cancel log
7. Repeat with a longer phrase and stop immediately after the last words
8. Expected: the final committed PTY text includes the last words, with no `UIDictationController` hypothesis-cancel log
9. While preview text is visible, tap the preview bubble
10. Expected: the preview dismisses immediately and does not stay stuck above the keyboard
11. Repeat with a dictated phrase that includes a newline or return command
12. Expected: newline input is routed as terminal enter rather than leaving literal marked text behind
13. Start dictation again, then cancel or force recognition failure by stopping before speech is recognized
14. Expected: the terminal remains focused, no stale dictation text is committed, and normal keyboard typing still reaches the PTY

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
17. Expected: the PTY receives `lê`, `chà`, `tối`, `với`, `đúng`, and `ư `, without duplicate base consonants such as `llê` or `chhà`, without placing the tone on the final vowel as `tôí`, without duplicating a replayed composed cluster as `vơới`, without dropping preserved prefixes such as `đ`, without dropping the standalone composed character before the space, and without showing the dictation preview while typing or backspacing
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
8. Expected: streamed text updates the preview first, commits to the PTY once at finalization, late native reconciliation does not duplicate or delete PTY text, and there is no `UIDictationController` hypothesis-cancel log
9. Terminal, native dictation stream: dictate `hello, how's it going` and watch the first `insertText` word plus following `replaceRange` rewrites
10. Expected: the first word and following rewrites update the preview while the synthetic marked range remains available for UIKit reconciliation; the final transcript commits once, with no hypothesis-cancel log
11. Terminal, native dictation preview: tap the visible preview bubble before finalization
12. Expected: the preview dismisses and the terminal does not keep a stuck streamed preview state
13. Cross-flow: after a dictation commit, press backspace repeatedly, then type `hel` and accept `hello`
14. Expected: dictation cleanup does not delete already-committed terminal text, repeated backspace continues through the PTY, and the following suggestion starts from a fresh keyboard context
15. Cross-flow: start a Japanese marked composition, cancel it, then accept a native suggestion in the same terminal focus session
16. Expected: cancelled marked text does not poison the following suggestion replacement or leave stale candidate UI
17. Commit message input: type Japanese marked text, accept a candidate, then accept a native suggestion
18. Expected: normal input uses the same native IME protocol correctly, with marked text committed once and suggestions replacing only the requested range
19. Commit message input, dictation: tap the microphone, dictate a short phrase, then stop dictation
20. Expected: the dictated phrase remains in the input after UIKit commits, the final cleanup delete does not clear the field, and any late `insertDictationResult` does not duplicate the phrase

## 11e. Terminal Keyboard Accessory Arrow Repeat On iOS

1. Connect to a session on iPhone or iOS simulator and open the terminal view
2. Tap each arrow button in the keyboard accessory once
3. Expected: each tap sends exactly one corresponding arrow keystroke
4. Press and hold each arrow button, then release it
5. Expected: the corresponding arrow input repeats continuously while held and stops immediately on release
6. Start holding an arrow button, then dismiss the keyboard or background the app
7. Expected: repeat stops and does not resume when the keyboard or app returns
8. Open floating file search or the git commit message input while a terminal is visible behind it
9. Expected: the software keyboard appears without the terminal accessory bar, and the visible terminal does not resize or shift for that keyboard

## 11f. Terminal Keyboard Accessory After Reconnect On iOS

1. Connect to a session on iPhone or iOS simulator and open a terminal
2. Tap the terminal so the software keyboard and accessory bar are visible
3. Press `Tab`, `Enter`, and an arrow key in the accessory bar
4. Expected: each key reaches the PTY
5. Put the workspace through an idle/reconnect recovery by backgrounding the app, sleeping/waking the host, or briefly interrupting network until the badge returns to `Connected`
6. Keep the same terminal active, show the software keyboard again, then press `Tab`, `Enter`, and an arrow key in the accessory bar
7. Expected: each key still reaches the PTY after reconnect; the accessory bar does not keep sending to a stale terminal channel

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
12. Expected: long filenames in the git panel file list truncate with an ellipsis, while status marks and change counts remain visible
13. Tap the untracked file entry
14. Expected: the diff view shows the untracked file content as added lines
15. Long-press a file entry
16. Expected: the file action sheet opens for that entry instead of doing nothing
17. Tap the dimmed backdrop outside the action sheet
18. Expected: the native action sheet dismisses without staging or unstaging the file

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

## 15c. Markdown Mermaid Diagram In Preview

1. Connect to a session and open the terminal view
2. Run:

```bash
printf '%s\n' '# Mermaid Test' '' '```mermaid' 'flowchart LR' '  A[Start] --> B[End]' '```' > /tmp/zedra-markdown-mermaid.md
printf '/tmp/zedra-markdown-mermaid.md:1\n'
```

3. Tap `/tmp/zedra-markdown-mermaid.md:1`
4. Expected: the preview opens in markdown mode
5. Expected: a rendered flowchart appears in a card at intrinsic SVG scale (not shrunk to viewport width)
6. Expected: wide diagrams scroll horizontally inside the card, like markdown tables
7. Expected: invalid mermaid syntax falls back to monospace source with a muted error line
8. Tap `Show source` below a rendered diagram
9. Expected: the fenced mermaid source appears and remains selectable for Add to Chat
10. Open the same file from the workspace docs tree or editor
11. Expected: the main workspace markdown view renders the diagram the same way
12. Open `examples/markdown-mermaid/diagrams.md` (or copy its ER and pie sections to `/tmp`)
13. Expected: diagram canvas, nodes, and ER entity boxes use dark fills with visible borders (not white boxes on a dark card)
14. Expected: ER attribute rows, pie legend labels, and edge relationship labels use light text on dark surfaces
15. Expected: flowchart connectors and arrowheads use accent blue (`#61afef`), not dim gray, with mostly straight vertical/horizontal segments

## 15d. Markdown Bottom Padding And Link Hit Slop

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

## 15e. Markdown Frontmatter In Preview

1. Connect to a session and open the terminal view
2. Create a Markdown file with YAML (`---`) frontmatter followed by a heading and body
3. Open the file from a terminal file link and from the workspace docs tree
4. Expected: the preview renders the frontmatter as a metadata block above the heading, with a narrow key column
5. Use `examples/frontmatter.md` (nested `owner`, `status`, list values)
6. Expected: nested objects/lists render their key on its own line with the value indented below; leaf `key: value` pairs render inline without a reserved column
7. Expected: plain text values (including the folded `description`) wrap within the available width; only unbreakable content (URLs) overflows and scrolls the metadata block horizontally like a Markdown table
8. Select body text and tap `Add to Chat`
9. Expected: the attached source line range points to the original lines after the frontmatter

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
16. Restart an agent CLI, open the agent detail view, and tap a config/memory file to open it in the native file-preview sheet
17. Select text in the preview sheet and tap `Add to Chat`, then pick an agent terminal
18. Expected: the agent-target picker appears and the selected range is pasted into the chosen terminal, the same as an editor selection — the agent-detail preview routes through the foreground workspace, not just terminal-link previews

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

## 18b. Agent Toolbar, Sessions, and Manage Views

1. Open a connected workspace and open the workspace drawer Terminals tab.
2. Expected: the top toolbar shows four icon actions: Create agent, Create
   terminal, View sessions, and Manage agents.
3. Tap `Create terminal`.
4. Expected: the drawer closes, a new shell terminal opens in the main view,
   and the terminal card appears in the drawer.
5. Tap `Create agent`.
6. Expected: a scrollable native list picker shows installed host agents with
   icon and version subtitle; unavailable agents are omitted.
7. Pick an installed agent such as Claude or Codex.
8. Expected: Zedra creates a new terminal using the host-owned launch command,
   opens it as the active main view, and the terminal card shows the matching
   agent icon once OSC metadata arrives.
9. Tap `View sessions`.
10. Expected: the drawer closes and the main view shows a unified session list
    grouped by day across Claude, Codex, OpenCode, Pi, and Hermes for the current
    workspace. Each row shows agent icon, title, datetime, branch/worktree,
    transcript size, and model when available. Hermes is a global agent, so its
    sessions appear in every workspace (not filtered by `cwd`).
11. Tap a resumable session row.
12. Expected: Zedra immediately resumes the session in a new terminal and opens
    it as the active main view.
13. Tap `Manage agents`.
14. Expected: the main view shows managed agents as a list with setup/session
    counts and usage gauges when live usage is available. Tapping an agent opens
    detail with metadata, usage gauges (5h / 7d and extra spend when present),
    account fields, and a session list below.
15. In manage detail, verify discovered account fields:
    - Claude: logged in, plan (Pro/Max/Team/Enterprise when OAuth, credentials
      file, or CLI PTY is available), model, effort, permission mode, today msgs
      and total cost when `stats-cache.json` exists; no week msgs/sessions/tools
      rows. Usage gauges: with a valid `~/.claude/.credentials.json` token,
      limits come from the OAuth usage API; with Keychain-only CLI auth or an
      expired file token while the CLI still works, from PTY `/usage` scrape
      (percents should match the CLI panel; inline reset duration may be missing
      if parsing fails). Host check: `zedra agent scan usage --json`.
    - Codex: logged in, plan, plan until, account name from `auth.json`, model/
      personality from `config.toml`
    - OpenCode: config dir presence
    - Pi: logged in (sessions dir presence). Install with `npm install -g
      @earendil-works/pi-coding-agent`; sessions live at
      `~/.pi/agent/sessions/--<workdir>--/<timestamp>_<uuid>.jsonl`. Resume
      should run `pi --session <id>` in a new terminal.
    - Hermes: per-provider auth + active provider (`$HERMES_HOME/auth.json`,
      default `~/.hermes`), Default model / Default provider (`config.yaml`),
      Skills count, Total spend + Platforms (`state.db` rollups). Sessions are
      global, read from `$HERMES_HOME/state.db` (curated titles, platform
      `source`, tool counts, per-session cost; falls back to `sessions/*.json`
      when the db is absent); resume should run `hermes --resume <session_id>`.
      Manage detail shows a **read-only "Config & memory"** section listing
      `SOUL.md`, `USER.md`, `MEMORY.md`, `config.yaml`, `.env`, `cron/jobs.json`.
      Tapping a present file opens its content in the **same native preview
      sheet as terminal file links** (`FilePreviewView`): `.md` files render as
      formatted markdown, others as a syntax-highlighted code editor. Absent
      files show "not created yet" and are not tappable; oversized files show
      "truncated" in the subtitle. The client cannot edit these.
16. Tap `Resume` on a session from manage detail.
17. Expected: same immediate resume behavior as the unified sessions view.
18. Re-open `View sessions` or `Manage agents` without tapping Refresh.
19. Expected: lists load quickly from the host startup cache (default limit 50
    sessions per agent).
20. Tap Refresh in either view.
21. Expected: the host rescans synchronous managed-agent metadata and session
    lists immediately; CLI versions may arrive a moment later (one
    `AgentInfoChanged` host event per agent) and update manage detail without
    another refresh.
22. In manage detail for an available agent, wait a few seconds after opening
    or refreshing.
23. Expected: the version line updates from `Checking…` to the real `--version`
    string when the host finishes that agent’s async version probe.
24. Open manage detail for an agent with zero sessions or a session-list error
    (for example Codex when the host scan fails).
25. Expected: the header (back, title, refresh) stays full content width and
    lines up with the metadata column; narrow error text must not shrink-wrap
    the header (see `docs/CONVENTIONS.md` GPUI flex width rules and
    `ui::subscreen_layout`).

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

## 21. Task Live Activity (local MVP)

Requires a physical device on iOS 16.2+ (Dynamic Island needs iPhone 14 Pro or
later). Confirm `Settings > Zedra > Live Activities` is on. Trigger with deeplinks
typed in Safari or via `xcrun devicectl device open` / a notes link.

Deeplinks are `zedra://la-test/<action>/<taskId>`, where `<action>` is
`start|needs|done|end` and `<taskId>` binds the action to a specific task (a terminal
id in production). The trailing shows the aggregate `done/total` rollup.

1. Open `zedra://la-test/start/term-1`, then `zedra://la-test/start/term-2`
2. Expected: a Live Activity appears; Dynamic Island leading shows the Zedra glyph,
   trailing shows `0/2`; lock screen headline reads "0/2 tasks done"
3. Open `zedra://la-test/done/term-1`
4. Expected: trailing updates to `1/2`
5. Open `zedra://la-test/needs/term-2`
6. Expected: trailing changes to an orange `!` (needs-attention outranks the count);
   headline reads "Waiting for you"
7. Open `zedra://la-test/done/term-2`
8. Expected: all tasks done — trailing changes to a green checkmark; headline reads
   "All done · 2/2"
9. Open `zedra://la-test/end/term-1`, then `zedra://la-test/end/term-2`
10. Expected: the activity stays while one task remains, then is removed when the last
    task is ended (equivalently, `zedra://la-test/end` with no id clears it at once)
11. Toggle `Settings > Zedra > Live Activities` off, then open `zedra://la-test/start/x`
12. Expected: no activity appears; device log shows `[LA] disabled in Settings`
## 21. iOS Release logging

1. Install a **Release** build on device (not Xcode Run with debugger attached)
2. Connect via **Scan QR**; optional: `scripts/log-ios.sh --grep connect`
3. Expected: no burst of per-packet iroh/quinn trace lines (`tracing-subscriber` requires `debug-logs`)

## 22. Terminal appearance (light/dark)

1. Connect to a workspace and open a terminal running `ls` with color
2. Open Settings → Appearance and tap the **sun** segment (light mode)
3. Expected: terminal background is light; directory names and ANSI colors are readable (not washed out)
4. Run `claude` (or another session already showing Claude output) and scroll to file-reference lines such as `L123 (file.rs):`
5. Expected: highlighted paths are readable on the light background (not pale lavender)
6. Open **Codex** in the same terminal (or a dedicated Codex session)
7. Expected: the composer / user-message background pill matches the light theme (not missing or using a dark gray from stale palette)
8. From the agent picker or quick action panel, open a fresh Codex launch-command terminal
9. Expected: Codex's first rendered composer / user-message background pill is visible without waiting for terminal reattach or focus changes
10. Toggle back to **Dark**
11. Expected: terminal colors, Claude highlights, and Codex pill update without restarting the app
12. Optional: from the host, run `printf '\e[10;?\e[11;?\e\\'` inside the Zedra session and confirm replies report the current fg/bg (light: `fafafa` background). Zedra answers OSC queries on the session PTY via `ColorRequest` and the active `TerminalTheme`; it does not inject palette setup bytes into scrollback on toggle.

## 22a. Android Native Presentations Follow Theme

1. Install an Android build and open Settings.
2. Toggle Appearance to **Light**.
3. Open the agent picker, keyboard accessory bar, and a native text input or alert.
4. Expected: native surfaces use light backgrounds and dark text/icons, matching the app theme.
5. Toggle Appearance back to **Dark** and repeat the same presentations.
6. Expected: native surfaces return to dark backgrounds and light text/icons without restarting the app.

## 22b. iOS Native Presentations Follow Theme

1. Install an iOS build and open Settings.
2. Toggle Appearance to **Light**.
3. Open an alert, action sheet, agent picker, keyboard accessory bar, native text input, custom sheet, and native notification.
4. Expected: UIKit presentation chrome uses light backgrounds and dark text/icons, matching the app theme.
5. Toggle Appearance back to **Dark** and repeat the same presentations.
6. Expected: UIKit presentation chrome returns to dark backgrounds and light text/icons without restarting the app.

## 23. Agent Hook Notification Sound + Haptic (iOS and Android)

1. Connect to a workspace and start an agent (for example Claude) in a terminal, keeping the app in the foreground.
2. Send a prompt and wait for the agent to finish its turn (a `Stop` hook event).
3. Expected: the device plays a notification sound and fires a short haptic at the same time.
4. Background the app and trigger another hook event.
5. Expected: no in-app sound or haptic fires while backgrounded.

## Floating File Search (cmd+P)

1. Connect to a workspace and open the drawer on the **Files** tab.
2. Tap the search (magnifier) icon in the top-right control cluster, above the
   Explorer / Docs toggle.
3. Verify a floating panel appears near the top over a dimmed full-screen
   backdrop, the keyboard opens, and the input is focused.
4. Type a query: results stream in below the input (name + relative path rows).
   Confirm "Searching…" shows briefly, then matches; clear the query to return
   to the "Type to search files" prompt. Matched characters in the relative path
   are emphasized exactly where the host scored them.
5. Tap a file result: the panel closes and the file opens in the editor.
6. Reopen the popup and tap the dim backdrop (or use system back / swipe back):
   the popup closes without opening anything.
7. Immediately tap the search icon again (without touching the drawer): the popup
   reopens. Repeat the dismiss/reopen cycle a few times to confirm it never gets
   stuck closed.
8. Confirm the old inline search bar no longer appears at the top of the file
   explorer list.

### Worktree results

1. In the workspace repo on the host, create a linked worktree under a
   gitignored directory:

   ```sh
   mkdir -p .claude/worktrees
   git worktree add .claude/worktrees/wt1
   ```

2. Search for a file that exists in the repo (so both the main checkout and the
   worktree contain it, e.g. `README.md`).
3. Expected: the worktree copy appears once with its path relative to the
   worktree, plus a third muted line under the path showing a branch icon and
   the worktree's branch name (`wt1` for the command above); the main-checkout
   copy has no branch line and stays two lines tall. Long names, paths, and
   branch labels truncate to one line each.
4. Search for a file that exists only in the worktree and confirm it is found
   even though `.claude/` is gitignored; tapping it opens the worktree file.
5. Clean up with `git worktree remove .claude/worktrees/wt1`.

## Delta Sign-In and Notification Channel (iOS and Android)

### Google sign-in (Android)
1. On a fresh install, open the Delta sign-in sheet and choose **Continue with Google**.
2. Pick a Google account in the system credential dialog.
3. Expected: sign-in completes (the ID token reaches `nativeDeltaGoogleSignInResult`);
   no "Unexpected credential type" error. Previously every attempt fell through to the
   error branch because the credential is a `CustomCredential`.

### Apple sign-in (iOS)
1. Open the Delta sign-in sheet and choose **Continue with Apple**.
2. Cancel the system sheet, then immediately start Apple sign-in again and complete it.
3. Expected: the second flow completes and delivers its callback; cancelling the first
   does not orphan or cross-wire the second (coordinators are now keyed per `callbackID`).

### Push-token request overlap (iOS)
1. Trigger push-token registration twice in quick succession (e.g. rapid re-entry of the
   notifications prompt).
2. Expected: the second request fails fast with "Push token request already in progress"
   rather than silently orphaning the first callback.

### FCM notification before first launch (Android)
1. Force-stop the app so `MainActivity.onCreate` has not run since install/clear-data.
2. Send a Delta push (or use the backend test hook) while the app is not running.
3. Expected: the notification appears. In logcat, confirm `ZedraMessagingService` creates the
   `zedra_delta` channel before `notify(...)`. Without this, the first notification was dropped
   on Android O+ because the channel did not exist yet.

### Google action icon inset (iOS)
1. Present a native selection/list picker whose Google action image is passed as `"Google.svg"`
   or a path-prefixed name.
2. Expected: the Google glyph keeps its 2pt inset (the inset lookup now uses the normalized name).

## Water Droplet Effect (iOS)

### Toggle and basic refraction
1. Open **Settings → Appearance** and switch **Water droplet** to **On**.
2. Expected: a droplet appears near the top left, magnifying and tinting the UI
   beneath it with a specular highlight.
3. Switch the toggle to **Off**. Expected: the droplet disappears immediately and
   rendering behaves exactly as before the toggle existed.

### Drag and shape
1. With the droplet on, drag it slowly over terminal or editor text.
2. Expected: text under the droplet appears refracted (bent toward the center),
   slightly magnified and cool-tinted; the droplet lags the finger like liquid.
3. Flick the droplet fast across the screen.
4. Expected: the shape stretches along the motion and shows a trailing drip blob;
   on release it springs to rest and the wobble settles (no permanent animation
   loop — frame updates stop once it rests).

### Touch suppression
1. Rest the droplet on top of a button or terminal, then tap and drag it.
2. Expected: the UI underneath never reacts — no button press, no terminal
   focus, no scroll — while the finger interacts with the droplet. Content
   under a resting droplet is reachable again after dragging the droplet aside.

### Rotation and resize
1. With the droplet on, rotate the device.
2. Expected: no crash, no stale grab artifacts; the droplet keeps rendering at its
   logical position and refraction stays sharp after the size change.

## Clipboard Sync (host <-> iOS, text v1)

Requires a paired host daemon with a reachable system clipboard (a normal macOS/Linux desktop session, not a headless server).

### Host to device (auto)
1. Open Settings and turn **Clipboard sync** on (it defaults off).
2. On the host machine, copy some text with the OS clipboard tool (`pbcopy` on macOS, `xclip`/`wl-copy` on Linux) or select+copy.
3. With the app foreground, expected: within ~1s the device pasteboard holds that text, paste into any iOS text field to confirm.
4. Turn Clipboard sync **off**, copy new text on the host.
5. Expected: the device pasteboard does NOT change (no auto-write when off).

### Device to host (manual)
1. Copy text on the device.
2. Tap **Send clipboard**.
3. Expected: the host system clipboard now holds that text (verify with the OS clipboard tool, e.g. `pbpaste`/`xclip -o`/`wl-paste`, or paste into another app).
4. Expected: the send does not bounce back and overwrite the device pasteboard.

### Degraded host
1. Point the app at a headless host with no display server / clipboard.
2. Expected: no crash; auto-sync is silently inactive and Send clipboard surfaces a clear error rather than hanging.
