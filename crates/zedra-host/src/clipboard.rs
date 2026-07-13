//! Host system-clipboard bridge.
//!
//! A single dedicated OS thread owns the one `arboard::Clipboard` for the whole
//! daemon and serves every clipboard operation: the periodic poll, `ClipboardGet`
//! reads, and `ClipboardSet` writes. This matters for correctness, not just tidiness:
//! on X11 a `Clipboard` must stay alive to keep serving paste requests, so a
//! per-call throwaway instance would lose a written value moments later. Routing
//! reads and writes through the same thread also serializes them with the poll, so
//! there is no read/write interleave race on the dedup hash.
//!
//! Tradeoff: arboard calls are synchronous and unbounded. Because one thread now
//! serves reads, writes, and the poll, a wedged OS clipboard call (notably an X11
//! selection owner that stops responding) blocks all clipboard ops until the daemon
//! restarts, whereas a per-call throwaway instance would scope a hang to that call.
//! Accepted for v1 (macOS is the primary host, where NSPasteboard does not exhibit
//! this); revisit with a watchdog/timeout if it bites on X11.
//!
//! Send-path dedup: after the thread writes a value (from `ClipboardSet`) it records
//! that value's hash, so the next poll tick recognizes it and does not echo it back
//! to clients as a `ClipboardChanged`. This is an optimization, not a correctness
//! wall: clients never auto-send, so no echo loop can form.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;
use zedra_rpc::proto::{ClipboardContent, ClipboardPayload, HostEvent, CLIPBOARD_MAX_BYTES};

use crate::session_registry::SessionRegistry;

/// How often the host system clipboard is polled. arboard exposes no
/// cross-platform change notification, so we poll. Tune here if it matters.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// A request handed to the clipboard thread. Each carries a channel for its reply.
enum ClipboardCommand {
    Read(oneshot::Sender<Result<Option<ClipboardContent>, String>>),
    Write(ClipboardContent, oneshot::Sender<Result<(), String>>),
}

/// Shared handle to the clipboard thread plus the poll/write dedup hash.
#[derive(Default)]
pub struct ClipboardSync {
    /// Hash of the last value observed or written by the host. `None` until the
    /// first observation.
    last_hash: Mutex<Option<u64>>,
    /// Command channel into the clipboard thread. `None` until `spawn` runs, or
    /// permanently `None` if the host has no reachable clipboard.
    commands: Mutex<Option<mpsc::Sender<ClipboardCommand>>>,
}

impl ClipboardSync {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record `hash` and report whether it is a change (so the poller broadcasts).
    fn observe(&self, hash: u64) -> bool {
        let mut guard = self.last_hash.lock().expect("clipboard hash mutex");
        if *guard == Some(hash) {
            return false;
        }
        *guard = Some(hash);
        true
    }

    fn note_written(&self, hash: u64) {
        *self.last_hash.lock().expect("clipboard hash mutex") = Some(hash);
    }

    /// Forget the last value so a later identical copy is seen as a fresh change,
    /// not suppressed as an echo. Called when the clipboard goes empty/non-text.
    fn clear(&self) {
        *self.last_hash.lock().expect("clipboard hash mutex") = None;
    }

    fn sender(&self) -> Option<mpsc::Sender<ClipboardCommand>> {
        self.commands
            .lock()
            .expect("clipboard commands mutex")
            .clone()
    }

    /// Read the host system clipboard (backs `ClipboardGet`). `Ok(None)` when
    /// empty/oversized; `Err` when the clipboard is unavailable.
    pub async fn read(&self) -> Result<Option<ClipboardContent>, String> {
        let Some(tx) = self.sender() else {
            return Err("clipboard is unavailable on this host".to_string());
        };
        let (respond, rx) = oneshot::channel();
        tx.send(ClipboardCommand::Read(respond))
            .map_err(|_| "clipboard thread stopped".to_string())?;
        rx.await
            .map_err(|_| "clipboard thread dropped reply".to_string())?
    }

    /// Write to the host system clipboard (backs `ClipboardSet`).
    pub async fn write(&self, content: ClipboardContent) -> Result<(), String> {
        let Some(tx) = self.sender() else {
            return Err("clipboard is unavailable on this host".to_string());
        };
        let (respond, rx) = oneshot::channel();
        tx.send(ClipboardCommand::Write(content, respond))
            .map_err(|_| "clipboard thread stopped".to_string())?;
        rx.await
            .map_err(|_| "clipboard thread dropped reply".to_string())?
    }
}

/// Stable content hash for change detection (non-crypto; dedup only).
fn hash_content(content: &ClipboardContent) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match content {
        ClipboardContent::Text(text) => {
            0u8.hash(&mut hasher);
            text.hash(&mut hasher);
        }
        ClipboardContent::Image { format, bytes } => {
            1u8.hash(&mut hasher);
            (*format as u8).hash(&mut hasher);
            bytes.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Read text off a live clipboard, applying the size cap. Text only in v1.
fn read_text(clipboard: &mut arboard::Clipboard) -> Result<Option<ClipboardContent>, String> {
    match clipboard.get_text() {
        Ok(text) if text.is_empty() || text.len() > CLIPBOARD_MAX_BYTES => Ok(None),
        Ok(text) => Ok(Some(ClipboardContent::Text(text))),
        // An empty or non-text clipboard is nothing to sync, not an error.
        Err(_) => Ok(None),
    }
}

/// Write to a live clipboard. Text only in v1.
fn write_content(
    clipboard: &mut arboard::Clipboard,
    content: &ClipboardContent,
) -> Result<(), String> {
    let ClipboardContent::Text(text) = content else {
        return Err("image clipboard is not supported yet".to_string());
    };
    if text.len() > CLIPBOARD_MAX_BYTES {
        return Err("clipboard content exceeds the size limit".to_string());
    }
    clipboard.set_text(text.clone()).map_err(|e| e.to_string())
}

fn is_sync_disabled(value: Option<&str>) -> bool {
    matches!(value, Some("0") | Some("false") | Some("off") | Some("no"))
}

/// Host-side kill switch. The client toggle only gates device-side *apply*; this
/// is the host's own opt-out from *capturing* and broadcasting its clipboard,
/// for a shared or privacy-sensitive host. Default on; unset means enabled.
fn clipboard_sync_disabled_by_env() -> bool {
    is_sync_disabled(std::env::var("ZEDRA_CLIPBOARD_SYNC").ok().as_deref())
}

/// Start the host clipboard watcher: one thread owning the clipboard plus an async
/// broadcaster. If the host disabled sync (`ZEDRA_CLIPBOARD_SYNC=0`) or the system
/// clipboard is unavailable, logs once and leaves `sync` without a command sender,
/// so reads/writes return an error and no poll runs (sync disabled on this host).
pub fn spawn(registry: Arc<SessionRegistry>, sync: Arc<ClipboardSync>) {
    if clipboard_sync_disabled_by_env() {
        tracing::info!("clipboard sync disabled by ZEDRA_CLIPBOARD_SYNC");
        return;
    }
    let (cmd_tx, cmd_rx) = mpsc::channel::<ClipboardCommand>();
    let (change_tx, mut change_rx) = tokio::sync::mpsc::unbounded_channel::<ClipboardContent>();

    let thread_sync = sync.clone();
    let spawn_result = std::thread::Builder::new()
        .name("clipboard-watcher".to_string())
        .spawn(move || {
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    tracing::info!("clipboard sync disabled: {e}");
                    return;
                }
            };
            loop {
                match cmd_rx.recv_timeout(POLL_INTERVAL) {
                    Ok(ClipboardCommand::Read(respond)) => {
                        let _ = respond.send(read_text(&mut clipboard));
                    }
                    Ok(ClipboardCommand::Write(content, respond)) => {
                        let result = write_content(&mut clipboard, &content);
                        // Record only a successful write, so a failed write can't
                        // make the poller swallow a later identical host-local copy.
                        if result.is_ok() {
                            thread_sync.note_written(hash_content(&content));
                        }
                        let _ = respond.send(result);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // First tick after startup broadcasts the pre-existing
                        // clipboard (last_hash is None): a deliberate seed for
                        // already-connected clients, not a stale-data echo.
                        match read_text(&mut clipboard) {
                            Ok(Some(content)) => {
                                if thread_sync.observe(hash_content(&content))
                                    && change_tx.send(content).is_err()
                                {
                                    return; // broadcaster gone; daemon shutting down
                                }
                            }
                            // Emptied / non-text / oversized: forget the last hash so a
                            // later re-copy of the same text reads as a change, not an echo.
                            Ok(None) => thread_sync.clear(),
                            Err(_) => {}
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    // A watcher that can't be spawned degrades like a missing clipboard: no
    // sender is installed, so reads/writes return an error instead of panicking
    // the daemon. The Ok(JoinHandle) is intentionally dropped (detached); the
    // thread lives for the daemon's lifetime.
    if let Err(e) = spawn_result {
        tracing::warn!("clipboard sync disabled: cannot spawn watcher thread: {e}");
        return;
    }

    *sync.commands.lock().expect("clipboard commands mutex") = Some(cmd_tx);

    // Async broadcaster fans each change out to every subscribed session.
    tokio::spawn(async move {
        while let Some(content) = change_rx.recv().await {
            registry
                .broadcast_event(HostEvent::ClipboardChanged(ClipboardPayload { content }))
                .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ClipboardContent {
        ClipboardContent::Text(s.to_string())
    }

    #[test]
    fn hash_distinguishes_content() {
        assert_eq!(hash_content(&text("a")), hash_content(&text("a")));
        assert_ne!(hash_content(&text("a")), hash_content(&text("b")));
    }

    #[test]
    fn observe_fires_only_on_change() {
        let sync = ClipboardSync::new();
        assert!(sync.observe(hash_content(&text("first"))));
        assert!(!sync.observe(hash_content(&text("first"))));
        assert!(sync.observe(hash_content(&text("second"))));
    }

    #[test]
    fn note_written_suppresses_echo() {
        let sync = ClipboardSync::new();
        // A client->host set records the hash; the next poll of the same value
        // must not re-broadcast it back to clients.
        sync.note_written(hash_content(&text("from client")));
        assert!(!sync.observe(hash_content(&text("from client"))));
    }

    #[test]
    fn clear_lets_identical_copy_rebroadcast() {
        let sync = ClipboardSync::new();
        assert!(sync.observe(hash_content(&text("keep"))));
        // Clipboard goes empty / non-text -> dedup reset.
        sync.clear();
        // The same text copied again is a real change now, not a suppressed echo.
        assert!(sync.observe(hash_content(&text("keep"))));
    }

    #[test]
    fn re_copy_after_other_value_rebroadcasts() {
        // A client push records X; the echo is suppressed, but a genuine host-local
        // re-copy of X after copying Y is a real change and must broadcast again.
        let sync = ClipboardSync::new();
        sync.note_written(hash_content(&text("X")));
        assert!(!sync.observe(hash_content(&text("X"))));
        assert!(sync.observe(hash_content(&text("Y"))));
        assert!(sync.observe(hash_content(&text("X"))));
    }

    #[test]
    fn env_kill_switch_parsing() {
        assert!(is_sync_disabled(Some("0")));
        assert!(is_sync_disabled(Some("false")));
        assert!(is_sync_disabled(Some("off")));
        assert!(!is_sync_disabled(None)); // default: enabled
        assert!(!is_sync_disabled(Some("1")));
    }

    #[tokio::test]
    async fn degraded_host_read_write_error_cleanly() {
        // No spawned thread => no clipboard on this host. Ops must error, not hang
        // or panic, so a client just sees a failed ClipboardGet/Set.
        let sync = ClipboardSync::new();
        assert!(sync.sender().is_none());
        assert!(sync.read().await.is_err());
        assert!(sync.write(text("hi")).await.is_err());
    }
}
