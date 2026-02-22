Capture and analyze performance diagnostics from the running Android app.

Steps:
1. Clear logcat and start background capture with `adb logcat -c && adb logcat > /tmp/zedra-perf-capture.log &`
2. Tell the user: "Logcat capture started. Interact with the app on your device — navigate screens, open files, scroll, use the terminal, open/close the drawer. Say **done** when finished."
3. Wait for the user to say "done"
4. Stop the background logcat capture
5. Run `./scripts/perf-debug.sh` with the captured log, or run it directly (it handles its own capture)
6. Read the output and present a categorized summary:
   - **Renderer**: frame timing stats from `[gpui_wgpu]` lines
   - **Navigation**: screen transitions, file loads
   - **Editor**: cache rebuild frequency and size
   - **Terminal**: data processing events
   - **Touch**: fling start/end counts, drawer snap events
   - **Transport**: connection type changes, reconnect attempts
   - **File Explorer**: entry count changes
   - **Errors**: any panics, crashes, or ANRs
   - **Memory**: RSS from dumpsys
7. Highlight any anomalies (frame times >16ms, excessive cache rebuilds, errors)
