// End-to-end tests for the RPC daemon.
//
// Spins up a real daemon over TCP, connects an RpcClient, and exercises the
// full request→handler→response loop for every domain: filesystem, git,
// terminal, AI, and LSP.

use std::process::Command;
use std::sync::Arc;
use tokio::net::TcpListener;
use zedra_rpc::{methods, RpcClient};

use zedra_host::rpc_daemon::{start_on_listener, DaemonState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spin up a temp git repo, daemon listener, and connect an RpcClient.
async fn setup() -> (
    tempfile::TempDir,
    RpcClient,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();

    // Initialise a git repo with a commit so git/log works.
    for args in [
        vec!["init"],
        vec!["config", "user.email", "e2e@test.com"],
        vec!["config", "user.name", "E2E"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .unwrap();
    }

    // Seed files
    std::fs::write(dir.path().join("README.md"), "# E2E test repo").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

    // Initial commit so branch exists
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let state = Arc::new(DaemonState::new(dir.path().to_path_buf()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state_clone = state.clone();
    let server_handle = tokio::spawn(async move {
        let _ = start_on_listener(&listener, state_clone).await;
    });

    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (r, w) = tokio::io::split(stream);
    let (client, _notifs) = RpcClient::spawn(r, w);

    (dir, client, server_handle)
}

// ---------------------------------------------------------------------------
// Filesystem E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_fs_list_read_write_stat() {
    let (_dir, client, handle) = setup().await;

    // List root — should contain README.md and src/
    let resp = client
        .call(methods::FS_LIST, serde_json::json!({"path": "."}))
        .await
        .unwrap();
    assert!(resp.error.is_none(), "fs/list error: {:?}", resp.error);
    let entries: Vec<zedra_rpc::FsEntry> =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(entries.iter().any(|e| e.name == "README.md"));
    assert!(entries.iter().any(|e| e.name == "src" && e.is_dir));

    // Write a new file
    let resp = client
        .call(
            methods::FS_WRITE,
            serde_json::json!({"path": "test.txt", "content": "e2e content"}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // Read it back
    let resp = client
        .call(methods::FS_READ, serde_json::json!({"path": "test.txt"}))
        .await
        .unwrap();
    let result: zedra_rpc::FsReadResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert_eq!(result.content, "e2e content");

    // Stat
    let resp = client
        .call(methods::FS_STAT, serde_json::json!({"path": "test.txt"}))
        .await
        .unwrap();
    let stat: zedra_rpc::FsStatResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(!stat.is_dir);
    assert!(stat.size > 0);

    // List subdirectory
    let resp = client
        .call(methods::FS_LIST, serde_json::json!({"path": "src"}))
        .await
        .unwrap();
    let entries: Vec<zedra_rpc::FsEntry> =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(entries.iter().any(|e| e.name == "main.rs"));

    handle.abort();
}

// ---------------------------------------------------------------------------
// Git E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_git_full_flow() {
    let (dir, client, handle) = setup().await;

    // Status — should be clean after initial commit
    let resp = client
        .call(methods::GIT_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    assert!(resp.error.is_none());
    let status: zedra_rpc::GitStatusResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(
        status.entries.is_empty(),
        "should be clean after init commit, got: {:?}",
        status.entries
    );

    // Log — should have the initial commit
    let resp = client
        .call(methods::GIT_LOG, serde_json::json!({"limit": 5}))
        .await
        .unwrap();
    let log: Vec<zedra_rpc::GitLogEntry> =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(!log.is_empty(), "should have at least one commit");
    assert_eq!(log[0].message, "init");

    // Branches
    let resp = client
        .call(methods::GIT_BRANCH_LIST, serde_json::json!({}))
        .await
        .unwrap();
    let branches: Vec<zedra_rpc::GitBranchEntry> =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(branches.iter().any(|b| b.is_head));

    // Create a new file, then check status shows it
    std::fs::write(dir.path().join("new_file.txt"), "new").unwrap();

    let resp = client
        .call(methods::GIT_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    let status: zedra_rpc::GitStatusResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(
        status.entries.iter().any(|e| e.path == "new_file.txt"),
        "new file should appear in status"
    );

    // Diff
    let resp = client
        .call(
            methods::GIT_DIFF,
            serde_json::json!({"path": null, "staged": false}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // Commit
    let resp = client
        .call(
            methods::GIT_COMMIT,
            serde_json::json!({"message": "add new_file", "paths": ["new_file.txt"]}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "commit error: {:?}", resp.error);
    let result: serde_json::Value = resp.result.unwrap();
    assert!(result.get("hash").is_some(), "commit should return hash");

    // Verify log now has 2 commits
    let resp = client
        .call(methods::GIT_LOG, serde_json::json!({"limit": 10}))
        .await
        .unwrap();
    let log: Vec<zedra_rpc::GitLogEntry> =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(log.len() >= 2, "should have ≥2 commits, got {}", log.len());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Terminal E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_terminal_create_data_resize_close() {
    let (_dir, client, handle) = setup().await;

    // Create
    let resp = client
        .call(
            methods::TERM_CREATE,
            serde_json::json!({"cols": 80, "rows": 24}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "create: {:?}", resp.error);
    let term: zedra_rpc::TermCreateResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    let id = &term.id;
    assert!(id.starts_with("term-"));

    // Send data (echo command)
    let data = base64_url::encode(b"echo hello\n");
    let resp = client
        .call(
            methods::TERM_DATA,
            serde_json::json!({"id": id, "data": data}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "data: {:?}", resp.error);

    // Resize
    let resp = client
        .call(
            methods::TERM_RESIZE,
            serde_json::json!({"id": id, "cols": 120, "rows": 40}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "resize: {:?}", resp.error);

    // Close
    let resp = client
        .call(methods::TERM_CLOSE, serde_json::json!({"id": id}))
        .await
        .unwrap();
    assert!(resp.error.is_none(), "close: {:?}", resp.error);

    // Operating on closed terminal should error
    let resp = client
        .call(
            methods::TERM_RESIZE,
            serde_json::json!({"id": id, "cols": 80, "rows": 24}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_some(), "should fail on closed terminal");

    handle.abort();
}

// ---------------------------------------------------------------------------
// AI E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_ai_prompt() {
    let (_dir, client, handle) = setup().await;

    let resp = client
        .call(
            methods::AI_PROMPT,
            serde_json::json!({"prompt": "What is 1+1?", "context": null}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none(), "ai/prompt: {:?}", resp.error);
    let chunk: zedra_rpc::AiStreamChunk =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(chunk.done);
    assert!(!chunk.text.is_empty());
}

// ---------------------------------------------------------------------------
// LSP E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_lsp_diagnostics_and_hover() {
    let (_dir, client, handle) = setup().await;

    // Diagnostics on a txt file — should return empty (no linter)
    let resp = client
        .call(
            "lsp/diagnostics",
            serde_json::json!({"path": "README.md"}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // Hover
    let resp = client
        .call("lsp/hover", serde_json::json!({"path": "README.md"}))
        .await
        .unwrap();
    assert!(resp.error.is_none());
    let result: serde_json::Value = resp.result.unwrap();
    assert!(result.get("contents").is_some());

    handle.abort();
}

// ---------------------------------------------------------------------------
// Error handling E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_unknown_method() {
    let (_dir, client, handle) = setup().await;

    let resp = client
        .call("nonexistent/method", serde_json::json!({}))
        .await
        .unwrap();
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, zedra_rpc::METHOD_NOT_FOUND);

    handle.abort();
}

#[tokio::test]
async fn e2e_invalid_params() {
    let (_dir, client, handle) = setup().await;

    // fs/read with missing path field
    let resp = client
        .call(methods::FS_READ, serde_json::json!({}))
        .await
        .unwrap();
    assert!(resp.error.is_some(), "should fail with missing params");

    handle.abort();
}

#[tokio::test]
async fn e2e_fs_read_nonexistent() {
    let (_dir, client, handle) = setup().await;

    let resp = client
        .call(
            methods::FS_READ,
            serde_json::json!({"path": "does_not_exist.txt"}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_some(), "reading nonexistent file should fail");

    handle.abort();
}

// ---------------------------------------------------------------------------
// Multi-call sequencing E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_sequential_multi_domain() {
    let (_dir, client, handle) = setup().await;

    // 1. Write a file via RPC
    let resp = client
        .call(
            methods::FS_WRITE,
            serde_json::json!({"path": "seq_test.txt", "content": "sequential"}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // 2. Read it back
    let resp = client
        .call(methods::FS_READ, serde_json::json!({"path": "seq_test.txt"}))
        .await
        .unwrap();
    let result: zedra_rpc::FsReadResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert_eq!(result.content, "sequential");

    // 3. Verify it shows in git status
    let resp = client
        .call(methods::GIT_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    let status: zedra_rpc::GitStatusResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(status.entries.iter().any(|e| e.path == "seq_test.txt"));

    // 4. Commit it
    let resp = client
        .call(
            methods::GIT_COMMIT,
            serde_json::json!({"message": "seq test", "paths": ["seq_test.txt"]}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // 5. Status should be clean now
    let resp = client
        .call(methods::GIT_STATUS, serde_json::json!({}))
        .await
        .unwrap();
    let status: zedra_rpc::GitStatusResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    assert!(
        status.entries.is_empty(),
        "should be clean after commit, got: {:?}",
        status.entries
    );

    // 6. Spawn and close a terminal
    let resp = client
        .call(
            methods::TERM_CREATE,
            serde_json::json!({"cols": 80, "rows": 24}),
        )
        .await
        .unwrap();
    let term: zedra_rpc::TermCreateResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();

    let resp = client
        .call(methods::TERM_CLOSE, serde_json::json!({"id": term.id}))
        .await
        .unwrap();
    assert!(resp.error.is_none());

    // 7. AI prompt
    let resp = client
        .call(
            methods::AI_PROMPT,
            serde_json::json!({"prompt": "hello"}),
        )
        .await
        .unwrap();
    assert!(resp.error.is_none());

    handle.abort();
}
