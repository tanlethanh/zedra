// irpc protocol definition for Zedra RPC.
//
// Typed, binary-serialized (postcard) messages over iroh QUIC streams.
//
// Connection lifecycle:
//   First pairing:   Register → Authenticate → AuthProve → (RPC calls)
//   Reconnect:       Authenticate → AuthProve → (RPC calls)
//   Health:          Ping → Pong (every 2s, foreground only)

use irpc::channel::{mpsc, oneshot};
use irpc::rpc_requests;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Protocol enum
// ---------------------------------------------------------------------------

#[rpc_requests(message = ZedraMessage)]
#[derive(Serialize, Deserialize, Debug)]
pub enum ZedraProto {
    // IMPORTANT: APPEND-ONLY ORDER.
    // Postcard encodes enum variants by ordinal index, so inserting/reordering
    // variants can break cross-version RPC compatibility in non-obvious ways.
    // Always append new variants at the end of this enum.
    // -- Auth (pre-session, must come before any RPC) --
    /// First pairing only: register a new client by proving QR possession.
    /// Must be sent before Authenticate on the very first connection.
    #[rpc(tx = oneshot::Sender<RegisterResult>)]
    Register(RegisterReq),

    /// Every connection: request a challenge from the host.
    /// Host generates a nonce, signs it with its iroh key, returns both.
    /// Client must verify host_signature before sending AuthProve.
    #[rpc(tx = oneshot::Sender<AuthChallengeResult>)]
    Authenticate(AuthReq),

    /// Every connection: prove client identity by signing the challenge nonce.
    /// Also specifies which session to attach to.
    #[rpc(tx = oneshot::Sender<AuthProveResult>)]
    AuthProve(AuthProveReq),

    // -- Health / RTT --
    /// Ping the host. Host echoes timestamp_ms back for RTT measurement.
    /// Sent every 2s (foreground only). 5 consecutive misses = reconnect.
    #[rpc(tx = oneshot::Sender<PongResult>)]
    Ping(PingReq),

    // -- Session --
    #[rpc(tx = oneshot::Sender<SessionInfoResult>)]
    GetSessionInfo(SessionInfoReq),

    #[rpc(tx = oneshot::Sender<SessionListResult>)]
    ListSessions(SessionListReq),

    #[rpc(tx = oneshot::Sender<SessionSwitchResult>)]
    SwitchSession(SessionSwitchReq),

    // -- Filesystem --
    #[rpc(tx = oneshot::Sender<FsListResult>)]
    FsList(FsListReq),

    #[rpc(tx = oneshot::Sender<FsReadResult>)]
    FsRead(FsReadReq),

    #[rpc(tx = oneshot::Sender<FsWriteResult>)]
    FsWrite(FsWriteReq),

    #[rpc(tx = oneshot::Sender<FsStatResult>)]
    FsStat(FsStatReq),

    // -- Terminal --
    #[rpc(tx = oneshot::Sender<TermCreateResult>)]
    TermCreate(TermCreateReq),

    /// Subscribe to host-initiated events (terminal created, etc.).
    /// The host pushes `HostEvent` values through the returned channel.
    /// Only one subscription is active per session; a new Subscribe replaces
    /// the old one. The channel stays open until the client disconnects.
    #[rpc(tx = mpsc::Sender<HostEvent>)]
    Subscribe(SubscribeReq),

    #[rpc(rx = mpsc::Receiver<TermInput>, tx = mpsc::Sender<TermOutput>)]
    TermAttach(TermAttachReq),

    #[rpc(tx = oneshot::Sender<TermResizeResult>)]
    TermResize(TermResizeReq),

    #[rpc(tx = oneshot::Sender<TermCloseResult>)]
    TermClose(TermCloseReq),

    #[rpc(tx = oneshot::Sender<TermListResult>)]
    TermList(TermListReq),

    // -- Git --
    #[rpc(tx = oneshot::Sender<GitStatusResult>)]
    GitStatus(GitStatusReq),

    #[rpc(tx = oneshot::Sender<GitDiffResult>)]
    GitDiff(GitDiffReq),

    #[rpc(tx = oneshot::Sender<GitLogResult>)]
    GitLog(GitLogReq),

    #[rpc(tx = oneshot::Sender<GitCommitResult>)]
    GitCommit(GitCommitReq),

    #[rpc(tx = oneshot::Sender<GitStageResult>)]
    GitStage(GitStageReq),

    #[rpc(tx = oneshot::Sender<GitUnstageResult>)]
    GitUnstage(GitUnstageReq),

    #[rpc(tx = oneshot::Sender<GitBranchesResult>)]
    GitBranches(GitBranchesReq),

    #[rpc(tx = oneshot::Sender<GitCheckoutResult>)]
    GitCheckout(GitCheckoutReq),

    // -- AI --
    #[rpc(tx = oneshot::Sender<AiPromptResult>)]
    AiPrompt(AiPromptReq),

    // -- LSP --
    #[rpc(tx = oneshot::Sender<LspDiagnosticsResult>)]
    LspDiagnostics(LspDiagnosticsReq),

    #[rpc(tx = oneshot::Sender<LspHoverResult>)]
    LspHover(LspHoverReq),

    // -- Filesystem observers (added later; keep at enum tail) --
    #[rpc(tx = oneshot::Sender<FsWatchResult>)]
    FsWatch(FsWatchReq),

    #[rpc(tx = oneshot::Sender<FsUnwatchResult>)]
    FsUnwatch(FsUnwatchReq),

    // -- TCP Proxy --
    /// Open a raw TCP tunnel to a port on the host's loopback interface.
    /// One stream instance per accepted TCP connection on the mobile proxy server.
    /// Host sends TcpData { data: [], closed: true } immediately if the port is unreachable.
    #[rpc(rx = mpsc::Receiver<TcpData>, tx = mpsc::Sender<TcpData>)]
    TcpTunnel(TcpTunnelReq),
}

// ---------------------------------------------------------------------------
// ALPN protocol identifier
// ---------------------------------------------------------------------------

pub const ZEDRA_ALPN: &[u8] = b"zedra/rpc/1";

/// Default page size for `FsList` requests (host uses this when `limit == 0`).
pub const FS_LIST_DEFAULT_LIMIT: u32 = 50;

// ---------------------------------------------------------------------------
// Serde helper for [u8; 64] (serde supports arrays only up to size 32 by default)
// ---------------------------------------------------------------------------

mod bytes64 {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let mut seq = s.serialize_tuple(64)?;
        for byte in val.iter() {
            seq.serialize_element(byte)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        use serde::de::{Error, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = [u8; 64];
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "exactly 64 bytes")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; 64], A::Error> {
                let mut arr = [0u8; 64];
                for (i, b) in arr.iter_mut().enumerate() {
                    *b = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(i, &self))?;
                }
                Ok(arr)
            }
        }
        d.deserialize_tuple(64, V)
    }
}

// ---------------------------------------------------------------------------
// Auth types
// ---------------------------------------------------------------------------

/// RegisterClient request — first pairing only.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterReq {
    /// Client's Ed25519 application public key (32 bytes).
    /// This is a SEPARATE key from the iroh transport key.
    pub client_pubkey: [u8; 32],
    /// Unix timestamp in seconds. Host rejects if |now - timestamp| > 60s.
    pub timestamp: u64,
    /// HMAC-SHA256(handshake_key, client_pubkey || timestamp_le_bytes).
    /// Proves the sender physically scanned the QR (has the handshake_key).
    pub hmac: [u8; 32],
    /// The session ID from the QR ticket (used to look up the pairing slot).
    pub slot_session_id: String,
}

/// Result of a RegisterClient attempt.
#[derive(Debug, Serialize, Deserialize)]
pub enum RegisterResult {
    /// Registration accepted. Client pubkey stored in authorized list and
    /// added to the session ACL. Proceed to Authenticate.
    Ok,
    /// Handshake slot already consumed by another device.
    /// Client should prompt: "This QR has already been used.
    /// Ask the host to run `zedra qr` to generate a new one."
    HandshakeConsumed,
    /// HMAC did not verify. Wrong key or tampered packet.
    InvalidHandshake,
    /// Timestamp outside ±60s window. Clock skew or replay attempt.
    StaleTimestamp,
    /// No pairing slot found for this session. QR may have expired (>10 min).
    SlotNotFound,
}

/// Authenticate request — sent on every connection (including after Register).
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthReq {
    /// Client's Ed25519 application public key (32 bytes).
    pub client_pubkey: [u8; 32],
}

/// Challenge issued by the host in response to Authenticate.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthChallengeResult {
    /// 32-byte random nonce generated fresh per connection.
    pub nonce: [u8; 32],
    /// Ed25519 signature of the nonce by the host's iroh SecretKey.
    /// Client MUST verify this using the stored EndpointId before signing
    /// the response — proves this challenge came from the real host.
    #[serde(with = "bytes64")]
    pub host_signature: [u8; 64],
}

/// AuthProve request — client signs the challenge nonce to prove identity.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthProveReq {
    /// Echo of the nonce from AuthChallengeResult.
    pub nonce: [u8; 32],
    /// Ed25519 signature of the nonce by the client's application SecretKey.
    #[serde(with = "bytes64")]
    pub client_signature: [u8; 64],
    /// The session ID the client wants to attach to.
    pub session_id: String,
}

/// Result of an AuthProve attempt.
#[derive(Debug, Serialize, Deserialize)]
pub enum AuthProveResult {
    /// Authentication succeeded and session attached. RPC calls may proceed.
    Ok,
    /// Client pubkey not in host's authorized list. Must re-pair via QR.
    Unauthorized,
    /// Client pubkey not in this session's ACL. Must pair via QR for this session.
    NotInSessionAcl,
    /// Another client is currently attached to this session.
    /// Use `zedra detach --session-id <id>` on the host to transfer ownership.
    SessionOccupied,
    /// Session ID not found. Session may have been removed or host restarted.
    SessionNotFound,
    /// The client_signature did not verify against the stored pubkey.
    InvalidSignature,
}

// ---------------------------------------------------------------------------
// Ping / Pong
// ---------------------------------------------------------------------------

/// Sent by the client every 2 seconds (foreground only).
#[derive(Debug, Serialize, Deserialize)]
pub struct PingReq {
    /// Client's Unix timestamp in milliseconds.
    /// Echoed back in PongResult so the client can compute RTT = now - timestamp_ms.
    pub timestamp_ms: u64,
}

/// Host echoes the timestamp for RTT measurement.
#[derive(Debug, Serialize, Deserialize)]
pub struct PongResult {
    pub timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// Session close reasons (sent as QUIC APPLICATION_CLOSE before disconnect)
// ---------------------------------------------------------------------------

/// Reason codes for host-initiated connection close.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u32)]
pub enum SessionCloseReason {
    /// Host operator ran `zedra detach --session-id <id>`.
    /// Active client receives this so the UI can show "Session taken over"
    /// rather than a generic connection drop.
    SessionTakenOver = 1,
    /// Host process is shutting down cleanly (SIGTERM / `zedra stop`).
    /// Client should show "Host disconnected" briefly then auto-reconnect.
    HostShutdown = 2,
}

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionInfoReq {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfoResult {
    pub hostname: String,
    pub workdir: String,
    pub username: String,
    pub session_id: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub os_version: Option<String>,
    pub host_version: Option<String>,
    pub home_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionListReq {}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionListResult {
    pub sessions: Vec<SessionListEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListEntry {
    pub id: String,
    pub name: Option<String>,
    pub workdir: Option<String>,
    pub terminal_count: usize,
    pub uptime_secs: u64,
    pub idle_secs: u64,
    /// Whether another client is currently attached to this session.
    pub is_occupied: bool,
}

/// Switch to a named session after authentication.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSwitchReq {
    pub session_name: String,
    pub last_notif_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSwitchResult {
    pub session_id: String,
    pub workdir: Option<String>,
}

// ---------------------------------------------------------------------------
// Filesystem types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct FsListReq {
    pub path: String,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsListResult {
    pub entries: Vec<FsEntry>,
    pub total: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsReadReq {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsReadResult {
    pub content: String,
    pub too_large: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsWriteReq {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsWriteResult {
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsStatReq {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsStatResult {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsWatchReq {
    /// Relative directory path to observe (for example: ".", "src", "src/editor").
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FsWatchResult {
    Ok,
    InvalidPath,
    RateLimited,
    QuotaExceeded,
    /// Client-local fallback when connected host does not support this RPC yet.
    Unsupported,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsUnwatchReq {
    /// Relative directory path to stop observing.
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum FsUnwatchResult {
    Ok,
    InvalidPath,
    RateLimited,
    NotWatched,
    /// Client-local fallback when connected host does not support this RPC yet.
    Unsupported,
}

// ---------------------------------------------------------------------------
// Subscribe / HostEvent types
// ---------------------------------------------------------------------------

/// Subscribe request — no fields needed; the response channel carries events.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribeReq {}

/// Events pushed from the host to the connected client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostEvent {
    /// A new terminal was created externally (e.g. via the local REST API).
    /// The client should open and display this terminal.
    TerminalCreated {
        id: String,
        /// The launch command injected into the terminal, if any.
        launch_cmd: Option<String>,
    },
    /// Host-side git working tree state changed and the client should refresh.
    GitChanged,
    /// A watched directory path changed and the client should invalidate its cached tree.
    FsChanged { path: String },
}

// ---------------------------------------------------------------------------
// Terminal types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct TermCreateReq {
    pub cols: u16,
    pub rows: u16,
    /// Optional shell command to run immediately after the shell starts.
    /// If `None`, the host's default launch command (if any) is used.
    /// Example: `"claude --resume"` to drop straight into a Claude session.
    pub launch_cmd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermCreateResult {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermAttachReq {
    pub id: String,
    pub last_seq: u64,
}

/// Terminal input from client to server (raw PTY bytes).
#[derive(Debug, Serialize, Deserialize)]
pub struct TermInput {
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// Terminal output from server to client (raw PTY bytes).
#[derive(Debug, Serialize, Deserialize)]
pub struct TermOutput {
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    pub seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermResizeReq {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermResizeResult {
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermCloseReq {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermCloseResult {
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermListReq {}

#[derive(Debug, Serialize, Deserialize)]
pub struct TermListResult {
    pub terminals: Vec<TermListEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermListEntry {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Git types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct GitStatusReq {}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitStatusResult {
    pub branch: String,
    pub entries: Vec<GitStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusEntry {
    pub path: String,
    /// Index status shown in the "staged" section. `None` means no staged change.
    pub staged_status: Option<String>,
    /// Working tree status shown in the "changes" / "untracked" sections.
    /// `None` means no unstaged change.
    pub unstaged_status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitDiffReq {
    pub path: Option<String>,
    pub staged: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitDiffResult {
    pub diff: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitLogReq {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitLogResult {
    pub entries: Vec<GitLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLogEntry {
    pub id: String,
    pub message: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitCommitReq {
    pub message: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitCommitResult {
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitStageReq {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitStageResult {}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitUnstageReq {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitUnstageResult {}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitBranchesReq {}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitBranchesResult {
    pub branches: Vec<GitBranchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranchEntry {
    pub name: String,
    pub is_head: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitCheckoutReq {
    pub branch: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitCheckoutResult {
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// AI types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct AiPromptReq {
    pub prompt: String,
    pub context: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AiPromptResult {
    pub text: String,
    pub done: bool,
}

// ---------------------------------------------------------------------------
// LSP types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct LspDiagnosticsReq {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LspDiagnosticsResult {
    pub diagnostics: Vec<LspDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub message: String,
    pub severity: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LspHoverReq {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LspHoverResult {
    pub contents: String,
}

// ---------------------------------------------------------------------------
// TCP Proxy types
// ---------------------------------------------------------------------------

/// Request to open a raw TCP tunnel to a port on the host's loopback.
#[derive(Debug, Serialize, Deserialize)]
pub struct TcpTunnelReq {
    /// Target port on the host's loopback (e.g. 3000 for a local dev server).
    pub port: u16,
}

/// A chunk of raw TCP data, flowing in either direction through a TcpTunnel stream.
///
/// When `closed` is true and `data` is empty, signals connection end (FIN) or
/// refused (host sends this immediately on the first message if the target port
/// is unreachable).
#[derive(Debug, Serialize, Deserialize)]
pub struct TcpData {
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    /// True on the final message — signals TCP half-close (FIN) or connection refused.
    pub closed: bool,
}

// ---------------------------------------------------------------------------
// Backlog entry for session_registry
// ---------------------------------------------------------------------------

/// Raw byte backlog entry stored per-terminal for replay on reconnect.
#[derive(Debug, Clone)]
pub struct BacklogEntry {
    pub seq: u64,
    pub terminal_id: String,
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_output_roundtrip() {
        let output = TermOutput {
            data: b"prompt$ ".to_vec(),
            seq: 42,
        };
        let encoded = postcard::to_allocvec(&output).unwrap();
        let decoded: TermOutput = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.data, b"prompt$ ");
        assert_eq!(decoded.seq, 42);
    }

    #[test]
    fn register_req_roundtrip() {
        let req = RegisterReq {
            client_pubkey: [1u8; 32],
            timestamp: 1_700_000_000,
            hmac: [2u8; 32],
            slot_session_id: "sess-123".to_string(),
        };
        let encoded = postcard::to_allocvec(&req).unwrap();
        let decoded: RegisterReq = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.client_pubkey, [1u8; 32]);
        assert_eq!(decoded.timestamp, 1_700_000_000);
        assert_eq!(decoded.slot_session_id, "sess-123");
    }

    #[test]
    fn auth_prove_req_roundtrip() {
        let req = AuthProveReq {
            nonce: [3u8; 32],
            client_signature: [4u8; 64],
            session_id: "sess-abc".to_string(),
        };
        let encoded = postcard::to_allocvec(&req).unwrap();
        let decoded: AuthProveReq = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.nonce, [3u8; 32]);
        assert_eq!(decoded.session_id, "sess-abc");
    }

    #[test]
    fn ping_roundtrip() {
        let req = PingReq {
            timestamp_ms: 9_999_999,
        };
        let encoded = postcard::to_allocvec(&req).unwrap();
        let decoded: PingReq = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.timestamp_ms, 9_999_999);
    }
}
