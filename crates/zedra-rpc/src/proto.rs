// irpc protocol definition for Zedra RPC.
//
// Replaces the JSON-RPC 2.0 protocol with typed, binary-serialized (postcard)
// messages over iroh QUIC streams. Each variant maps to an RPC method with
// typed request/response pairs.

use irpc::channel::{mpsc, oneshot};
use irpc::rpc_requests;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Protocol enum
// ---------------------------------------------------------------------------

#[rpc_requests(message = ZedraMessage)]
#[derive(Serialize, Deserialize, Debug)]
pub enum ZedraProto {
    // -- Session --
    #[rpc(tx = oneshot::Sender<ResumeResult>)]
    ResumeOrCreate(ResumeOrCreateReq),

    #[rpc(tx = oneshot::Sender<HeartbeatResult>)]
    Heartbeat(HeartbeatReq),

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
}

// ---------------------------------------------------------------------------
// ALPN protocol identifier
// ---------------------------------------------------------------------------

pub const ZEDRA_ALPN: &[u8] = b"zedra/rpc/2";

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeOrCreateReq {
    pub session_id: Option<String>,
    pub auth_token: String,
    pub last_notif_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeResult {
    pub session_id: String,
    pub resumed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HeartbeatReq {}

#[derive(Debug, Serialize, Deserialize)]
pub struct HeartbeatResult {
    pub ok: bool,
}

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSwitchReq {
    pub session_name: String,
    pub auth_token: String,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsListResult {
    pub entries: Vec<FsEntry>,
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

// ---------------------------------------------------------------------------
// Terminal types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct TermCreateReq {
    pub cols: u16,
    pub rows: u16,
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
    pub cols: u16,
    pub rows: u16,
    /// Shell title (from OSC 0/2 escape sequences), if available.
    pub title: Option<String>,
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
    pub status: String,
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
    fn term_input_serde_roundtrip() {
        let input = TermInput {
            data: b"hello world".to_vec(),
        };
        let encoded = postcard::to_allocvec(&input).unwrap();
        let decoded: TermInput = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.data, b"hello world");
    }

    #[test]
    fn term_output_serde_roundtrip() {
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
    fn resume_or_create_roundtrip() {
        let req = ResumeOrCreateReq {
            session_id: Some("abc-123".to_string()),
            auth_token: "tok".to_string(),
            last_notif_seq: 99,
        };
        let encoded = postcard::to_allocvec(&req).unwrap();
        let decoded: ResumeOrCreateReq = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.session_id.as_deref(), Some("abc-123"));
        assert_eq!(decoded.last_notif_seq, 99);
    }

    #[test]
    fn fs_entry_roundtrip() {
        let entry = FsEntry {
            name: "main.rs".to_string(),
            path: "/src/main.rs".to_string(),
            is_dir: false,
            size: 1024,
        };
        let encoded = postcard::to_allocvec(&entry).unwrap();
        let decoded: FsEntry = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.name, "main.rs");
        assert!(!decoded.is_dir);
    }

    #[test]
    fn git_status_roundtrip() {
        let status = GitStatusResult {
            branch: "main".to_string(),
            entries: vec![GitStatusEntry {
                path: "src/lib.rs".to_string(),
                status: "modified".to_string(),
            }],
        };
        let encoded = postcard::to_allocvec(&status).unwrap();
        let decoded: GitStatusResult = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.branch, "main");
        assert_eq!(decoded.entries.len(), 1);
    }
}
