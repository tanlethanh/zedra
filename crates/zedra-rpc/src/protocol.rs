// JSON-RPC 2.0 message types for Zedra remote protocol

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a unique request ID.
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Core JSON-RPC 2.0
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Envelope: any message on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl Request {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: next_id(),
            method: method.into(),
            params,
        }
    }
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

impl Notification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Standard error codes
// ---------------------------------------------------------------------------

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// ---------------------------------------------------------------------------
// Domain-specific RPC methods
// ---------------------------------------------------------------------------

/// All RPC method names used by the Zedra protocol.
pub mod methods {
    // Filesystem
    pub const FS_LIST: &str = "fs/list";
    pub const FS_READ: &str = "fs/read";
    pub const FS_WRITE: &str = "fs/write";
    pub const FS_STAT: &str = "fs/stat";
    pub const FS_MKDIR: &str = "fs/mkdir";
    pub const FS_REMOVE: &str = "fs/remove";

    // Terminal
    pub const TERM_CREATE: &str = "terminal/create";
    pub const TERM_DATA: &str = "terminal/data";
    pub const TERM_RESIZE: &str = "terminal/resize";
    pub const TERM_CLOSE: &str = "terminal/close";

    // Git
    pub const GIT_STATUS: &str = "git/status";
    pub const GIT_DIFF: &str = "git/diff";
    pub const GIT_LOG: &str = "git/log";
    pub const GIT_COMMIT: &str = "git/commit";
    pub const GIT_BRANCH_LIST: &str = "git/branches";
    pub const GIT_CHECKOUT: &str = "git/checkout";

    // AI / Claude Code
    pub const AI_PROMPT: &str = "ai/prompt";
    pub const AI_CANCEL: &str = "ai/cancel";

    // LSP proxy
    pub const LSP_DIAGNOSTICS: &str = "lsp/diagnostics";
    pub const LSP_HOVER: &str = "lsp/hover";

    // Notifications (server → client)
    pub const TERM_OUTPUT: &str = "terminal/output";
    pub const AI_STREAM: &str = "ai/stream";
}

// ---------------------------------------------------------------------------
// Domain parameter/result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsListParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsReadParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsReadResult {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsWriteParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsStatParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsStatResult {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusResult {
    pub branch: String,
    pub entries: Vec<GitStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusEntry {
    pub path: String,
    pub status: String, // "modified", "added", "deleted", "untracked", etc.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffParams {
    pub path: Option<String>,
    pub staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffResult {
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLogParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLogEntry {
    pub id: String,
    pub message: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitParams {
    pub message: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranchEntry {
    pub name: String,
    pub is_head: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermCreateParams {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermCreateResult {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermDataParams {
    pub id: String,
    pub data: String, // base64-encoded
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermResizeParams {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPromptParams {
    pub prompt: String,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiStreamChunk {
    pub text: String,
    pub done: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes() {
        let req = Request::new("fs/list", serde_json::json!({"path": "/tmp"}));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"fs/list\""));
    }

    #[test]
    fn response_ok_roundtrip() {
        let resp = Response::ok(1, serde_json::json!({"status": "ok"}));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, 1);
        assert!(parsed.error.is_none());
        assert!(parsed.result.is_some());
    }

    #[test]
    fn response_err_roundtrip() {
        let resp = Response::err(2, METHOD_NOT_FOUND, "not found");
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.error.as_ref().unwrap().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn notification_no_id() {
        let notif = Notification::new("terminal/output", serde_json::json!({"data": "hello"}));
        let json = serde_json::to_string(&notif).unwrap();
        assert!(!json.contains("\"id\""));
    }

    #[test]
    fn message_envelope_request() {
        let req = Request::new("fs/read", serde_json::json!({}));
        let json = serde_json::to_string(&req).unwrap();
        let msg: Message = serde_json::from_str(&json).unwrap();
        assert!(matches!(msg, Message::Request(_)));
    }

    #[test]
    fn unique_ids() {
        let a = next_id();
        let b = next_id();
        assert_ne!(a, b);
    }

    #[test]
    fn fs_entry_serde() {
        let entry = FsEntry {
            name: "main.rs".into(),
            path: "/src/main.rs".into(),
            is_dir: false,
            size: 1024,
        };
        let json = serde_json::to_value(&entry).unwrap();
        let parsed: FsEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.name, "main.rs");
        assert!(!parsed.is_dir);
    }

    #[test]
    fn git_status_serde() {
        let status = GitStatusResult {
            branch: "main".into(),
            entries: vec![GitStatusEntry {
                path: "src/lib.rs".into(),
                status: "modified".into(),
            }],
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: GitStatusResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.entries.len(), 1);
    }
}
