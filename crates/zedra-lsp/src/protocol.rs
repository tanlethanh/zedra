//! Minimal LSP JSON-RPC client over stdio.
//!
//! Scope: framing, initialize/initialized handshake, parse
//! `textDocument/publishDiagnostics` notifications. Hover, code-action,
//! goto-definition, and didOpen/didChange wire-up land in follow-up commits.
//! Servers like rust-analyzer and gopls scan the workspace on `initialize`
//! and publish diagnostics without the client needing to open files, which
//! is enough to prove the end-to-end push path before buffer sync arrives.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use zedra_rpc::proto::{LspDiagnosticV2, LspPosition, LspRange, LspRelated, LspSeverity};

/// Per-file diagnostic store. Wholesale replacement: each `publishDiagnostics`
/// overwrites the entry for that path.
pub type DiagnosticStore = HashMap<PathBuf, Vec<LspDiagnosticV2>>;

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcNotification<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    #[allow(dead_code)]
    id: Option<Value>,
    #[serde(default)]
    #[allow(dead_code)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
}

/// Write one Content-Length-framed JSON-RPC message to `stdin`.
async fn write_message(stdin: &mut ChildStdin, body: &str) -> Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin
        .write_all(header.as_bytes())
        .await
        .context("write LSP header")?;
    stdin
        .write_all(body.as_bytes())
        .await
        .context("write LSP body")?;
    stdin.flush().await.context("flush LSP stdin")?;
    Ok(())
}

/// Read one Content-Length-framed message from `stdout`. Returns the JSON
/// body as a `String`.
async fn read_message(stdout: &mut ChildStdout) -> Result<String> {
    // Read headers byte-by-byte until \r\n\r\n.
    let mut headers = Vec::with_capacity(64);
    let mut last4 = [0u8; 4];
    let mut byte = [0u8; 1];
    loop {
        let n = stdout.read(&mut byte).await.context("read LSP header")?;
        if n == 0 {
            return Err(anyhow!("LSP server closed stdout"));
        }
        headers.push(byte[0]);
        last4.rotate_left(1);
        last4[3] = byte[0];
        if last4 == *b"\r\n\r\n" {
            break;
        }
        if headers.len() > 8192 {
            return Err(anyhow!("LSP header exceeded 8 KiB"));
        }
    }
    let header_str = std::str::from_utf8(&headers).context("non-UTF8 LSP header")?;
    let mut content_length = 0usize;
    for line in header_str.split("\r\n") {
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().context("invalid Content-Length")?;
        }
    }
    if content_length == 0 {
        return Err(anyhow!("missing Content-Length"));
    }
    if content_length > 16 * 1024 * 1024 {
        return Err(anyhow!(
            "LSP message body too large ({} bytes); refusing",
            content_length,
        ));
    }
    let mut body = vec![0u8; content_length];
    stdout
        .read_exact(&mut body)
        .await
        .context("read LSP body")?;
    String::from_utf8(body).context("non-UTF8 LSP body")
}

/// Perform the `initialize` → `initialized` handshake. Returns when the
/// server has acknowledged `initialized`, after which it is free to push
/// diagnostics. `workspace_root` is the absolute path of the workspace.
pub async fn handshake(
    stdin: &mut ChildStdin,
    stdout: &mut ChildStdout,
    workspace_root: &std::path::Path,
) -> Result<()> {
    let root_uri = path_to_uri(workspace_root);
    let init_params = serde_json::json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "zedra-lsp", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }],
        "capabilities": {
            "textDocument": {
                "publishDiagnostics": { "relatedInformation": true, "versionSupport": false }
            },
            "workspace": { "workspaceFolders": true }
        },
    });
    let req = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: init_params,
    };
    let body = serde_json::to_string(&req)?;
    write_message(stdin, &body).await?;

    // Drain until we see the initialize response (id=1). Some servers push
    // progress notifications before responding.
    loop {
        let body = read_message(stdout).await?;
        let parsed: JsonRpcResponse = serde_json::from_str(&body)
            .with_context(|| format!("invalid JSON from LSP server: {body}"))?;
        if parsed.id == Some(Value::from(1)) {
            if let Some(err) = parsed.error {
                return Err(anyhow!("LSP initialize failed: {err}"));
            }
            break;
        }
    }

    let notif = JsonRpcNotification {
        jsonrpc: "2.0",
        method: "initialized",
        params: serde_json::json!({}),
    };
    let body = serde_json::to_string(&notif)?;
    write_message(stdin, &body).await?;
    Ok(())
}

/// Background reader loop. Parses incoming messages and updates `store` on
/// every `textDocument/publishDiagnostics` notification. Exits cleanly when
/// the server closes its stdout.
pub async fn run_reader(
    mut stdout: ChildStdout,
    store: Arc<Mutex<DiagnosticStore>>,
    on_diagnostic: impl Fn(PathBuf, Vec<LspDiagnosticV2>) + Send + 'static,
) {
    loop {
        let body = match read_message(&mut stdout).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("LSP reader exiting: {}", e);
                return;
            }
        };
        let parsed: JsonRpcResponse = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("LSP malformed message: {}", e);
                continue;
            }
        };
        let Some(method) = parsed.method.as_deref() else {
            continue;
        };
        if method != "textDocument/publishDiagnostics" {
            continue;
        }
        let Some(params) = parsed.params else {
            continue;
        };
        let Some((path, diagnostics)) = decode_publish_diagnostics(&params) else {
            continue;
        };
        {
            let mut guard = store.lock().await;
            if diagnostics.is_empty() {
                guard.remove(&path);
            } else {
                guard.insert(path.clone(), diagnostics.clone());
            }
        }
        on_diagnostic(path, diagnostics);
    }
}

fn decode_publish_diagnostics(params: &Value) -> Option<(PathBuf, Vec<LspDiagnosticV2>)> {
    let uri = params.get("uri")?.as_str()?;
    let path = uri_to_path(uri)?;
    let raw = params.get("diagnostics")?.as_array()?;
    let diagnostics = raw.iter().filter_map(decode_diagnostic).collect();
    Some((path, diagnostics))
}

fn decode_diagnostic(v: &Value) -> Option<LspDiagnosticV2> {
    let range = decode_range(v.get("range")?)?;
    let severity = match v.get("severity").and_then(|s| s.as_u64()) {
        Some(1) => LspSeverity::Error,
        Some(2) => LspSeverity::Warning,
        Some(3) => LspSeverity::Info,
        Some(4) => LspSeverity::Hint,
        _ => LspSeverity::Info,
    };
    let code = v.get("code").map(|c| match c {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    let source = v
        .get("source")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let message = v
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let related = v
        .get("relatedInformation")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(decode_related).collect())
        .unwrap_or_default();
    Some(LspDiagnosticV2 {
        range,
        severity,
        code,
        source,
        message,
        related,
    })
}

fn decode_related(v: &Value) -> Option<LspRelated> {
    let location = v.get("location")?;
    let uri = location.get("uri")?.as_str()?;
    let path = uri_to_path(uri)?;
    let range = decode_range(location.get("range")?)?;
    let message = v
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    Some(LspRelated {
        path: path.to_string_lossy().into_owned(),
        range,
        message,
    })
}

fn decode_range(v: &Value) -> Option<LspRange> {
    let start = decode_position(v.get("start")?)?;
    let end = decode_position(v.get("end")?)?;
    Some(LspRange {
        start_line: start.line,
        start_col: start.col,
        end_line: end.line,
        end_col: end.col,
    })
}

fn decode_position(v: &Value) -> Option<LspPosition> {
    Some(LspPosition {
        line: v.get("line")?.as_u64()? as u32,
        col: v.get("character")?.as_u64()? as u32,
    })
}

fn path_to_uri(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    if s.starts_with("/") {
        format!("file://{}", s)
    } else {
        format!("file:///{}", s.replace('\\', "/"))
    }
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest
        .strip_prefix('/')
        .map(|r| format!("/{r}"))
        .unwrap_or_else(|| rest.to_string());
    Some(PathBuf::from(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_publish_diagnostics_parses_rust_analyzer_payload() {
        let raw = serde_json::json!({
            "uri": "file:///tmp/zedra/src/lib.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 12, "character": 4 },
                        "end":   { "line": 12, "character": 9 }
                    },
                    "severity": 1,
                    "code": "E0382",
                    "source": "rustc",
                    "message": "borrow of moved value: `foo`"
                }
            ]
        });
        let (path, diags) = decode_publish_diagnostics(&raw).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/zedra/src/lib.rs"));
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.severity, LspSeverity::Error);
        assert_eq!(d.code.as_deref(), Some("E0382"));
        assert_eq!(d.source.as_deref(), Some("rustc"));
        assert_eq!(d.range.start_line, 12);
        assert_eq!(d.range.start_col, 4);
    }

    #[test]
    fn empty_diagnostics_array_is_clearing_signal() {
        let raw = serde_json::json!({
            "uri": "file:///tmp/a.rs",
            "diagnostics": []
        });
        let (_path, diags) = decode_publish_diagnostics(&raw).unwrap();
        assert!(diags.is_empty());
    }
}
