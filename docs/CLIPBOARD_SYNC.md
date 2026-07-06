# Zedra Clipboard Sync

System-clipboard bridge between the host daemon and the mobile client over the existing iroh RPC tunnel. v1 is iOS + text. Images are reserved in the wire type but not implemented until a `gpui_ios` image-clipboard spike lands (see v1.1).

## Architecture

```
                 macOS / Linux system clipboard
                          ^          |
                  arboard |          | arboard          one dedicated thread owns
                          |          v                  the arboard handle and
        +-----------------+----------------------+      serves poll + Get + Set
        |  HOST daemon (zedra-host)              |      (serialized => race-free
        |    clipboard::spawn thread             |       dedup, X11-safe writes)
        |    - poll 1s -> hash -> ClipboardSync  |
        |    - ClipboardGet / ClipboardSet arms  |
        +-----------------+----------------------+
                          |  iroh QUIC tunnel (e2e encrypted)
                          |    HostEvent::ClipboardChanged  (host -> client, stream)
                          |    ClipboardSet / ClipboardGet  (client -> host, unary)
        +-----------------+----------------------+
        |  iOS client (zedra, GPUI)             |
        |    host-event loop -> auto-apply       |
        |    "Send clipboard" action             |
        +-----------------+----------------------+
                          ^          |
             UIPasteboard |          | UIPasteboard     via gpui_ios
                          |          v                  cx.read/write_to_clipboard
                     iOS device pasteboard
```

**Host -> device (automatic):**

```
host clipboard   watcher (poll 1s)         iOS app              device pasteboard
   copy "X" ------->  hash != last_hash
                      broadcast ==HostEvent::ClipboardChanged("X")==>  if foreground
                                                                       && sync ON:
                                                                       write "X" -------> "X"
```

**Device -> host (manual) + loop suppression:**

```
device pasteboard   "Send clipboard"        host dispatch            watcher
   "Y" ----read---->  ClipboardSet("Y") ==>  note_written(hash "Y")
                                             arboard.set_text("Y")
                                             (next poll: hash == last_hash => NO echo back)
```

## Interaction model

Two directions, deliberately asymmetric because iOS forbids background pasteboard reads:

- **Host to client (automatic).** The host watches its own system clipboard and, on change, pushes the latest value to subscribed clients as a `HostEvent`. When the client app is foreground and the "Clipboard sync" setting is on, it writes the value into the device pasteboard.
- **Client to host (manual).** A "Send clipboard" action reads the device pasteboard and issues a `ClipboardSet` RPC. The host writes it to its system clipboard.

Single latest slot, last-write-wins. No history, no snippets.

## Scope

- **v1:** text only, iOS only. Auto host to client, manual client to host, OFF-by-default settings toggle, 2 MiB payload cap, content-hash loop suppression.
- **v1.1 (deferred):** images (PNG/JPEG). The `ClipboardContent::Image` wire variant ships in v1 unused so v1.1 is purely additive, but image capture and apply are gated on verifying `gpui_ios` `ClipboardItem` image support (spike first; FFI `UIPasteboard.image` fallback if unsupported). Android is out of scope for both.

## Wire protocol

Postcard is positional, so these are appended at the tail of `ZedraProto` and `HostEvent` (never inserted/reordered). Following the repo's compatibility model (the live `zedra/rpc/4` line rolls forward with host and client shipping together; the frozen `proto_v3.rs` is the cross-version boundary), `zedra/rpc/4` is not bumped, matching the 2026-07-05 `FsSearch` precedent. The new v4-only request variants need no `proto_v3` work; `HostEvent::ClipboardChanged` gets a `host_event_v3` filter arm so v3 clients never receive it.

Caveat, not a general decode-compat guarantee: a v4 client built *before* this variant existed cannot decode a `ClipboardChanged` on its Subscribe stream. Per the rolling-v4 model a host should not out-run its paired clients; if that ever becomes a real rollout hazard, gate the broadcast on a client-capability check rather than relying on lockstep.

Shared payload type (in `crates/zedra-rpc/src/proto.rs`):

```rust
pub enum ClipboardContent {
    Text(String),
    // v1.1, unused in v1; keeps the wire additive.
    Image { format: ClipboardImageFormat, bytes: Vec<u8> }, // serde_bytes
}
```

RPC surface (appended to `ZedraProto`, after `ClearClientDeltaInfo`):

- `ClipboardSet(ClipboardSetReq)` to `oneshot<ClipboardSetResult>`, client to host. `ClipboardSetReq { content: ClipboardContent }`, `ClipboardSetResult { error: Option<String> }` (mirrors `FsWriteResult`).
- `ClipboardGet(ClipboardGetReq)` to `oneshot<ClipboardGetResult>`, seed on connect / explicit pull. `ClipboardGetResult { content: Option<ClipboardContent>, error: Option<String> }`.

Host-initiated event (appended to `HostEvent`, streamed over `Subscribe`):

- `HostEvent::ClipboardChanged(ClipboardPayload)` where `ClipboardPayload { content: ClipboardContent }`.

The 2 MiB cap is enforced before anything crosses the tunnel: the host drops oversized content from `ClipboardChanged` and `ClipboardGet`; the client rejects oversized `ClipboardSet` locally with a user-facing notice.

## Host watcher

`crates/zedra-host` gains an `arboard` dependency (the daemon is headless, so it has no GPUI clipboard). A single watcher per host, not per client, reads the system clipboard, hashes the content, and polls every ~1 s. On a hash change it broadcasts `ClipboardChanged` to all subscribers. `ClipboardSet` writes via `arboard`; `ClipboardGet` reads via `arboard`.

Graceful degradation: if `arboard` fails to initialize (headless host with no display server), clipboard sync is disabled, the watcher does not start and `ClipboardGet`/`ClipboardSet` return an `error` string instead of panicking.

## Loop suppression (correctness-critical)

Without this the feature echoes: client sends -> host writes its clipboard -> watcher sees a "change" -> pushes `ClipboardChanged` back -> client re-applies. The watcher tracks the hash of the last value it wrote or observed. A detected change whose hash equals the last-known hash is ignored and never broadcast. A `ClipboardSet` updates the last-known hash before writing, so the resulting watcher tick is a no-op.

## iOS integration

Clipboard I/O goes through GPUI, not the Swift FFI bridge (which is for keyboard/safe-area/version glue only). The mobile client:

- Client RPC methods `clipboard_set` / `clipboard_get` in `crates/zedra-session/src/handle.rs`, consuming `ClipboardChanged` in the host event handler (`session.rs`).
- On connect, calls `clipboard_get` once to seed the current host value.
- On `ClipboardChanged` (or the seed), if the toggle is on and the app is foreground, applies via `cx.write_to_clipboard(ClipboardItem::new_string(...))`.
- "Send clipboard" reads `cx.read_from_clipboard()`, enforces the cap, and calls `clipboard_set`.

## Settings

One "Clipboard sync" toggle, **default OFF**. It gates the automatic host-to-client pasteboard write (the invasive path: it clobbers whatever the user copied, and clipboards often hold secrets). The manual "Send clipboard" button is an explicit user action and is always available regardless of the toggle.

## Security

Clipboard content is end-to-end encrypted in transit by the existing tunnel. The residual risk is on-device: once auto-written, the iOS pasteboard is readable by other apps. The OFF-by-default toggle is the mitigation; users opt in knowingly.

**Host-side opt-out.** The client toggle gates only device-side *apply*. The host also captures its own clipboard, so it has its own kill switch: set `ZEDRA_CLIPBOARD_SYNC=0` (`false`/`off`/`no`) in the daemon's environment to stop the watcher from starting at all, for a shared or privacy-sensitive host. Default is on. A fuller fix (host only captures when a paired client has sync on, via a capability bit at Connect) is planned for v1.1.

**Broadcast scope (deliberate).** Clipboard is the first host-*global* event: a host clipboard change is delivered to every currently-subscribed session, whereas existing events (`GitChanged`, `FsChanged`, `TerminalAgentChanged`) target one session. This is inherent to "one host clipboard, N paired devices" and is intended. The value only reaches a device's *pasteboard* if that device has the setting on and is foreground; the delivery itself is bounded to live subscribers (no at-rest backlog). In a multi-device pairing, be aware every subscribed device receives the host clipboard.

## Testing

- Postcard roundtrip tests in `zedra-rpc` for every new type (required by `PROTOCOL_SPECS.md` §10).
- Host watcher unit test with a fake clipboard: hash-change detection, cap enforcement, and loop-suppression (a set-then-tick produces no broadcast).
- On-device manual test appended to `docs/MANUAL_TEST.md`: copy on host -> appears on device; send from device -> appears on host; toggle off -> no auto-write.

## Protocol checklist (`PROTOCOL_SPECS.md` §10)

1. `proto.rs`, new variants + structs. 2. `rpc_daemon.rs`, dispatch arms + watcher. 3. `handle.rs`, client methods + event consumption. 4. UI, toggle + send button. 5. Update `PROTOCOL_SPECS.md` §5 (RPC surface) and §11 (changelog). 6. Roundtrip + behavior tests. 7. Note the protocol change in the PR.
