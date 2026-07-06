// RPC daemon: exposes filesystem, git, terminal, LSP, and AI operations over irpc.
//
// Connection lifecycle:
//   First pairing:  Register → Connect → Challenge → AuthProve → Ok(SyncSessionResult) → (RPC calls)
//   Token resume:   Connect(session_token) → Ok(SyncSessionResult) → (RPC calls)
//   PKI reconnect:  Connect(None) → Challenge → AuthProve → Ok(SyncSessionResult) → (RPC calls)
//   Health:         Ping (every 2s, foreground only, 5 misses = client reconnects)

use crate::agent;
use crate::agent::cache as agent_cache;
use crate::docs_tree::{
    build_snapshot, docs_tree_cache_key, docs_tree_limit, snapshot_page_result,
    validate_docs_tree_offset,
};
use crate::fs::{Filesystem, LocalFs};
use crate::git::GitRepo;
use crate::host_info;
use crate::identity::SharedIdentity;
use crate::metrics;
use crate::paths;
use crate::pty::{ShellSession, SpawnOptions};
use crate::session_registry::{
    finish_auth_failed_connection, finish_host_connection, ActiveClientConnection, AttachResult,
    ConsumeSlotResult, HostTermMeta, OutputSenderSlot, PairingSlotMode, ServerSession,
    SessionRegistry, TermBacklog, TermSession, MAX_WATCHED_PATHS_PER_SESSION,
};
use crate::uploads;
use crate::utils;
use anyhow::Result;
use iroh::endpoint::ConnectionError;
use irpc::rpc::{RemoteService, MAX_MESSAGE_SIZE};
use irpc::util::AsyncReadVarintExt;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zedra_rpc::proto::*;
use zedra_rpc::proto_v3::{ZedraProtoV3, ZEDRA_ALPN_V3};
use zedra_telemetry::Event;

/// Log a decode failure with the leading discriminant varint (the RPC's index in
/// `ZedraProto`/`ZedraProtoV3`) and a payload preview.
fn log_decode_failure(alpn: &[u8], buf: &[u8], err: &postcard::Error) {
    // Metadata only — never log payload bytes; client frames can carry paths,
    // file contents, or tokens.
    let variant_index = postcard::take_from_bytes::<u32>(buf).map(|(v, _)| v).ok();
    tracing::warn!(
        "ignoring undecodable request: variant_index={variant_index:?} alpn={} \
         payload_len={} error={err}",
        String::from_utf8_lossy(alpn),
        buf.len(),
    );
}

/// Read one request, decoding with the negotiated version (`zedra/rpc/3` lifts to
/// the live message). Mirrors `irpc_iroh::read_request` but decodes locally so a
/// failure can name the RPC (see `log_decode_failure`).
async fn read_zedra_message(
    conn: &iroh::endpoint::Connection,
) -> std::io::Result<Option<ZedraMessage>> {
    use std::io;

    // The negotiated ALPN is the only version seam; keep it local to decoding.
    let is_v3 = conn.alpn() == ZEDRA_ALPN_V3;

    let (send, mut recv) = match conn.accept_bi().await {
        Ok(pair) => pair,
        // Remote closed the connection cleanly (error code 0).
        Err(ConnectionError::ApplicationClosed(cause)) if cause.error_code.into_inner() == 0 => {
            return Ok(None);
        }
        Err(cause) => return Err(cause.into()),
    };

    let size = recv
        .read_varint_u64()
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "failed to read size"))?;
    if size > MAX_MESSAGE_SIZE {
        conn.close(1u32.into(), b"request exceeded max message size");
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request exceeded max message size",
        ));
    }
    let mut buf = vec![0u8; size as usize];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e))?;

    if is_v3 {
        match postcard::from_bytes::<ZedraProtoV3>(&buf) {
            Ok(proto) => Ok(Some(proto.with_remote_channels(recv, send).into_live())),
            Err(e) => {
                log_decode_failure(conn.alpn(), &buf, &e);
                Err(io::Error::new(io::ErrorKind::InvalidData, e))
            }
        }
    } else {
        match postcard::from_bytes::<ZedraProto>(&buf) {
            Ok(proto) => Ok(Some(proto.with_remote_channels(recv, send))),
            Err(e) => {
                log_decode_failure(conn.alpn(), &buf, &e);
                Err(io::Error::new(io::ErrorKind::InvalidData, e))
            }
        }
    }
}

struct HostEnvInfo {
    hostname: String,
    username: String,
    workdir: String,
    home_dir: Option<String>,
}

fn collect_host_env(workdir: &std::path::Path) -> HostEnvInfo {
    HostEnvInfo {
        hostname: hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string()),
        username: current_username(),
        workdir: paths::user_path_string(workdir),
        home_dir: current_home_dir(),
    }
}

fn current_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn current_home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .or_else(|| {
            directories::BaseDirs::new().map(|base| base.home_dir().to_string_lossy().into_owned())
        })
}

async fn build_sync_result(
    session: &Arc<ServerSession>,
    state: &DaemonState,
    session_token: [u8; 32],
) -> SyncSessionResult {
    let info = collect_host_env(&state.workdir);

    SyncSessionResult {
        session_id: session.id.clone(),
        session_token,
        hostname: info.hostname,
        workdir: info.workdir,
        username: info.username,
        home_dir: info.home_dir,
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        os_version: os_version_string(),
        host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        delta_pubkey: state.delta_pubkey,
        terminals: session.terminal_sync_entries().await,
    }
}

#[allow(unused)]
fn ts() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        (s % 86400) / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// Build a synthetic OSC preamble encoding cached terminal metadata.
/// Sent as seq=0 on TermAttach so the client seeds its meta from the PTY stream.
fn encode_meta_preamble(meta: &HostTermMeta) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(t) = &meta.title {
        out.extend_from_slice(b"\x1b]2;");
        out.extend_from_slice(t.as_bytes());
        out.push(0x07);
    }
    if let Some(name) = &meta.icon_name {
        out.extend_from_slice(b"\x1b]1;");
        out.extend_from_slice(name.as_bytes());
        out.push(0x07);
    }
    if let Some(c) = &meta.cwd {
        out.extend_from_slice(b"\x1b]7;file://");
        out.extend_from_slice(c.as_bytes());
        out.push(0x07);
    }

    match meta.shell_state {
        TermShellState::Running => {
            if let Some(command) = &meta.current_command {
                out.extend_from_slice(b"\x1b]633;E;");
                out.extend_from_slice(escape_osc633(command).as_bytes());
                out.push(0x07);
            }
            out.extend_from_slice(b"\x1b]633;C\x07");
        }
        TermShellState::Idle => {
            // Agent between turns: replay command + start + prompt-ready so
            // the client reseeds agent identity; a stale CommandEnd would
            // clear it.
            if let Some(command) = &meta.current_command {
                out.extend_from_slice(b"\x1b]633;E;");
                out.extend_from_slice(escape_osc633(command).as_bytes());
                out.push(0x07);
                out.extend_from_slice(b"\x1b]633;C\x07");
                out.extend_from_slice(b"\x1b]633;A\x07");
            } else if let Some(exit_code) = meta.last_exit_code {
                out.extend_from_slice(b"\x1b]633;D;");
                out.extend_from_slice(exit_code.to_string().as_bytes());
                out.push(0x07);
            } else {
                out.extend_from_slice(b"\x1b]633;A\x07");
            }
        }
        TermShellState::Unknown => {}
    }
    out
}

fn escape_osc633(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' | ';' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn initial_host_meta(opts: &SpawnOptions) -> HostTermMeta {
    // Launch commands can run before a prompt emits OSC 7; seed cwd from the spawn request.
    let mut meta = HostTermMeta {
        cwd: opts
            .workdir
            .as_ref()
            .map(|path| paths::user_path_string(path)),
        ..Default::default()
    };
    if let Some(command) = opts
        .launch_cmd
        .as_ref()
        .filter(|command| !command.is_empty())
    {
        // Spawned terminals never emit 633;E for the launch command itself.
        meta.current_command = Some(command.clone());
        meta.shell_state = TermShellState::Running;
        meta.refresh_agent_slug();
    }
    meta
}

#[derive(Default)]
enum ColorQueryScanState {
    #[default]
    Idle,
    SawEsc,
    SawBracket,
    Collecting {
        buf: Vec<u8>,
        esc_pending: bool,
    },
}

struct TerminalColorQueryResponder {
    color_scheme: TerminalColorScheme,
    state: ColorQueryScanState,
}

impl TerminalColorQueryResponder {
    fn new(color_scheme: TerminalColorScheme) -> Self {
        Self {
            color_scheme,
            state: ColorQueryScanState::Idle,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut replies = Vec::new();
        for &b in bytes {
            self.state = match std::mem::take(&mut self.state) {
                ColorQueryScanState::Idle => {
                    if b == 0x1b {
                        ColorQueryScanState::SawEsc
                    } else {
                        ColorQueryScanState::Idle
                    }
                }
                ColorQueryScanState::SawEsc => match b {
                    b']' => ColorQueryScanState::SawBracket,
                    0x1b => ColorQueryScanState::SawEsc,
                    _ => ColorQueryScanState::Idle,
                },
                ColorQueryScanState::SawBracket => match b {
                    0x07 => ColorQueryScanState::Idle,
                    0x1b => ColorQueryScanState::Collecting {
                        buf: Vec::new(),
                        esc_pending: true,
                    },
                    _ => ColorQueryScanState::Collecting {
                        buf: vec![b],
                        esc_pending: false,
                    },
                },
                ColorQueryScanState::Collecting {
                    mut buf,
                    esc_pending,
                } => {
                    if esc_pending {
                        match b {
                            b'\\' => {
                                if let Some(reply) =
                                    terminal_color_query_reply(&buf, self.color_scheme)
                                {
                                    replies.push(reply);
                                }
                                ColorQueryScanState::Idle
                            }
                            b']' => ColorQueryScanState::SawBracket,
                            _ => {
                                buf.push(0x1b);
                                buf.push(b);
                                ColorQueryScanState::Collecting {
                                    buf,
                                    esc_pending: false,
                                }
                            }
                        }
                    } else {
                        match b {
                            0x07 => {
                                if let Some(reply) =
                                    terminal_color_query_reply(&buf, self.color_scheme)
                                {
                                    replies.push(reply);
                                }
                                ColorQueryScanState::Idle
                            }
                            0x1b => ColorQueryScanState::Collecting {
                                buf,
                                esc_pending: true,
                            },
                            _ => {
                                buf.push(b);
                                if buf.len() > 64 {
                                    ColorQueryScanState::Idle
                                } else {
                                    ColorQueryScanState::Collecting {
                                        buf,
                                        esc_pending: false,
                                    }
                                }
                            }
                        }
                    }
                }
            };
        }
        replies
    }
}

fn terminal_color_query_reply(body: &[u8], color_scheme: TerminalColorScheme) -> Option<Vec<u8>> {
    let (kind, hex) = match body {
        b"10;?" => (b"10".as_slice(), terminal_foreground(color_scheme)),
        b"11;?" => (b"11".as_slice(), terminal_background(color_scheme)),
        b"12;?" => (b"12".as_slice(), terminal_cursor(color_scheme)),
        _ => return None,
    };
    Some(format_osc_color_reply(kind, hex))
}

fn terminal_foreground(color_scheme: TerminalColorScheme) -> u32 {
    match color_scheme {
        TerminalColorScheme::Dark => 0xabb2bf,
        TerminalColorScheme::Light => 0x1f2328,
    }
}

fn terminal_background(color_scheme: TerminalColorScheme) -> u32 {
    match color_scheme {
        TerminalColorScheme::Dark => 0x0e0c0c,
        TerminalColorScheme::Light => 0xfafafa,
    }
}

fn terminal_cursor(color_scheme: TerminalColorScheme) -> u32 {
    match color_scheme {
        TerminalColorScheme::Dark => 0x528bff,
        TerminalColorScheme::Light => 0x0969da,
    }
}

fn format_osc_color_reply(kind: &[u8], hex: u32) -> Vec<u8> {
    let r = ((hex >> 16) & 0xff) as u8;
    let g = ((hex >> 8) & 0xff) as u8;
    let b = (hex & 0xff) as u8;
    format!(
        "\x1b]{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x07",
        std::str::from_utf8(kind).unwrap_or("10"),
        r,
        r,
        g,
        g,
        b,
        b
    )
    .into_bytes()
}

#[cfg(test)]
mod terminal_meta_preamble_tests {
    use super::*;
    use std::path::PathBuf;
    use zedra_osc::{OscEvent, OscScanner};

    #[test]
    fn running_preamble_replays_command_before_start() {
        let meta = HostTermMeta {
            title: Some("Editing terminal_state.rs".to_owned()),
            icon_name: Some("codex".to_owned()),
            cwd: Some("/Users/thomasle/projects/zedra".to_owned()),
            current_command: Some("npx @openai/codex --prompt 'a;b'".to_owned()),
            shell_state: TermShellState::Running,
            last_exit_code: None,
            ..HostTermMeta::default()
        };

        let bytes = encode_meta_preamble(&meta);
        let events = OscScanner::new().feed(&bytes);

        assert!(
            matches!(&events[0], OscEvent::Title(title) if title == "Editing terminal_state.rs")
        );
        assert!(matches!(&events[1], OscEvent::IconName(icon_name) if icon_name == "codex"));
        assert!(
            matches!(&events[2], OscEvent::Cwd(cwd) if cwd == "/Users/thomasle/projects/zedra")
        );
        assert!(
            matches!(&events[3], OscEvent::CommandLine(command) if command == "npx @openai/codex --prompt 'a;b'")
        );
        assert!(matches!(events[4], OscEvent::CommandStart));
    }

    #[test]
    fn idle_preamble_replays_last_exit_code() {
        let meta = HostTermMeta {
            shell_state: TermShellState::Idle,
            last_exit_code: Some(17),
            ..HostTermMeta::default()
        };

        let bytes = encode_meta_preamble(&meta);
        let events = OscScanner::new().feed(&bytes);

        assert!(matches!(
            events.as_slice(),
            [OscEvent::CommandEnd { exit_code: 17 }]
        ));
    }

    #[test]
    fn idle_preamble_with_current_command_reseeds_identity() {
        // Agent between turns: prompt-ready after command start, no CommandEnd.
        let mut meta = HostTermMeta::default();
        meta.apply_osc_event(&OscEvent::CommandLine("codex".to_owned()));
        meta.apply_osc_event(&OscEvent::CommandStart);
        meta.apply_osc_event(&OscEvent::CommandEnd { exit_code: 0 });
        meta.apply_osc_event(&OscEvent::CommandLine("pi".to_owned()));
        meta.apply_osc_event(&OscEvent::CommandStart);
        meta.apply_osc_event(&OscEvent::PromptReady);

        let bytes = encode_meta_preamble(&meta);
        let events = OscScanner::new().feed(&bytes);

        assert!(
            matches!(
                events.as_slice(),
                [
                    OscEvent::CommandLine(cmd),
                    OscEvent::CommandStart,
                    OscEvent::PromptReady,
                ] if cmd == "pi"
            ),
            "latched command must replay so a fresh client reseeds agent identity: {events:?}"
        );
    }

    #[test]
    fn initial_host_meta_uses_spawn_workdir_for_cwd() {
        let opts = SpawnOptions {
            workdir: Some(PathBuf::from("/repo/project")),
            launch_cmd: Some("claude --resume session".to_owned()),
            color_scheme: None,
            env: Vec::new(),
        };

        assert_eq!(
            initial_host_meta(&opts).cwd.as_deref(),
            Some("/repo/project")
        );
        assert_eq!(
            initial_host_meta(&opts).current_command.as_deref(),
            Some("claude --resume session")
        );
        assert_eq!(
            initial_host_meta(&opts).shell_state,
            TermShellState::Running
        );
    }

    #[test]
    fn color_query_responder_answers_light_default_queries() {
        let mut responder = TerminalColorQueryResponder::new(TerminalColorScheme::Light);
        let replies = responder.feed(b"\x1b]10;?\x07\x1b]11;?\x1b\\");

        assert_eq!(
            replies,
            vec![
                b"\x1b]10;rgb:1f1f/2323/2828\x07".to_vec(),
                b"\x1b]11;rgb:fafa/fafa/fafa\x07".to_vec(),
            ]
        );
    }

    #[test]
    fn color_query_responder_handles_chunked_queries_and_ignores_setters() {
        let mut responder = TerminalColorQueryResponder::new(TerminalColorScheme::Dark);
        assert!(responder.feed(b"\x1b]11").is_empty());
        assert!(responder.feed(b";#112233\x07").is_empty());
        assert!(responder.feed(b"\x1b]12").is_empty());
        let replies = responder.feed(b";?\x07");

        assert_eq!(replies, vec![b"\x1b]12;rgb:5252/8b8b/ffff\x07".to_vec()]);
    }
}

#[cfg(test)]
mod file_search_tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn file_search_finds_names_below_root_and_respects_gitignore() {
        let temp = tempfile::tempdir().unwrap();
        create_dir_all(temp.path().join(".git")).unwrap();
        create_dir_all(temp.path().join("src/nested")).unwrap();
        create_dir_all(temp.path().join("ignored")).unwrap();
        write(temp.path().join(".gitignore"), "ignored/\n").unwrap();
        write(temp.path().join("src/search_panel.rs"), "match").unwrap();
        write(temp.path().join("src/nested/other.rs"), "skip").unwrap();
        write(temp.path().join("ignored/search_hidden.rs"), "skip").unwrap();

        let result = search_files(&temp.path().canonicalize().unwrap(), "SEARCH", 10).unwrap();

        let hit = result
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("src/search_panel.rs"))
            .expect("expected search_panel.rs match");
        // Host-supplied match indices are sorted and reference rel_path.
        assert!(!hit.match_indices.is_empty());
        assert!(hit.match_indices.windows(2).all(|w| w[0] < w[1]));
        assert!(*hit.match_indices.last().unwrap() < hit.rel_path.chars().count() as u32);
        assert!(result
            .entries
            .iter()
            .all(|entry| !entry.path.contains("ignored/search_hidden.rs")));
        assert!(!result.truncated);
    }

    #[test]
    fn file_search_reports_truncation_when_limit_is_hit() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path().join("search_a.rs"), "a").unwrap();
        write(temp.path().join("search_b.rs"), "b").unwrap();

        let result = search_files(&temp.path().canonicalize().unwrap(), "search", 1).unwrap();

        assert_eq!(result.entries.len(), 1);
        assert!(result.truncated);
    }

    #[test]
    fn file_search_discovers_git_worktrees_even_when_gitignored() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();

        // Init a real git repo so `git worktree add` works.
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init"])
            .output()
            .expect("git init failed");
        assert!(
            init.status.success(),
            "git init: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        // Configure git identity (needed in CI environments without global config).
        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let cfg = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", key, val])
                .output()
                .expect("git config failed");
            assert!(cfg.status.success());
        }

        // Commit something so HEAD exists (worktree add needs a valid HEAD).
        write(root.join("README.md"), "# hello").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "README.md"])
            .output()
            .expect("git add failed");
        assert!(add.status.success());
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "init", "--no-gpg-sign"])
            .output()
            .expect("git commit failed");
        assert!(
            commit.status.success(),
            "git commit: {}",
            String::from_utf8_lossy(&commit.stderr)
        );

        // Gitignore the directory that will hold the worktree.
        write(root.join(".gitignore"), ".claude/\n").unwrap();

        // Add the gitignore to the index so the ignore rule is active.
        let add_gi = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", ".gitignore"])
            .output()
            .expect("git add .gitignore failed");
        assert!(add_gi.status.success());
        let commit_gi = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "add gitignore", "--no-gpg-sign"])
            .output()
            .expect("git commit gitignore failed");
        assert!(commit_gi.status.success());

        // Create a worktree inside the gitignored path.
        let wt_parent = root.join(".claude/worktrees");
        create_dir_all(&wt_parent).unwrap();
        let wt_path = wt_parent.join("wt");
        let wt = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "worktree",
                "add",
                "-b",
                "feat-search",
                wt_path.to_str().unwrap(),
            ])
            .output()
            .expect("git worktree add failed");
        assert!(
            wt.status.success(),
            "git worktree add: {}",
            String::from_utf8_lossy(&wt.stderr)
        );

        // Write a uniquely-named file in the worktree.
        write(wt_path.join("wt_unique_file.rs"), "// unique").unwrap();

        let result = search_files(&root, "wt_unique", 10).unwrap();

        let hit = result
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("wt_unique_file.rs"))
            .expect("expected wt_unique_file.rs to be found in gitignored worktree");
        assert!(hit.match_indices.windows(2).all(|w| w[0] < w[1]));
        // `worktree` carries the branch name; `rel_path` is worktree-relative.
        assert_eq!(hit.worktree.as_deref(), Some("feat-search"));
        assert_eq!(hit.rel_path, "wt_unique_file.rs");
    }

    #[test]
    fn file_search_ignores_worktrees_outside_root() {
        use std::fs::{create_dir_all, write};
        use tempfile::tempdir;

        let tmp_dir = tempdir().unwrap();
        let root = tmp_dir.path().join("repo");
        create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init"])
            .output()
            .expect("git init failed");
        assert!(init.status.success());

        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let cfg = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", key, val])
                .output()
                .expect("git config failed");
            assert!(cfg.status.success());
        }

        write(root.join("README.md"), "# hello").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "README.md"])
            .output()
            .expect("git add failed");
        assert!(add.status.success());
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "init", "--no-gpg-sign"])
            .output()
            .expect("git commit failed");
        assert!(commit.status.success());

        // Create a worktree OUTSIDE the search root (sibling directory).
        let outside_wt = tmp_dir.path().join("outside_wt");
        create_dir_all(&outside_wt).unwrap();
        let wt = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "add", outside_wt.to_str().unwrap()])
            .output()
            .expect("git worktree add failed");
        assert!(
            wt.status.success(),
            "git worktree add: {}",
            String::from_utf8_lossy(&wt.stderr)
        );

        write(outside_wt.join("outside_secret.rs"), "// secret").unwrap();

        let result = search_files(&root, "outside_secret", 10).unwrap();

        assert!(
            !result
                .entries
                .iter()
                .any(|entry| entry.path.ends_with("outside_secret.rs")),
            "files from a worktree outside root must NOT appear in search results"
        );
    }

    #[test]
    fn file_search_match_path_uses_longest_prefix_for_nested_worktrees() {
        use std::fs::{create_dir_all, write};
        use tempfile::tempdir;

        let tmp_dir = tempdir().unwrap();
        let root = tmp_dir.path().join("repo");
        create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init"])
            .output()
            .expect("git init failed");
        assert!(init.status.success());

        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let cfg = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", key, val])
                .output()
                .expect("git config failed");
            assert!(cfg.status.success());
        }

        write(root.join("README.md"), "# hello").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "README.md"])
            .output()
            .expect("git add failed");
        assert!(add.status.success());
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "init", "--no-gpg-sign"])
            .output()
            .expect("git commit failed");
        assert!(commit.status.success());

        // Gitignore the worktree parent directory.
        write(root.join(".gitignore"), "wt/\n").unwrap();
        let add_gi = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", ".gitignore"])
            .output()
            .expect("git add .gitignore failed");
        assert!(add_gi.status.success());
        let commit_gi = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "add gitignore", "--no-gpg-sign"])
            .output()
            .expect("git commit gitignore failed");
        assert!(commit_gi.status.success());

        // Create a worktree nested under root.
        let wt_parent = root.join("wt");
        create_dir_all(&wt_parent).unwrap();
        let wt_path = wt_parent.join("nested");
        let wt = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "add", wt_path.to_str().unwrap()])
            .output()
            .expect("git worktree add failed");
        assert!(
            wt.status.success(),
            "git worktree add: {}",
            String::from_utf8_lossy(&wt.stderr)
        );

        write(wt_path.join("nested_file.rs"), "// nested").unwrap();

        let result = search_files(&root, "nested_file", 10).unwrap();

        let hit = result
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("nested_file.rs"))
            .expect("expected nested_file.rs to be found");

        // match_path must be worktree-relative (just the file name), not root-relative
        // (which would include the ignored "wt/nested/" prefix).
        assert!(
            !hit.rel_path.starts_with("wt/"),
            "rel_path for a nested worktree file must NOT start with the ignored parent path; got: {}",
            hit.rel_path
        );
        assert_eq!(hit.worktree.as_deref(), Some("nested"));
    }

    #[test]
    fn file_search_does_not_duplicate_non_ignored_nested_worktree() {
        use std::fs::{create_dir_all, write};
        use tempfile::tempdir;

        let tmp_dir = tempdir().unwrap();
        let root = tmp_dir.path().join("repo");
        create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init"])
            .output()
            .expect("git init failed");
        assert!(init.status.success());

        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let cfg = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", key, val])
                .output()
                .expect("git config failed");
            assert!(cfg.status.success());
        }

        write(root.join("README.md"), "# hello").unwrap();
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "README.md"])
            .output()
            .expect("git add failed");
        assert!(add.status.success());
        let commit = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", "init", "--no-gpg-sign"])
            .output()
            .expect("git commit failed");
        assert!(commit.status.success());

        // Non-gitignored nested worktree: reachable via the main walk AND its added root.
        let wt_path = root.join("visible_wt");
        let wt = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "add", wt_path.to_str().unwrap()])
            .output()
            .expect("git worktree add failed");
        assert!(
            wt.status.success(),
            "git worktree add: {}",
            String::from_utf8_lossy(&wt.stderr)
        );

        write(wt_path.join("dup_probe_file.rs"), "// probe").unwrap();

        let result = search_files(&root, "dup_probe", 10).unwrap();

        let hits: Vec<_> = result
            .entries
            .iter()
            .filter(|entry| entry.path.ends_with("dup_probe_file.rs"))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "file in a nested worktree must appear exactly once"
        );
        assert_eq!(hits[0].worktree.as_deref(), Some("visible_wt"));
    }
}

#[allow(unused)]
fn short_key(key: &[u8; 32]) -> String {
    key[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Snapshot the iroh connection path type at the current moment.
/// Returns "direct" for P2P, "relay" for relay-only, "unknown" if undetermined.
fn initial_path_type(conn: &iroh::endpoint::Connection) -> &'static str {
    use iroh::Watcher;
    let mut paths = conn.paths();
    let path_list = paths.get();
    let result = path_list
        .iter()
        .find(|p| p.is_selected())
        .map(|p| if p.is_ip() { "direct" } else { "relay" })
        .unwrap_or("unknown");
    drop(path_list);
    result
}

fn connection_latency_sample(
    path: &iroh::endpoint::PathInfo,
    path_count: usize,
    interval_secs: u64,
) -> Event {
    let (connection_type, network_type, relay, relay_region, nearest_relay_region) =
        match path.remote_addr() {
            iroh::TransportAddr::Ip(addr) => (
                "p2p",
                zedra_telemetry::ip_network_type(addr.ip()),
                "none",
                "none",
                "unknown",
            ),
            iroh::TransportAddr::Relay(url) => {
                let relay = url.host_str().unwrap_or(url.as_str());
                let relay_id = zedra_telemetry::relay_id_label(relay);
                let relay_region = zedra_telemetry::relay_region_label(relay);
                ("relay", "relay", relay_id, relay_region, relay_region)
            }
            _ => ("unknown", "unknown", "none", "unknown", "unknown"),
        };

    Event::ConnectionLatencySample {
        source: "host",
        connection_type,
        network_type,
        rtt_ms: path.stats().rtt.as_millis() as u64,
        relay,
        relay_region,
        nearest_relay_region,
        path_count,
        interval_secs,
        sample_reason: "periodic",
    }
}

/// Resolve `user_path` relative to `workdir`, then verify the canonical path
/// stays inside `workdir`. Rejects absolute paths, `..` escapes, and symlinks
/// that point outside the jail.
pub(crate) fn resolve_path(workdir: &Path, user_path: &str) -> Result<PathBuf> {
    // Reject empty paths
    anyhow::ensure!(!user_path.is_empty(), "empty path");
    let joined = workdir.join(user_path);
    let resolved = joined.canonicalize().or_else(|_| {
        // File may not exist yet (e.g. FsWrite to a new path).
        // Walk up to the first existing ancestor and canonicalize that.
        let mut base = joined.as_path();
        while let Some(parent) = base.parent() {
            if parent.exists() {
                let canon = parent.canonicalize()?;
                // Reconstruct: canon + the non-existing tail
                let tail = joined.strip_prefix(parent).unwrap_or(base);
                return Ok(canon.join(tail));
            }
            base = parent;
        }
        anyhow::bail!("could not resolve path");
    })?;
    let jail = workdir.canonicalize()?;
    anyhow::ensure!(
        resolved.starts_with(&jail),
        "path {} escapes workspace {}",
        resolved.display(),
        jail.display(),
    );
    Ok(resolved)
}

/// Normalize a client-provided observer path into a canonical relative key.
/// Returns `None` for invalid input (absolute paths or parent traversal).
fn normalize_observer_path(path: &str) -> Option<String> {
    let raw = path.trim();
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(out.to_string_lossy().into_owned())
    }
}

fn git_status_fingerprint(workdir: &Path) -> Option<u64> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workdir)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("--branch")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    output.stdout.hash(&mut hasher);
    Some(hasher.finish())
}

fn fs_dir_fingerprint(workdir: &Path, rel_path: &str) -> Option<u64> {
    let target = resolve_path(workdir, rel_path).ok()?;
    let entries = std::fs::read_dir(target).ok()?;
    // File explorer invalidation should track tree shape changes only.
    // Including mtime/size causes noisy false positives (for example `.git`
    // metadata churn) that collapse expanded directories via root reload.
    let mut rows: Vec<(String, bool)> = Vec::new();
    for entry in entries.flatten() {
        let meta = entry.metadata().ok()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        rows.push((name, is_dir));
    }
    rows.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    Some(hasher.finish())
}

fn fs_search_limit(limit: u32) -> usize {
    (if limit == 0 {
        FS_SEARCH_DEFAULT_LIMIT
    } else {
        limit.min(FS_SEARCH_MAX_LIMIT)
    }) as usize
}

/// A linked git worktree nested under the search root.
struct DiscoveredWorktree {
    path: PathBuf,
    label: String,
}

/// Worktrees nested under root, from `git worktree list --porcelain`; empty if git fails.
fn discover_git_worktrees(root: &Path) -> Vec<DiscoveredWorktree> {
    let output = match std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output);
    let mut worktrees = Vec::new();
    // Porcelain output is one blank-line-separated block per worktree.
    for block in text.split("\n\n") {
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        for line in block.lines() {
            if let Some(path_str) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(path_str));
            } else if let Some(branch_ref) = line.strip_prefix("branch ") {
                branch = Some(
                    branch_ref
                        .strip_prefix("refs/heads/")
                        .unwrap_or(branch_ref)
                        .to_string(),
                );
            }
        }
        let Some(path) = path else { continue };
        // Skip the main worktree (duplicate results) and outside-root worktrees (sibling leak).
        if path != root && path.starts_with(root) {
            let label = branch.unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            worktrees.push(DiscoveredWorktree { path, label });
        }
    }
    worktrees
}

fn is_file_search_ignored(entry: &ignore::DirEntry) -> bool {
    // Only filter directories; matched files should still surface as results.
    if !entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
    {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    // Reuse the canonical generated/vendor directory list shared with the docs
    // tree so both walkers skip the same noise. Unlike the docs tree, file
    // search keeps dot-directories (e.g. `.github`) visible.
    crate::docs_tree::FALLBACK_COMPONENT_IGNORES.contains(&name)
}

struct FileSearchCandidate {
    path: PathBuf,
    is_dir: bool,
    match_path: String,
    worktree: Option<String>,
}

fn fs_search_error(message: String) -> FsSearchResult {
    FsSearchResult {
        entries: vec![],
        truncated: false,
        error: Some(message),
    }
}

fn search_files(root: &Path, query: &str, limit: u32) -> Result<FsSearchResult> {
    let query = query.trim();
    anyhow::ensure!(!query.is_empty(), "empty search query");
    anyhow::ensure!(root.is_dir(), "search path must be a directory");

    let limit = fs_search_limit(limit);
    let mut candidates = Vec::new();
    let mut visited = 0u32;
    let mut truncated = false;

    let worktrees = discover_git_worktrees(root);

    let mut builder = ignore::WalkBuilder::new(root);
    for wt in &worktrees {
        builder.add(&wt.path);
    }
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .ignore(true);
    let worktree_paths: Vec<PathBuf> = worktrees.iter().map(|wt| wt.path.clone()).collect();
    builder.filter_entry(move |entry| {
        // Prune nested worktree copies from the main walk; added roots are exempt (filters skip depth 0).
        if worktree_paths.iter().any(|wt| entry.path() == wt) {
            return false;
        }
        !is_file_search_ignored(entry)
    });

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!("file search: skipping unreadable entry: {error}");
                continue;
            }
        };
        // Depth 0 = a walk root (search root or worktree root); not a result row.
        if entry.depth() == 0 {
            continue;
        }

        visited += 1;
        if visited > FS_SEARCH_MAX_VISITED_ENTRIES {
            truncated = true;
            break;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path().to_path_buf();
        // Innermost containing worktree wins so nested worktrees are handled correctly.
        let owner_worktree = worktrees
            .iter()
            .filter(|wt| path.starts_with(&wt.path))
            .max_by_key(|wt| wt.path.as_os_str().len());
        let (prefix, worktree) = match owner_worktree {
            Some(wt) => (wt.path.as_path(), Some(wt.label.clone())),
            None => (root, None),
        };
        let match_path = path
            .strip_prefix(prefix)
            .unwrap_or(path.as_path())
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        candidates.push(FileSearchCandidate {
            path,
            is_dir: file_type.is_dir(),
            match_path,
            worktree,
        });
    }

    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut match_buf = Vec::new();
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            match_buf.clear();
            let haystack = Utf32Str::new(candidate.match_path.as_str(), &mut match_buf);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.match_path.cmp(&right.match_path))
    });

    if ranked.len() > limit {
        truncated = true;
    }

    // Recompute match indices only for the rows we return so the client can
    // highlight exactly the characters the host scored.
    let mut indices = Vec::new();
    let entries = ranked
        .into_iter()
        .take(limit)
        .map(|(_, candidate)| {
            match_buf.clear();
            let haystack = Utf32Str::new(candidate.match_path.as_str(), &mut match_buf);
            indices.clear();
            pattern.indices(haystack, &mut matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();
            FsSearchEntry {
                path: candidate.path.to_string_lossy().into_owned(),
                rel_path: candidate.match_path.clone(),
                is_dir: candidate.is_dir,
                match_indices: indices.clone(),
                worktree: candidate.worktree.clone(),
            }
        })
        .collect();

    Ok(FsSearchResult {
        entries,
        truncated,
        error: None,
    })
}

async fn run_observer(session: Arc<ServerSession>, workdir: PathBuf, my_gen: u64) {
    let mut last_git: Option<u64> = None;
    let mut fs_snapshots: HashMap<String, u64> = HashMap::new();
    let mut tick_count: u64 = 0;
    loop {
        let current = session.observer_gen.load(Ordering::Acquire);
        if current != my_gen {
            break;
        }

        if let Ok(Some(git_hash)) = tokio::task::spawn_blocking({
            let workdir = workdir.clone();
            move || git_status_fingerprint(&workdir)
        })
        .await
        {
            if last_git.is_some() && last_git != Some(git_hash) {
                let _ = session.push_event(HostEvent::GitChanged).await;
            }
            last_git = Some(git_hash);
        }

        let watched: Vec<String> = {
            let set = session.fs_watched_paths.lock().await;
            set.iter().cloned().collect()
        };
        let watched_len = watched.len();

        let mut retained: HashMap<String, u64> = HashMap::new();
        for path in watched {
            let fingerprint = match tokio::task::spawn_blocking({
                let workdir = workdir.clone();
                let path_clone = path.clone();
                move || fs_dir_fingerprint(&workdir, &path_clone)
            })
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("fs_dir_fingerprint error for path {}: {}", path, e);
                    None
                }
            };
            let Some(next_hash) = fingerprint else {
                continue;
            };
            if let Some(prev_hash) = fs_snapshots.get(&path) {
                if *prev_hash != next_hash {
                    let _ = session
                        .push_event(HostEvent::FsChanged { path: path.clone() })
                        .await;
                }
            }
            retained.insert(path, next_hash);
        }
        fs_snapshots = retained;

        tick_count += 1;
        if tick_count.is_multiple_of(30) {
            tracing::info!(
                "observer metrics: session={} watched={} sent={} dropped_full={} dropped_no_subscriber={} rate_limited={} quota_rejected={}",
                session.id,
                watched_len,
                session.observer_events_sent.load(Ordering::Relaxed),
                session.observer_events_dropped_full.load(Ordering::Relaxed),
                session
                    .observer_events_dropped_no_subscriber
                    .load(Ordering::Relaxed),
                session.fs_watch_rate_limited.load(Ordering::Relaxed),
                session.fs_watch_quota_rejected.load(Ordering::Relaxed),
            );
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Shared state for RPC handlers.
pub struct DaemonState {
    pub fs: Arc<dyn Filesystem>,
    pub workdir: std::path::PathBuf,
    /// Host identity for signing challenges in the Authenticate step.
    pub identity: SharedIdentity,
    /// Dedicated host Delta node authorization public key.
    pub delta_pubkey: [u8; 32],
    /// When the daemon started; used to compute uptime.
    pub started_at: std::time::Instant,
    pub agent_cache: Arc<agent_cache::AgentCache>,
    /// Delta client; updated at runtime when a mobile client reports its
    /// Delta info via `SetClientDeltaInfo`. `None` if Delta is not configured.
    pub delta: Arc<tokio::sync::RwLock<Option<Arc<crate::delta::DeltaClient>>>>,
}

impl std::fmt::Debug for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonState")
            .field("workdir", &self.workdir)
            .finish_non_exhaustive()
    }
}

impl DaemonState {
    pub fn new(
        workdir: std::path::PathBuf,
        identity: SharedIdentity,
        delta_pubkey: [u8; 32],
        delta: Option<Arc<crate::delta::DeltaClient>>,
    ) -> Self {
        Self {
            fs: Arc::new(LocalFs),
            workdir,
            identity,
            delta_pubkey,
            started_at: std::time::Instant::now(),
            agent_cache: agent_cache::AgentCache::new(),
            delta: Arc::new(tokio::sync::RwLock::new(delta)),
        }
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

/// Handle a single iroh connection using the irpc protocol.
///
/// Auth phase: optional Register, then Authenticate → AuthProve.
/// After successful auth, enters the RPC dispatch loop.
pub async fn handle_connection(
    conn: iroh::endpoint::Connection,
    registry: Arc<SessionRegistry>,
    state: Arc<DaemonState>,
) -> Result<()> {
    let remote = conn.remote_id();
    tracing::info!(
        "connection from {} (alpn={})",
        remote.fmt_short(),
        String::from_utf8_lossy(conn.alpn()),
    );

    // Auth phase: returns (session, client_pubkey, is_new_client) or closes connection
    let auth_start = std::time::Instant::now();
    let path_type = initial_path_type(&conn);
    let mut failure_reason: &'static str = "io_error";
    let mut failure_is_new_client = false;
    let (session, client_pubkey, active_connection, is_new_client, auth_timing) = match auth_phase(
        &conn,
        &registry,
        &state,
        &mut failure_reason,
        &mut failure_is_new_client,
    )
    .await
    {
        Ok(quad) => quad,
        Err(e) => {
            zedra_telemetry::send(Event::AuthFailed {
                reason: failure_reason,
                elapsed_ms: auth_start.elapsed().as_millis() as u64,
                is_new_client: failure_is_new_client,
                path_type,
            });
            tracing::warn!("auth failed from {}: {}", remote.fmt_short(), e);
            finish_auth_failed_connection(&conn).await;
            return Ok(());
        }
    };

    zedra_telemetry::send(Event::AuthSuccess {
        is_new_client,
        register_ms: auth_timing.register_ms,
        challenge_ms: auth_timing.challenge_ms,
        prove_ms: auth_timing.prove_ms,
        total_ms: auth_start.elapsed().as_millis() as u64,
        path_type,
    });

    tracing::info!(
        "Authenticated client {:?}... → session={}",
        &client_pubkey[..4],
        session.id,
    );
    let monitor_task = active_connection.spawn_monitor();
    let metrics_connection_open = match metrics::record_connection_opened(&state.workdir) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("Failed to record connection metrics: {}", e);
            false
        }
    };
    if is_new_client {
        if let Err(e) = metrics::record_pairing_completed(&state.workdir) {
            tracing::warn!("Failed to record pairing metrics: {}", e);
        }
    }

    let session_start = std::time::Instant::now();

    // Spawn bandwidth sampler: reads iroh path stats every 60s while connected.
    let bandwidth_task = {
        use iroh::Watcher;
        let conn_for_bw = conn.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await; // skip immediate first tick
            let mut paths = conn_for_bw.paths(); // hold watcher for the lifetime of the task
            let mut prev_tx: u64 = 0;
            let mut prev_rx: u64 = 0;
            const SAMPLE_INTERVAL_SECS: u64 = 60;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if conn_for_bw.close_reason().is_some() {
                            break;
                        }
                        let path_list = paths.get();
                        let mut cur_tx = 0u64;
                        let mut cur_rx = 0u64;
                        let mut bw_found = false;
                        let mut latency_sample = None;
                        for p in path_list.iter() {
                            if p.is_selected() {
                                let s = p.stats();
                                cur_tx = s.udp_tx.bytes;
                                cur_rx = s.udp_rx.bytes;
                                bw_found = true;
                                latency_sample = Some(connection_latency_sample(
                                    p,
                                    path_list.len(),
                                    SAMPLE_INTERVAL_SECS,
                                ));
                                break;
                            }
                        }
                        drop(path_list);
                        if bw_found {
                            let delta_tx = cur_tx.saturating_sub(prev_tx);
                            let delta_rx = cur_rx.saturating_sub(prev_rx);
                            prev_tx = cur_tx;
                            prev_rx = cur_rx;
                            zedra_telemetry::send(Event::BandwidthSample {
                                bytes_sent: delta_tx,
                                bytes_recv: delta_rx,
                                interval_secs: SAMPLE_INTERVAL_SECS,
                            });
                        }
                        if let Some(sample) = latency_sample {
                            zedra_telemetry::send(sample);
                        }
                    }
                    _ = conn_for_bw.closed() => break,
                }
            }
        })
    };

    // RPC dispatch loop. Treat decode failures (newer-client variant /
    // extended struct) as per-stream, not connection-level: breaking here
    // would brick every other in-flight RPC on the same QUIC connection.
    loop {
        match read_zedra_message(&conn).await {
            Ok(Some(msg)) => {
                let s = session.clone();
                let st = state.clone();
                let r = registry.clone();
                let cpk = client_pubkey;
                let active_connection_id = active_connection.id();
                tokio::spawn(async move {
                    if let Err(e) = dispatch(msg, s, st, r, cpk, active_connection_id).await {
                        tracing::warn!("dispatch error: {}", e);
                    }
                });
            }
            Ok(None) => break,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                // Detailed per-variant log already emitted in read_zedra_message.
                continue;
            }
            Err(e) => {
                tracing::debug!("read_request error: {}", e);
                break;
            }
        }
    }

    // Cleanup on disconnect.
    // clear_output_senders() is intentionally NOT called here: the TermAttach
    // cleanup above guards its None-set with a generation check, and the PTY
    // reader task self-heals by clearing a dead sender on the next write attempt.
    // Calling it here would race with a concurrent new TermAttach and silence
    // the new client's output.
    let session_duration_ms = session_start.elapsed().as_millis() as u64;
    let terminal_count = session.terminals.lock().await.len() as u64;
    zedra_telemetry::send(Event::SessionEnd {
        duration_ms: session_duration_ms,
        terminal_count,
        path_type,
        fs_reads: session.rpc_fs_reads.load(Ordering::Relaxed),
        fs_writes: session.rpc_fs_writes.load(Ordering::Relaxed),
        git_ops: session.rpc_git_ops.load(Ordering::Relaxed),
        git_commits: session.rpc_git_commits.load(Ordering::Relaxed),
        ai_prompts: session.rpc_ai_prompts.load(Ordering::Relaxed),
    });

    registry
        .detach_client(&session.id, client_pubkey, active_connection.id())
        .await;
    if metrics_connection_open {
        if let Err(e) = metrics::record_connection_closed(&state.workdir) {
            tracing::warn!("Failed to record connection metrics: {}", e);
        }
    }

    if let Some(monitor_task) = monitor_task {
        monitor_task.abort();
    }
    bandwidth_task.abort();
    finish_host_connection(&conn).await;

    tracing::info!(
        "Connection closed: session={} (session stays alive in registry)",
        session.id,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Auth phase
// ---------------------------------------------------------------------------

struct AuthTiming {
    register_ms: u64,
    challenge_ms: u64,
    prove_ms: u64,
}

/// Perform the full auth handshake for a new connection.
///
/// Flow:
///   1. Optional Register (first-time only, proves QR possession via HMAC)
///   2. Connect — universal initiator for all non-Register paths:
///      - session_token present and valid → Ok(SyncSessionResult) fast path
///      - otherwise → Challenge (nonce + host_sig embedded, saves Authenticate RTT)
///   3. AuthProve (client signs nonce, specifies session to attach)
///      → Ok(SyncSessionResult) (bootstrap data piggybacked, no SyncSession needed)
async fn auth_phase(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    state: &Arc<DaemonState>,
    failure_reason: &mut &'static str,
    failure_is_new_client: &mut bool,
) -> Result<(
    Arc<ServerSession>,
    [u8; 32],
    ActiveClientConnection,
    bool,
    AuthTiming,
)> {
    let first = read_zedra_message(conn).await?;

    match first {
        Some(ZedraMessage::Register(msg)) => {
            // First pairing: verify HMAC, consume slot, add to ACL.
            // After success, expect Connect (which will always issue a Challenge
            // since no session_token exists yet for a brand-new client).
            *failure_is_new_client = true;
            let t = std::time::Instant::now();
            let result = handle_register(&msg, registry).await;
            let ok = matches!(result, RegisterResult::Ok);
            let register_ms = t.elapsed().as_millis() as u64;
            *failure_reason = match &result {
                RegisterResult::StaleTimestamp => "stale_timestamp",
                RegisterResult::InvalidHandshake => "bad_hmac",
                RegisterResult::HandshakeConsumed => "slot_consumed",
                RegisterResult::SlotNotFound => "slot_not_found",
                RegisterResult::Ok => "io_error",
            };
            let _ = msg.tx.send(result).await;
            if !ok {
                anyhow::bail!("register rejected");
            }
            let connect_msg = read_zedra_message(conn).await?;
            match connect_msg {
                Some(ZedraMessage::Connect(msg)) => {
                    // is_new_client = true: came through the Register path
                    let t_connect = std::time::Instant::now();
                    let (session, pubkey, active_connection, is_new, prove_ms) =
                        handle_connect(msg, conn, registry, state, true, failure_reason).await?;
                    Ok((
                        session,
                        pubkey,
                        active_connection,
                        is_new,
                        AuthTiming {
                            register_ms,
                            challenge_ms: t_connect.elapsed().as_millis() as u64,
                            prove_ms,
                        },
                    ))
                }
                _ => {
                    *failure_reason = "unexpected_message";
                    anyhow::bail!("expected Connect after Register")
                }
            }
        }
        Some(ZedraMessage::Connect(msg)) => {
            // PKI reconnect or token resume — is_new_client = false
            let t = std::time::Instant::now();
            let (session, pubkey, active_connection, is_new, prove_ms) =
                handle_connect(msg, conn, registry, state, false, failure_reason).await?;
            Ok((
                session,
                pubkey,
                active_connection,
                is_new,
                AuthTiming {
                    register_ms: 0,
                    challenge_ms: t.elapsed().as_millis() as u64,
                    prove_ms,
                },
            ))
        }
        _ => {
            *failure_reason = "unexpected_message";
            anyhow::bail!("expected Register or Connect as first message")
        }
    }
}

/// Process a `Connect` message. If the client presents a valid session_token,
/// attach immediately and return Ok with SyncSessionResult. Otherwise, issue a
/// Challenge (nonce + host_signature) and wait for AuthProve.
/// Returns (session, pubkey, active connection, is_new_client, prove_ms).
async fn handle_connect(
    msg: irpc::WithChannels<ConnectReq, ZedraProto>,
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    state: &Arc<DaemonState>,
    is_new_client: bool,
    failure_reason: &mut &'static str,
) -> Result<(
    Arc<ServerSession>,
    [u8; 32],
    ActiveClientConnection,
    bool,
    u64,
)> {
    let pubkey = msg.client_pubkey;
    let session_id = msg.session_id.clone();

    // Fast path: try session token if client provided one.
    if let Some(token) = msg.session_token {
        let session = registry.get(&session_id).await;
        if let Some(ref session) = session {
            if session.validate_session_token(&pubkey, &token).await {
                let active_connection = ActiveClientConnection::new(pubkey, conn.clone());
                match registry
                    .attach_client(&session_id, active_connection.clone())
                    .await
                {
                    AttachResult::Ok { previous } => {
                        if let Some(previous) = previous {
                            previous.close_for_takeover();
                        }
                        let new_token = session.issue_session_token(pubkey).await;
                        let sync = build_sync_result(session, state, new_token).await;
                        let _ = msg.tx.send(ConnectResult::Ok(sync)).await;
                        // Token fast-path: no challenge/prove round trip, prove_ms=0
                        return Ok((session.clone(), pubkey, active_connection, false, 0));
                    }
                    AttachResult::NotInSessionAcl => {
                        *failure_reason = "not_in_session_acl";
                        let _ = msg.tx.send(ConnectResult::NotInSessionAcl).await;
                        anyhow::bail!("client not in session ACL");
                    }
                    AttachResult::SessionOccupied => {
                        *failure_reason = "session_occupied";
                        let _ = msg.tx.send(ConnectResult::SessionOccupied).await;
                        anyhow::bail!("session {} is occupied", session_id);
                    }
                    AttachResult::SessionNotFound => {
                        // Fall through to PKI challenge below
                    }
                }
            }
        }
        // Token invalid/expired or session not found — fall through to challenge
    }

    // Check global authorization before issuing a challenge.
    if !is_new_client && !registry.is_globally_authorized(&pubkey).await {
        *failure_reason = "not_authorized";
        let _ = msg.tx.send(ConnectResult::Unauthorized).await;
        anyhow::bail!("client not globally authorized");
    }

    // Issue challenge (nonce + host signature) embedded in ConnectResult::Challenge,
    // saving the separate Authenticate round trip.
    let mut nonce = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let host_signature = state.identity.sign_challenge(&nonce);
    let _ = msg
        .tx
        .send(ConnectResult::Challenge {
            nonce,
            host_signature,
        })
        .await;

    finish_auth(
        conn,
        registry,
        pubkey,
        nonce,
        state,
        is_new_client,
        failure_reason,
    )
    .await
}

/// Handle a Register request: verify HMAC, consume slot, add to ACL.
async fn handle_register(
    msg: &irpc::WithChannels<RegisterReq, ZedraProto>,
    registry: &Arc<SessionRegistry>,
) -> RegisterResult {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Check timestamp (±60s window)
    if now.abs_diff(msg.timestamp) > 60 {
        tracing::warn!(
            "Register: stale timestamp (now={}, ts={})",
            now,
            msg.timestamp
        );
        return RegisterResult::StaleTimestamp;
    }

    // Atomically consume the pairing slot
    match registry.consume_pairing_slot(&msg.session_id).await {
        ConsumeSlotResult::Active(slot) => {
            // Verify HMAC (slot is already consumed regardless of outcome)
            if !zedra_rpc::verify_registration_hmac(
                &slot.handshake_secret,
                &msg.client_pubkey,
                msg.timestamp,
                &msg.hmac,
            ) {
                if slot.mode == PairingSlotMode::Static
                    && registry
                        .matches_superseded_pairing_hmac(
                            &msg.session_id,
                            &msg.client_pubkey,
                            msg.timestamp,
                            &msg.hmac,
                        )
                        .await
                {
                    tracing::warn!("Register: superseded QR for session {}", msg.session_id);
                    utils::eprintln_warn("QR expired or replaced. Generate a new one.");
                    return RegisterResult::SlotNotFound;
                }

                tracing::warn!(
                    "Register: invalid HMAC from {:?}...",
                    &msg.client_pubkey[..4]
                );
                utils::eprintln_warn("Invalid HMAC. Try again.");
                return RegisterResult::InvalidHandshake;
            }

            // Add to session ACL + global list
            registry
                .add_client_to_session(&slot.session_id, msg.client_pubkey)
                .await;

            tracing::info!(
                "Register: client {:?}... added to session {}",
                &msg.client_pubkey[..4],
                slot.session_id,
            );
            utils::eprintln_success(format!(
                "New device registered to session {}.",
                slot.session_id
            ));
            zedra_telemetry::send(Event::ClientPaired);
            RegisterResult::Ok
        }
        ConsumeSlotResult::Consumed => {
            // The slot may have been consumed by THIS client on an earlier
            // connection that dropped before the client observed the result.
            // Re-registration from a pubkey already in the ACL is idempotent:
            // report success instead of a fatal HandshakeConsumed. A pubkey
            // not in the ACL (e.g. the slot was burned by a bad-HMAC attempt)
            // still gets HandshakeConsumed.
            if registry
                .is_in_session_acl(&msg.session_id, &msg.client_pubkey)
                .await
            {
                tracing::info!(
                    "Register: client {:?}... already registered to session {}; idempotent ok",
                    &msg.client_pubkey[..4],
                    msg.session_id,
                );
                return RegisterResult::Ok;
            }
            tracing::warn!("Register: slot for {} already consumed", msg.session_id);
            utils::eprintln_warn(
                "QR already used. Run `zedra qr` from the workspace, or add `--workdir <path>` from another directory.",
            );
            RegisterResult::HandshakeConsumed
        }
        ConsumeSlotResult::NotFound => {
            tracing::warn!("Register: no slot found for session {}", msg.session_id);
            utils::eprintln_warn(
                "QR invalid, expired, or replaced. Run `zedra qr` from the workspace, or add `--workdir <path>` from another directory.",
            );
            RegisterResult::SlotNotFound
        }
    }
}

/// Read AuthProve, verify client signature, attach to session.
/// Returns (session, pubkey, active connection, is_new_client, prove_ms).
/// On success, sends `AuthProveResult::Ok(SyncSessionResult)` so the client
/// has everything it needs without a separate SyncSession round trip.
async fn finish_auth(
    conn: &iroh::endpoint::Connection,
    registry: &Arc<SessionRegistry>,
    client_pubkey: [u8; 32],
    nonce: [u8; 32],
    state: &Arc<DaemonState>,
    is_new_client: bool,
    failure_reason: &mut &'static str,
) -> Result<(
    Arc<ServerSession>,
    [u8; 32],
    ActiveClientConnection,
    bool,
    u64,
)> {
    let prove_start = std::time::Instant::now();
    let prove_msg = read_zedra_message(conn).await?;

    let msg = match prove_msg {
        Some(ZedraMessage::AuthProve(m)) => m,
        _ => {
            *failure_reason = "unexpected_message";
            anyhow::bail!("expected AuthProve")
        }
    };

    // Extract fields before any moves
    let prove_nonce = msg.nonce;
    let prove_sig = msg.client_signature;
    let session_id = msg.session_id.clone();
    let tx = msg.tx;

    // Verify nonce echo
    if prove_nonce != nonce {
        *failure_reason = "nonce_mismatch";
        let _ = tx.send(AuthProveResult::InvalidSignature).await;
        anyhow::bail!("AuthProve: nonce mismatch");
    }

    // Verify client signature of the nonce using stored pubkey
    {
        use ed25519_dalek::{Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&client_pubkey)
            .map_err(|e| anyhow::anyhow!("invalid client pubkey: {e}"))?;
        let sig = ed25519_dalek::Signature::from_bytes(&prove_sig);
        if vk.verify(&nonce, &sig).is_err() {
            *failure_reason = "invalid_signature";
            let _ = tx.send(AuthProveResult::InvalidSignature).await;
            anyhow::bail!("AuthProve: signature invalid");
        }
    }

    // Attach to the requested session, with fallback for stale session IDs
    // (e.g. after a daemon restart the client's stored session_id is gone).
    let active_connection = ActiveClientConnection::new(client_pubkey, conn.clone());
    let (attach_result, resolved_session_id) = match registry
        .attach_client(&session_id, active_connection.clone())
        .await
    {
        AttachResult::SessionNotFound => {
            // Client is globally authorized but their session was lost.
            // Try to find another session they have ACL for, or create one.
            let fallback = if let Some(s) = registry.find_session_for_client(&client_pubkey).await {
                tracing::info!(
                    "finish_auth: session {} gone, falling back to session {}",
                    session_id,
                    s.id,
                );
                s
            } else {
                // No existing session — create a fresh default one.
                let workdir = &state.workdir;
                let name = workdir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("default");
                let session_was_existing = registry.get_by_name(name).await.is_some();
                let s = registry.create_named(name, workdir.to_path_buf()).await;
                if !session_was_existing {
                    let session_count = registry.session_count().await;
                    if let Err(e) = metrics::record_session_created(workdir, session_count) {
                        tracing::warn!("Failed to record session metrics: {}", e);
                    }
                }
                registry.add_client_to_session(&s.id, client_pubkey).await;
                tracing::info!(
                    "finish_auth: session {} gone, created new session {} ({})",
                    session_id,
                    s.id,
                    name,
                );
                s
            };
            let new_id = fallback.id.clone();
            (
                registry
                    .attach_client(&new_id, active_connection.clone())
                    .await,
                new_id,
            )
        }
        other => (other, session_id.clone()),
    };

    match attach_result {
        AttachResult::Ok { previous } => {
            if let Some(previous) = previous {
                previous.close_for_takeover();
            }
            let Some(session) = registry.get(&resolved_session_id).await else {
                let _ = tx.send(AuthProveResult::SessionNotFound).await;
                anyhow::bail!("session {} vanished after attach", resolved_session_id);
            };
            let session_token = session.issue_session_token(client_pubkey).await;
            let sync = build_sync_result(&session, state, session_token).await;
            let _ = tx.send(AuthProveResult::Ok(sync)).await;
            Ok((
                session,
                client_pubkey,
                active_connection,
                is_new_client,
                prove_start.elapsed().as_millis() as u64,
            ))
        }
        AttachResult::SessionNotFound => {
            *failure_reason = "session_not_found";
            let _ = tx.send(AuthProveResult::SessionNotFound).await;
            anyhow::bail!("session {} not found", resolved_session_id)
        }
        AttachResult::NotInSessionAcl => {
            *failure_reason = "not_in_session_acl";
            let _ = tx.send(AuthProveResult::NotInSessionAcl).await;
            anyhow::bail!("client not in session ACL")
        }
        AttachResult::SessionOccupied => {
            *failure_reason = "session_occupied";
            let _ = tx.send(AuthProveResult::SessionOccupied).await;
            anyhow::bail!("session {} is occupied", resolved_session_id)
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal creation (shared by RPC dispatch and REST API)
// ---------------------------------------------------------------------------

pub const MAX_TERMINALS_PER_SESSION: usize = 16;

/// Spawn a new PTY shell and register it in `session`.
///
/// Returns the new terminal ID on success. Used by both the `TermCreate` RPC
/// handler and the local REST API so both paths share identical behaviour.
pub async fn create_terminal(
    session: &Arc<ServerSession>,
    cols: u16,
    rows: u16,
    mut opts: SpawnOptions,
) -> Result<String> {
    if session.terminals.lock().await.len() >= MAX_TERMINALS_PER_SESSION {
        anyhow::bail!(
            "session {} already has {} terminals (limit {})",
            session.id,
            MAX_TERMINALS_PER_SESSION,
            MAX_TERMINALS_PER_SESSION,
        );
    }

    let id = session.next_terminal_id().await;
    opts.env.push(("ZEDRA_TERMINAL_ID".to_string(), id.clone()));
    if let Some(workdir) = &opts.workdir {
        opts.env.push((
            "ZEDRA_WORKDIR".to_string(),
            workdir.to_string_lossy().into_owned(),
        ));
    }

    let color_scheme = opts.color_scheme.unwrap_or(TerminalColorScheme::Dark);
    let initial_meta = initial_host_meta(&opts);
    let shell = ShellSession::spawn(cols, rows, opts)?;
    let (pty_reader, pty_writer, master, child) = shell.take_reader();

    tracing::info!(
        "create_terminal: id={} cols={} rows={} session={}",
        id,
        cols,
        rows,
        session.id,
    );

    let output_sender = Arc::new(std::sync::Mutex::new(OutputSenderSlot {
        gen: 0,
        sender: None,
    }));
    let host_meta = Arc::new(std::sync::Mutex::new(initial_meta));
    let backlog = Arc::new(std::sync::Mutex::new(TermBacklog::new()));
    // Wrap the writer so TermAttach can hold a direct Arc clone and write
    // without locking session.terminals on every keystroke (Fix 3).
    let writer = Arc::new(std::sync::Mutex::new(pty_writer));
    let query_reply_writer = writer.clone();

    session
        .insert_terminal(
            id.clone(),
            TermSession {
                writer: writer.clone(),
                master,
                child,
                output_sender: output_sender.clone(),
                host_meta: host_meta.clone(),
                backlog: backlog.clone(),
                created_at: std::time::SystemTime::now(),
                started_at: std::time::Instant::now(),
            },
        )
        .await;

    let term_id = id.clone();
    // Captured so the blocking PTY reader can push host-resolved agent identity
    // changes back onto the session's event channel.
    let event_session = session.clone();
    let rt = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let mut reader = pty_reader;
        let mut buf = [0u8; 8192];
        let mut color_query_responder = TerminalColorQueryResponder::new(color_scheme);
        // Chunks that couldn't be sent (channel full) are held here and
        // coalesced with the next PTY read. This keeps the spawn_blocking
        // thread alive under QUIC back-pressure without blocking (Fix 2).
        let mut pending: Option<TermOutput> = None;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();

                    // Launch-command TUIs can query colors before a client TerminalView attaches.
                    // Answer these tiny OSC queries at the PTY boundary so startup style probes do
                    // not race the mobile render path.
                    let replies = color_query_responder.feed(&data);
                    if !replies.is_empty() {
                        if let Ok(mut writer) = query_reply_writer.lock() {
                            for reply in replies {
                                let _ = writer.write_all(&reply);
                            }
                            let _ = writer.flush();
                        }
                    }

                    // Scan for OSC sequences to keep the per-terminal metadata
                    // cache up to date. This runs on every PTY
                    // chunk so the host always has the latest values even after
                    // old backlog entries have been evicted.
                    let agent_identity_change = if let Ok(mut m) = host_meta.lock() {
                        let events = m.scanner.feed(&data);
                        // Identity is derived only from the foreground command and
                        // OSC 1 icon name, so skip the allocating re-resolve unless
                        // one of those changed (most chunks carry no OSC at all).
                        let identity_relevant = events.iter().any(|ev| {
                            matches!(
                                ev,
                                zedra_osc::OscEvent::CommandLine(_)
                                    | zedra_osc::OscEvent::IconName(_)
                                    | zedra_osc::OscEvent::CommandEnd { .. }
                            )
                        });
                        for ev in events {
                            m.apply_osc_event(&ev);
                        }
                        // Recompute identity under the same lock; emit only on change.
                        identity_relevant
                            .then(|| {
                                m.refresh_agent_slug()
                                    .then(|| m.agent_slug.map(str::to_string))
                            })
                            .flatten()
                    } else {
                        None
                    };
                    if let Some(agent_slug) = agent_identity_change {
                        let session = event_session.clone();
                        let terminal_id = term_id.clone();
                        rt.spawn(async move {
                            session
                                .push_event(HostEvent::TerminalAgentChanged {
                                    terminal_id,
                                    agent_slug,
                                })
                                .await;
                        });
                    }

                    // Push to per-terminal backlog (Fix 1: sync, no rt.block_on).
                    let seq = backlog.lock().unwrap().push(term_id.clone(), data.clone());

                    let sender: Option<tokio::sync::mpsc::Sender<TermOutput>> =
                        output_sender.lock().unwrap().sender.clone();
                    if let Some(tx) = sender {
                        // Coalesce new data with any previously unsent chunk,
                        // then attempt a non-blocking send (Fix 2).
                        let out = match pending.take() {
                            Some(mut p) => {
                                p.data.extend_from_slice(&data);
                                p.seq = seq;
                                p
                            }
                            None => TermOutput { data, seq },
                        };
                        match tx.try_send(out) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(ret)) => {
                                // Channel full (QUIC congested): hold for next iteration.
                                pending = Some(ret);
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                output_sender.lock().unwrap().sender = None;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("read PTY buffer error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(id)
}

// ---------------------------------------------------------------------------
// RPC dispatch
// ---------------------------------------------------------------------------

fn git_status_result(workdir: PathBuf) -> GitStatusResult {
    match GitRepo::open(&workdir) {
        Ok(repo) => {
            let branch = repo.branch().unwrap_or_default();
            let entries = repo
                .status()
                .unwrap_or_default()
                .into_iter()
                .map(|e| GitStatusEntry {
                    path: e.path,
                    staged_status: e
                        .staged_status
                        .map(|status| format!("{:?}", status).to_lowercase()),
                    unstaged_status: e
                        .unstaged_status
                        .map(|status| format!("{:?}", status).to_lowercase()),
                })
                .collect();
            GitStatusResult {
                branch,
                entries,
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!("GitStatus: failed to open repo at {:?}: {}", workdir, e);
            GitStatusResult {
                branch: String::new(),
                entries: vec![],
                error: Some(e.to_string()),
            }
        }
    }
}

fn git_diff_result(workdir: PathBuf, path: Option<String>, staged: bool) -> GitDiffResult {
    match GitRepo::open(&workdir) {
        Ok(repo) => match repo.diff(path.as_deref(), staged) {
            Ok(diff) => GitDiffResult { diff, error: None },
            Err(e) => {
                tracing::warn!("GitDiff: failed to diff {:?}: {}", path, e);
                GitDiffResult {
                    diff: String::new(),
                    error: Some(e.to_string()),
                }
            }
        },
        Err(e) => {
            tracing::warn!("GitDiff: failed to open repo at {:?}: {}", workdir, e);
            GitDiffResult {
                diff: String::new(),
                error: Some(e.to_string()),
            }
        }
    }
}

fn git_log_result(workdir: PathBuf, limit: Option<usize>) -> GitLogResult {
    match GitRepo::open(&workdir) {
        Ok(repo) => {
            let entries = repo
                .log(limit.unwrap_or(20).min(500))
                .unwrap_or_default()
                .into_iter()
                .map(|e| GitLogEntry {
                    id: e.id,
                    message: e.message,
                    author: e.author,
                    timestamp: e.timestamp,
                })
                .collect();
            GitLogResult {
                entries,
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!("GitLog: failed to open repo at {:?}: {}", workdir, e);
            GitLogResult {
                entries: vec![],
                error: Some(e.to_string()),
            }
        }
    }
}

fn git_commit_result(
    workdir: PathBuf,
    message: String,
    paths: Vec<String>,
) -> (GitCommitResult, bool) {
    match GitRepo::open(&workdir) {
        Ok(repo) => match repo.commit(&message, &paths) {
            Ok(hash) => (GitCommitResult { hash, error: None }, true),
            Err(e) => {
                tracing::warn!("GitCommit: commit failed: {}", e);
                (
                    GitCommitResult {
                        hash: String::new(),
                        error: Some(e.to_string()),
                    },
                    false,
                )
            }
        },
        Err(e) => {
            tracing::warn!("GitCommit: failed to open repo at {:?}: {}", workdir, e);
            (
                GitCommitResult {
                    hash: String::new(),
                    error: Some(e.to_string()),
                },
                false,
            )
        }
    }
}

fn git_stage_result(workdir: PathBuf, paths: Vec<String>) -> GitStageResult {
    let error = GitRepo::open(&workdir)
        .and_then(|repo| repo.stage(&paths))
        .err()
        .map(|e| {
            tracing::warn!("GitStage failed: {}", e);
            e.to_string()
        });
    GitStageResult { error }
}

fn git_unstage_result(workdir: PathBuf, paths: Vec<String>) -> GitUnstageResult {
    let error = GitRepo::open(&workdir)
        .and_then(|repo| repo.unstage(&paths))
        .err()
        .map(|e| {
            tracing::warn!("GitUnstage failed: {}", e);
            e.to_string()
        });
    GitUnstageResult { error }
}

fn git_branches_result(workdir: PathBuf) -> GitBranchesResult {
    match GitRepo::open(&workdir) {
        Ok(repo) => {
            let branches = repo
                .branches()
                .unwrap_or_default()
                .into_iter()
                .map(|b| GitBranchEntry {
                    name: b.name,
                    is_head: b.is_head,
                })
                .collect();
            GitBranchesResult {
                branches,
                error: None,
            }
        }
        Err(e) => {
            tracing::warn!("GitBranches: failed to open repo at {:?}: {}", workdir, e);
            GitBranchesResult {
                branches: vec![],
                error: Some(e.to_string()),
            }
        }
    }
}

fn git_checkout_result(workdir: PathBuf, branch: String) -> GitCheckoutResult {
    let ok = GitRepo::open(&workdir)
        .and_then(|repo| repo.checkout(&branch))
        .is_ok();
    GitCheckoutResult { ok }
}

async fn dispatch(
    msg: ZedraMessage,
    session: Arc<ServerSession>,
    state: Arc<DaemonState>,
    registry: Arc<SessionRegistry>,
    client_pubkey: [u8; 32],
    active_connection_id: u64,
) -> Result<()> {
    if !registry
        .is_active_client(&session.id, client_pubkey, active_connection_id)
        .await
    {
        tracing::debug!(
            "ignoring request from stale client {:?}... for session {}",
            &client_pubkey[..4],
            session.id,
        );
        return Ok(());
    }

    match msg {
        // -- Auth / bootstrap (should not appear in dispatch loop) --
        ZedraMessage::Register(_)
        | ZedraMessage::Authenticate(_)
        | ZedraMessage::AuthProve(_)
        | ZedraMessage::Connect(_) => {
            tracing::warn!("auth message received in dispatch loop (ignored)");
        }

        // -- Health --
        ZedraMessage::Ping(msg) => {
            session.touch().await;
            let ts = msg.timestamp_ms;
            let _ = msg.tx.send(PongResult { timestamp_ms: ts }).await;
        }

        // -- Session --
        ZedraMessage::GetSessionInfo(msg) => {
            let info = collect_host_env(&state.workdir);
            let _ = msg
                .tx
                .send(SessionInfoResult {
                    hostname: info.hostname,
                    workdir: info.workdir,
                    username: info.username,
                    home_dir: info.home_dir,
                    session_id: Some(session.id.clone()),
                    os: Some(std::env::consts::OS.to_string()),
                    arch: Some(std::env::consts::ARCH.to_string()),
                    os_version: os_version_string(),
                    host_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                })
                .await;
        }

        ZedraMessage::SyncSession(msg) => {
            let session_token = session.issue_session_token(client_pubkey).await;
            let _ = msg
                .tx
                .send(build_sync_result(&session, &state, session_token).await)
                .await;
        }

        ZedraMessage::ListSessions(msg) => {
            let list = registry.list_sessions().await;
            let sessions = list
                .into_iter()
                .map(|s| SessionListEntry {
                    id: s.id,
                    name: s.name,
                    workdir: s.workdir.map(|p| p.to_string_lossy().into_owned()),
                    terminal_count: s.terminal_count,
                    uptime_secs: s.created_at_elapsed_secs,
                    idle_secs: s.last_activity_elapsed_secs,
                    is_occupied: s.is_occupied,
                })
                .collect();
            let _ = msg.tx.send(SessionListResult { sessions }).await;
        }

        ZedraMessage::SwitchSession(msg) => {
            tracing::warn!(
                "SwitchSession: session switching is unsupported for {:?}",
                msg.session_name
            );
            let _ = msg
                .tx
                .send(SessionSwitchResult {
                    session_id: String::new(),
                    workdir: None,
                    error: Some(
                        "session switching is unsupported; reconnect to the target session"
                            .to_string(),
                    ),
                })
                .await;
        }

        // -- Filesystem --
        ZedraMessage::FsList(msg) => {
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsList: rejected path {:?}: {}", msg.path, e);
                    let _ = msg
                        .tx
                        .send(FsListResult {
                            entries: vec![],
                            total: 0,
                            has_more: false,
                            error: Some(e.to_string()),
                        })
                        .await;
                    return Ok(());
                }
            };
            match state.fs.list(&path) {
                Ok(entries) => {
                    let total = entries.len() as u32;
                    let limit = if msg.limit == 0 {
                        FS_LIST_DEFAULT_LIMIT
                    } else {
                        msg.limit.min(FS_LIST_DEFAULT_LIMIT)
                    } as usize;
                    let offset = msg.offset as usize;
                    let page: Vec<FsEntry> = entries
                        .into_iter()
                        .skip(offset)
                        .take(limit)
                        .map(|e| FsEntry {
                            name: e.name,
                            path: e.path.to_string_lossy().into_owned(),
                            is_dir: e.is_dir,
                            size: e.size,
                        })
                        .collect();
                    let has_more = (offset + page.len()) < total as usize;
                    let _ = msg
                        .tx
                        .send(FsListResult {
                            entries: page,
                            total,
                            has_more,
                            error: None,
                        })
                        .await;
                }
                Err(e) => {
                    tracing::warn!("FsList: list failed for {:?}: {}", path, e);
                    let _ = msg
                        .tx
                        .send(FsListResult {
                            entries: vec![],
                            total: 0,
                            has_more: false,
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::FsSearch(msg) => {
            session.rpc_fs_reads.fetch_add(1, Ordering::Relaxed);
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsSearch: rejected path {:?}: {}", msg.path, e);
                    let _ = msg.tx.send(fs_search_error(e.to_string())).await;
                    return Ok(());
                }
            };

            let query = msg.query.clone();
            let limit = msg.limit;
            let search_result =
                tokio::task::spawn_blocking(move || search_files(&path, &query, limit))
                    .await
                    .map_err(|error| anyhow::anyhow!("file search task failed: {error}"))
                    .and_then(|result| result);

            match search_result {
                Ok(result) => {
                    let _ = msg.tx.send(result).await;
                }
                Err(e) => {
                    tracing::warn!("FsSearch: search failed for {:?}: {}", msg.path, e);
                    let _ = msg.tx.send(fs_search_error(e.to_string())).await;
                }
            }
        }

        ZedraMessage::SetAppState(msg) => {
            registry
                .set_foreground_if_active(
                    &session.id,
                    client_pubkey,
                    active_connection_id,
                    msg.in_foreground,
                )
                .await;
            tracing::info!(
                in_foreground = msg.in_foreground,
                "client app state updated"
            );
            let _ = msg.tx.send(SetAppStateResult {}).await;
        }

        ZedraMessage::SetClientDeltaInfo(msg) => {
            let info = crate::delta::ClientDeltaInfo {
                delta_url: msg.delta_url.clone(),
                stack_id: msg.stack_id,
                client_node_id: msg.client_node_id,
                host_node_id: msg.host_node_id,
            };
            match crate::delta::DeltaClient::from_client_info(&info) {
                Ok(client) => {
                    *state.delta.write().await = Some(client);
                    tracing::info!(
                        stack_id = %msg.stack_id,
                        client_node_id = %msg.client_node_id,
                        host_node_id = %msg.host_node_id,
                        delta_url = %msg.delta_url,
                        "Delta client updated from connected mobile client"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        stack_id = %msg.stack_id,
                        client_node_id = %msg.client_node_id,
                        host_node_id = %msg.host_node_id,
                        error = %err,
                        "Failed to build Delta client from client info"
                    )
                }
            }
            let _ = msg.tx.send(SetClientDeltaInfoResult {}).await;
        }

        ZedraMessage::ClearClientDeltaInfo(msg) => {
            if let Some(delta) = state.delta.read().await.as_ref() {
                tracing::info!(
                    stack_id = %delta.stack_id(),
                    client_node_id = ?delta.client_node_id(),
                    host_node_id = %delta.host_node_id(),
                    "Delta client cleared from connected mobile client"
                );
            } else {
                tracing::info!(
                    "Delta client clear requested but no in-memory client delta was set"
                );
            }
            *state.delta.write().await = None;
            let _ = msg.tx.send(ClearClientDeltaInfoResult {}).await;
        }

        ZedraMessage::FsRead(msg) => {
            session.rpc_fs_reads.fetch_add(1, Ordering::Relaxed);
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsRead: rejected path {:?}: {}", msg.path, e);
                    let _ = msg
                        .tx
                        .send(FsReadResult {
                            content: String::new(),
                            too_large: false,
                            error: Some(e.to_string()),
                        })
                        .await;
                    return Ok(());
                }
            };
            const MAX_FILE_SIZE: u64 = 500 * 1024;
            if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_SIZE {
                let _ = msg
                    .tx
                    .send(FsReadResult {
                        content: String::new(),
                        too_large: true,
                        error: None,
                    })
                    .await;
                return Ok(());
            }
            match state.fs.read(&path) {
                Ok(content) => {
                    let _ = msg
                        .tx
                        .send(FsReadResult {
                            content,
                            too_large: false,
                            error: None,
                        })
                        .await;
                }
                Err(e) => {
                    tracing::warn!("FsRead: read failed for {:?}: {}", path, e);
                    let _ = msg
                        .tx
                        .send(FsReadResult {
                            content: String::new(),
                            too_large: false,
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::FsWrite(msg) => {
            session.rpc_fs_writes.fetch_add(1, Ordering::Relaxed);
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsWrite: rejected path {:?}: {}", msg.path, e);
                    let _ = msg.tx.send(FsWriteResult { ok: false }).await;
                    return Ok(());
                }
            };
            let ok = state.fs.write(&path, &msg.content).is_ok();
            let _ = msg.tx.send(FsWriteResult { ok }).await;
        }

        ZedraMessage::FsUpload(msg) => {
            if msg.data.len() > FS_UPLOAD_MAX_BYTES {
                tracing::warn!(
                    "FsUpload: rejected {} byte payload (max {})",
                    msg.data.len(),
                    FS_UPLOAD_MAX_BYTES
                );
                let _ = msg
                    .tx
                    .send(FsUploadResult {
                        path: String::new(),
                        error: Some("image exceeds the maximum upload size".to_string()),
                    })
                    .await;
                return Ok(());
            }
            match uploads::store_upload(&state.workdir, &msg.data, &msg.extension) {
                Ok(path) => {
                    let _ = msg.tx.send(FsUploadResult { path, error: None }).await;
                }
                Err(e) => {
                    tracing::warn!("FsUpload: store failed: {e:#}");
                    let _ = msg
                        .tx
                        .send(FsUploadResult {
                            path: String::new(),
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::FsStat(msg) => {
            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("FsStat: rejected path {:?}: {}", msg.path, e);
                    let _ = msg
                        .tx
                        .send(FsStatResult {
                            path: String::new(),
                            is_dir: false,
                            size: 0,
                            modified: None,
                            error: Some(e.to_string()),
                        })
                        .await;
                    return Ok(());
                }
            };
            match state.fs.stat(&path) {
                Ok(stat) => {
                    let _ = msg
                        .tx
                        .send(FsStatResult {
                            path: stat.path.to_string_lossy().into_owned(),
                            is_dir: stat.is_dir,
                            size: stat.size,
                            modified: stat.modified,
                            error: None,
                        })
                        .await;
                }
                Err(e) => {
                    tracing::warn!("FsStat: stat failed for {:?}: {}", path, e);
                    let _ = msg
                        .tx
                        .send(FsStatResult {
                            path: String::new(),
                            is_dir: false,
                            size: 0,
                            modified: None,
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::FsDocsTree(msg) => {
            if let Err(error) = validate_docs_tree_offset(msg.offset) {
                let _ = msg
                    .tx
                    .send(FsDocsTreeResult {
                        root: None,
                        snapshot_id: None,
                        next_offset: 0,
                        has_more: false,
                        truncated: false,
                        error: Some(error),
                    })
                    .await;
                return Ok(());
            }

            let path = match resolve_path(&state.workdir, &msg.path) {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!("FsDocsTree: rejected path {:?}: {}", msg.path, error);
                    let _ = msg
                        .tx
                        .send(FsDocsTreeResult {
                            root: None,
                            snapshot_id: None,
                            next_offset: 0,
                            has_more: false,
                            truncated: false,
                            error: Some(FsDocsTreeError::InvalidPath),
                        })
                        .await;
                    return Ok(());
                }
            };

            if !path.is_dir() {
                let _ = msg
                    .tx
                    .send(FsDocsTreeResult {
                        root: None,
                        snapshot_id: None,
                        next_offset: 0,
                        has_more: false,
                        truncated: false,
                        error: Some(FsDocsTreeError::InvalidRequest(
                            "docs tree path must be a directory".to_string(),
                        )),
                    })
                    .await;
                return Ok(());
            }

            let root_key = docs_tree_cache_key(&path);
            let limit = docs_tree_limit(msg.limit);

            if msg.rebuild {
                if !session.try_begin_docs_tree_scan() {
                    let _ = msg
                        .tx
                        .send(FsDocsTreeResult {
                            root: None,
                            snapshot_id: None,
                            next_offset: 0,
                            has_more: false,
                            truncated: false,
                            error: Some(FsDocsTreeError::Busy),
                        })
                        .await;
                    return Ok(());
                }

                let scan_path = path.clone();
                let scan_result = tokio::task::spawn_blocking(move || build_snapshot(scan_path))
                    .await
                    .map_err(|error| anyhow::anyhow!("docs tree scan task failed: {error}"))
                    .and_then(|result| result);
                session.finish_docs_tree_scan();

                match scan_result {
                    Ok(snapshot) => {
                        // Manual rebuild replaces the snapshot and always restarts paging.
                        let result = snapshot_page_result(&snapshot, 0, limit);
                        session.store_docs_tree_snapshot(root_key, snapshot).await;
                        let _ = msg.tx.send(result).await;
                    }
                    Err(error) => {
                        tracing::warn!("FsDocsTree: scan failed for {:?}: {}", path, error);
                        let _ = msg
                            .tx
                            .send(FsDocsTreeResult {
                                root: None,
                                snapshot_id: None,
                                next_offset: 0,
                                has_more: false,
                                truncated: false,
                                error: Some(FsDocsTreeError::ScanFailed(error.to_string())),
                            })
                            .await;
                    }
                }
                return Ok(());
            }

            match session
                .docs_tree_page(&root_key, msg.snapshot_id.as_deref(), msg.offset, limit)
                .await
            {
                Ok(result) => {
                    let _ = msg.tx.send(result).await;
                }
                Err(error) => {
                    let _ = msg
                        .tx
                        .send(FsDocsTreeResult {
                            root: None,
                            snapshot_id: None,
                            next_offset: 0,
                            has_more: false,
                            truncated: false,
                            error: Some(error),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::FsWatch(msg) => {
            if !session.allow_fs_watch_rpc().await {
                session
                    .fs_watch_rate_limited
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!("FsWatch rate limited: session={}", session.id);
                let _ = msg.tx.send(FsWatchResult::RateLimited).await;
                return Ok(());
            }
            let result = match normalize_observer_path(&msg.path) {
                Some(path) => {
                    if session.try_add_watched_path(path).await {
                        FsWatchResult::Ok
                    } else {
                        FsWatchResult::QuotaExceeded
                    }
                }
                None => FsWatchResult::InvalidPath,
            };
            if !matches!(result, FsWatchResult::Ok) {
                tracing::warn!(
                    "FsWatch rejected: session={} path={:?} quota={} max_watched_paths={}",
                    session.id,
                    msg.path,
                    session.fs_watch_quota_rejected.load(Ordering::Relaxed),
                    MAX_WATCHED_PATHS_PER_SESSION
                );
            }
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::FsUnwatch(msg) => {
            if !session.allow_fs_watch_rpc().await {
                session
                    .fs_watch_rate_limited
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!("FsUnwatch rate limited: session={}", session.id);
                let _ = msg.tx.send(FsUnwatchResult::RateLimited).await;
                return Ok(());
            }
            let result = match normalize_observer_path(&msg.path) {
                Some(path) => {
                    if session.remove_watched_path(&path).await {
                        FsUnwatchResult::Ok
                    } else {
                        FsUnwatchResult::NotWatched
                    }
                }
                None => FsUnwatchResult::InvalidPath,
            };
            let _ = msg.tx.send(result).await;
        }

        // -- Terminal --
        ZedraMessage::TermCreate(msg) => {
            session.touch().await;
            let workdir = session
                .workdir
                .clone()
                .or_else(|| Some(state.workdir.clone()));
            let has_launch_cmd = msg.launch_cmd.is_some();
            let launch_cmd = msg.launch_cmd.clone();
            match create_terminal(
                &session,
                msg.cols,
                msg.rows,
                SpawnOptions {
                    workdir,
                    launch_cmd,
                    color_scheme: None,
                    env: Vec::new(),
                },
            )
            .await
            {
                Ok(id) => {
                    zedra_telemetry::send(Event::HostTerminalOpen { has_launch_cmd });
                    let terminal_count = session.terminals.lock().await.len();
                    if let Err(e) = metrics::record_terminal_created(&state.workdir, terminal_count)
                    {
                        tracing::warn!("Failed to record terminal metrics: {}", e);
                    }
                    let _ = msg.tx.send(TermCreateResult { id, error: None }).await;
                }
                Err(e) => {
                    tracing::warn!("TermCreate failed: {}", e);
                    let _ = msg
                        .tx
                        .send(TermCreateResult {
                            id: String::new(),
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::TermCreateV2(msg) => {
            session.touch().await;
            let workdir = session
                .workdir
                .clone()
                .or_else(|| Some(state.workdir.clone()));
            let has_launch_cmd = msg.launch_cmd.is_some();
            let launch_cmd = msg.launch_cmd.clone();
            match create_terminal(
                &session,
                msg.cols,
                msg.rows,
                SpawnOptions {
                    workdir,
                    launch_cmd,
                    color_scheme: msg.color_scheme,
                    env: Vec::new(),
                },
            )
            .await
            {
                Ok(id) => {
                    zedra_telemetry::send(Event::HostTerminalOpen { has_launch_cmd });
                    let terminal_count = session.terminals.lock().await.len();
                    if let Err(e) = metrics::record_terminal_created(&state.workdir, terminal_count)
                    {
                        tracing::warn!("Failed to record terminal metrics: {}", e);
                    }
                    let _ = msg.tx.send(TermCreateResult { id, error: None }).await;
                }
                Err(e) => {
                    tracing::warn!("TermCreateV2 failed: {}", e);
                    let _ = msg
                        .tx
                        .send(TermCreateResult {
                            id: String::new(),
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        ZedraMessage::Subscribe(msg) => {
            session.touch().await;
            // Assume the client is in the foreground on a fresh connection; the app
            // will send SetAppState when it actually backgrounds. Guard on the active
            // client so a superseded Subscribe cannot force-foreground the session.
            registry
                .set_foreground_if_active(&session.id, client_pubkey, active_connection_id, true)
                .await;
            // Bridge: store a regular tokio sender in the session; spawn a task
            // that forwards events from it to the irpc channel toward the client.
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<HostEvent>(32);
            *session.event_tx.lock().await = Some(bridge_tx);
            let irpc_tx = msg.tx;
            {
                let mut watched = session.fs_watched_paths.lock().await;
                watched.insert(".".to_string());
            }
            let my_gen = session.observer_gen.fetch_add(1, Ordering::AcqRel) + 1;
            let observer_session = session.clone();
            let observer_workdir = state.workdir.clone();
            tokio::spawn(async move {
                run_observer(observer_session, observer_workdir, my_gen).await;
            });
            tokio::spawn(async move {
                while let Some(event) = bridge_rx.recv().await {
                    if irpc_tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }

        ZedraMessage::SubscribeHostInfo(msg) => {
            session.touch().await;
            let irpc_tx = msg.tx;
            tokio::spawn(async move {
                let sampler = tokio::task::spawn_blocking(host_info::new_system_sampler).await;
                let Ok(mut system) = sampler else {
                    tracing::warn!("host_info: failed to initialize system sampler");
                    return;
                };

                tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
                loop {
                    let sampled = tokio::task::spawn_blocking(move || {
                        let snapshot = host_info::collect_host_info_snapshot(&mut system);
                        (system, snapshot)
                    })
                    .await;

                    let Ok((next_system, snapshot)) = sampled else {
                        tracing::warn!("host_info: sampler task failed");
                        break;
                    };
                    system = next_system;

                    if irpc_tx.send(snapshot).await.is_err() {
                        break;
                    }

                    tokio::time::sleep(host_info::HOST_INFO_SAMPLE_INTERVAL).await;
                }
            });
        }

        ZedraMessage::TermAttach(msg) => {
            session.touch().await;

            let term_id = msg.id.clone();
            let last_seq = msg.last_seq;
            let irpc_tx = msg.tx;
            let mut irpc_rx = msg.rx;

            {
                let terms = session.terminals.lock().await;
                if !terms.contains_key(&term_id) {
                    tracing::warn!("TermAttach: unknown terminal {}", term_id);
                    return Ok(());
                }
            }

            // Synthetic metadata preamble (seq=0): inject cached OSC terminal
            // metadata so the client seeds TerminalMeta even when those OSC
            // sequences were evicted from the backlog. seq=0 is a reserved
            // marker; the client pump processes its data but skips seq
            // tracking and gap detection.
            // Extract the preamble bytes while holding the sync lock, then send
            // after releasing it to avoid holding a MutexGuard across an await.
            let preamble: Option<Vec<u8>> = {
                let terms = session.terminals.lock().await;
                terms.get(&term_id).and_then(|term| {
                    term.host_meta.lock().ok().and_then(|meta| {
                        let p = encode_meta_preamble(&meta);
                        if p.is_empty() {
                            None
                        } else {
                            Some(p)
                        }
                    })
                })
            };
            if let Some(p) = preamble {
                tracing::debug!(
                    "TermAttach: sending meta preamble ({} bytes) for {}",
                    p.len(),
                    term_id
                );
                if irpc_tx.send(TermOutput { data: p, seq: 0 }).await.is_err() {
                    return Ok(());
                }
            }

            // Replay backlog
            let Some(backlog_replay) = session.backlog_replay_after(&term_id, last_seq).await
            else {
                tracing::warn!(
                    "TermAttach: terminal {} vanished before backlog replay",
                    term_id
                );
                return Ok(());
            };
            if let Some(oldest_seq) = backlog_replay.oldest_seq {
                let first_missing_seq = last_seq.saturating_add(1);
                if first_missing_seq < oldest_seq {
                    tracing::warn!(
                        "TermAttach: backlog gap detected id={} last_seq={} first_missing_seq={} oldest_retained_seq={} newest_retained_seq={} retained_entries={} retained_bytes={} replay_entries={} session={}",
                        term_id,
                        last_seq,
                        first_missing_seq,
                        oldest_seq,
                        backlog_replay.newest_seq,
                        backlog_replay.retained_entries,
                        backlog_replay.retained_bytes,
                        backlog_replay.entries.len(),
                        session.id,
                    );
                }
            }
            tracing::info!(
                "TermAttach: id={} last_seq={} backlog_entries={} session={}",
                term_id,
                last_seq,
                backlog_replay.entries.len(),
                session.id,
            );
            for entry in backlog_replay.entries {
                if irpc_tx
                    .send(TermOutput {
                        data: entry.data,
                        seq: entry.seq,
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }

            // Extract the writer Arc at setup so the input loop can write
            // directly without re-acquiring session.terminals on every keystroke
            // (Fix 3). The Arc stays valid even if the terminal is removed from
            // the map; writes will simply fail harmlessly against the closed PTY.
            let pty_writer = {
                let terms = session.terminals.lock().await;
                terms.get(&term_id).map(|t| t.writer.clone())
            };
            let Some(pty_writer) = pty_writer else {
                tracing::warn!(
                    "TermAttach: terminal {} vanished before writer extract",
                    term_id
                );
                return Ok(());
            };

            // Set up bridge.
            // Capture the generation we install so cleanup can guard against
            // clobbering a sender installed by a concurrent newer TermAttach.
            let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<TermOutput>(256);
            let my_sender_gen: u64 = {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    let mut slot = term.output_sender.lock().unwrap();
                    slot.gen = slot.gen.wrapping_add(1);
                    slot.sender = Some(bridge_tx);
                    slot.gen
                } else {
                    0
                }
            };

            // Separate output task so slow relay sends don't block input processing.
            // With high-latency connections (e.g. relay RTT ~300ms), irpc_tx.send().await
            // can stall waiting for QUIC flow control acks. If input and output share a
            // single select! loop, that stall prevents keystrokes from reaching the PTY.
            let output_task = tokio::spawn(async move {
                while let Some(mut term_output) = bridge_rx.recv().await {
                    // Coalesce any chunks that arrived while the previous send was in
                    // flight. Under relay congestion the channel can accumulate many
                    // small PTY reads; merging them reduces irpc framing overhead and
                    // the number of QUIC stream writes without adding any extra delay
                    // for interactive typing (single-byte keystrokes never accumulate).
                    while let Ok(next) = bridge_rx.try_recv() {
                        term_output.data.extend_from_slice(&next.data);
                        term_output.seq = next.seq;
                    }
                    if irpc_tx.send(term_output).await.is_err() {
                        break;
                    }
                }
            });

            loop {
                match irpc_rx.recv().await {
                    Ok(Some(term_input)) => {
                        // Write directly via the pre-captured writer Arc —
                        // no session.terminals lock needed per keystroke (Fix 3).
                        if let Ok(mut w) = pty_writer.lock() {
                            let _ = w.write_all(&term_input.data);
                            let _ = w.flush();
                        }
                    }
                    Ok(None) => break,
                    Err(irpc::channel::mpsc::RecvError::Io { source, .. }) => {
                        tracing::info!(
                            "TermAttach: input stream closed id={} session={} error={:?}",
                            term_id,
                            session.id,
                            source
                        );
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "TermAttach: input receiver failed id={} session={} error={}",
                            term_id,
                            session.id,
                            e
                        );
                        break;
                    }
                }
            }

            output_task.abort();

            // Only clear output_sender if it still belongs to this TermAttach.
            // A concurrent newer TermAttach may have already replaced the sender;
            // clearing it unconditionally would silence that client's output.
            {
                let terms = session.terminals.lock().await;
                if let Some(term) = terms.get(&term_id) {
                    let mut slot = term.output_sender.lock().unwrap();
                    if slot.gen == my_sender_gen {
                        slot.sender = None;
                    }
                }
            }
        }

        ZedraMessage::TermResize(msg) => {
            let terms = session.terminals.lock().await;
            let ok = if let Some(term) = terms.get(&msg.id) {
                term.master
                    .resize(portable_pty::PtySize {
                        rows: msg.rows,
                        cols: msg.cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .is_ok()
            } else {
                false
            };
            let _ = msg.tx.send(TermResizeResult { ok }).await;
        }

        ZedraMessage::TermClose(msg) => {
            let terminal = session.remove_terminal(&msg.id).await;
            let ok = if let Some(terminal) = terminal {
                tokio::task::spawn_blocking(move || terminal.terminate())
                    .await
                    .unwrap_or(false)
            } else {
                false
            };
            let _ = msg.tx.send(TermCloseResult { ok }).await;
        }

        ZedraMessage::TermList(msg) => {
            let terminals = session
                .terminal_ids()
                .await
                .into_iter()
                .enumerate()
                .map(|(position, id)| TermListEntry {
                    id,
                    position: position as u64,
                })
                .collect();
            let _ = msg.tx.send(TermListResult { terminals }).await;
        }

        ZedraMessage::TermReorder(msg) => {
            let result = match session.reorder_terminals(msg.ordered_ids.clone()).await {
                Ok(()) => TermReorderResult {
                    ok: true,
                    error: None,
                },
                Err(error) => TermReorderResult {
                    ok: false,
                    error: Some(error),
                },
            };
            let _ = msg.tx.send(result).await;
        }

        // -- Git --
        ZedraMessage::GitStatus(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let result = tokio::task::spawn_blocking(move || git_status_result(workdir))
                .await
                .unwrap_or_else(|e| GitStatusResult {
                    branch: String::new(),
                    entries: vec![],
                    error: Some(format!("git status worker failed: {e}")),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitDiff(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let path = msg.path.clone();
            let staged = msg.staged;
            let result =
                tokio::task::spawn_blocking(move || git_diff_result(workdir, path, staged))
                    .await
                    .unwrap_or_else(|e| GitDiffResult {
                        diff: String::new(),
                        error: Some(format!("git diff worker failed: {e}")),
                    });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitLog(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let limit = msg.limit;
            let result = tokio::task::spawn_blocking(move || git_log_result(workdir, limit))
                .await
                .unwrap_or_else(|e| GitLogResult {
                    entries: vec![],
                    error: Some(format!("git log worker failed: {e}")),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitCommit(msg) => {
            let files_staged = msg.paths.len();
            let workdir = state.workdir.clone();
            let message = msg.message.clone();
            let paths = msg.paths.clone();
            let (result, success) =
                tokio::task::spawn_blocking(move || git_commit_result(workdir, message, paths))
                    .await
                    .unwrap_or_else(|e| {
                        (
                            GitCommitResult {
                                hash: String::new(),
                                error: Some(format!("git commit worker failed: {e}")),
                            },
                            false,
                        )
                    });
            if success {
                session.rpc_git_commits.fetch_add(1, Ordering::Relaxed);
            }
            zedra_telemetry::send(Event::GitCommitMade {
                files_staged,
                success,
            });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitStage(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let paths = msg.paths.clone();
            let result = tokio::task::spawn_blocking(move || git_stage_result(workdir, paths))
                .await
                .unwrap_or_else(|e| GitStageResult {
                    error: Some(format!("git stage worker failed: {e}")),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitUnstage(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let paths = msg.paths.clone();
            let result = tokio::task::spawn_blocking(move || git_unstage_result(workdir, paths))
                .await
                .unwrap_or_else(|e| GitUnstageResult {
                    error: Some(format!("git unstage worker failed: {e}")),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitBranches(msg) => {
            session.rpc_git_ops.fetch_add(1, Ordering::Relaxed);
            let workdir = state.workdir.clone();
            let result = tokio::task::spawn_blocking(move || git_branches_result(workdir))
                .await
                .unwrap_or_else(|e| GitBranchesResult {
                    branches: vec![],
                    error: Some(format!("git branches worker failed: {e}")),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::GitCheckout(msg) => {
            let workdir = state.workdir.clone();
            let branch = msg.branch.clone();
            let result = tokio::task::spawn_blocking(move || git_checkout_result(workdir, branch))
                .await
                .unwrap_or(GitCheckoutResult { ok: false });
            let _ = msg.tx.send(result).await;
        }

        // -- AI --
        ZedraMessage::AiPrompt(msg) => {
            // Resolve the Claude binary path. Prefer an explicit absolute path
            // from the environment to avoid executing a malicious `claude` binary
            // that might appear earlier in $PATH.
            let claude_bin =
                std::env::var("ZEDRA_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
            let prompt = msg.prompt.clone();
            let prompt_bytes = prompt.len();
            let ai_start = std::time::Instant::now();
            let workdir = state.workdir.clone();
            let prompt_for_command = prompt.clone();
            let output = tokio::task::spawn_blocking(move || {
                std::process::Command::new(&claude_bin)
                    .args(["--print", prompt_for_command.as_str()])
                    .current_dir(workdir)
                    .output()
            })
            .await
            .unwrap_or_else(|e| {
                Err(std::io::Error::other(format!(
                    "AI prompt worker failed: {e}"
                )))
            });
            let duration_ms = ai_start.elapsed().as_millis() as u64;

            let (text, done, success) = match output {
                Ok(out) if out.status.success() => {
                    (String::from_utf8_lossy(&out.stdout).into_owned(), true, true)
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).into_owned();
                    (format!("Error: {}", err), true, false)
                }
                Err(e) => (
                    format!(
                        "Claude Code not found on host. Install with: npm i -g @anthropic-ai/claude-code\n\nPrompt was: {}\n\nError: {}",
                        prompt,
                        e
                    ),
                    true,
                    false,
                ),
            };
            session.rpc_ai_prompts.fetch_add(1, Ordering::Relaxed);
            zedra_telemetry::send(Event::AiPromptSent {
                success,
                duration_ms,
                prompt_bytes,
                response_bytes: text.len(),
            });
            let _ = msg.tx.send(AiPromptResult { text, done }).await;
        }

        ZedraMessage::AgentList(msg) => {
            session.touch().await;
            let workdir = session.workdir.as_ref().unwrap_or(&state.workdir);
            let result =
                agent::list_agents(&state.agent_cache, workdir, Some(&session), msg.refresh).await;
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::AgentSessions(msg) => {
            session.touch().await;
            let workdir = session.workdir.as_ref().unwrap_or(&state.workdir);
            let result = agent::list_agent_sessions(
                &state.agent_cache,
                &msg.slug,
                workdir,
                Some(&session),
                msg.limit,
                msg.refresh,
            )
            .await;
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::AgentInstalledList(msg) => {
            session.touch().await;
            let result = agent::list_installed_agents(&state.agent_cache, msg.refresh).await;
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::AgentFiles(msg) => {
            session.touch().await;
            let slug = msg.slug.clone();
            // File reads are blocking; keep them off the async dispatch path.
            let result = tokio::task::spawn_blocking(move || agent::agent_files(&slug))
                .await
                .map(|files| match files {
                    Ok(files) => AgentFilesResult { files, error: None },
                    Err(error) => AgentFilesResult {
                        files: Vec::new(),
                        error: Some(error),
                    },
                })
                .unwrap_or_else(|e| AgentFilesResult {
                    files: Vec::new(),
                    error: Some(e.to_string()),
                });
            let _ = msg.tx.send(result).await;
        }

        ZedraMessage::AgentResume(msg) => {
            session.touch().await;
            let workdir = session
                .workdir
                .clone()
                .or_else(|| Some(state.workdir.clone()));
            let launch_cmd = agent::resume_launch_command(&msg.slug, &msg.session_id);
            let Some(launch_cmd) = launch_cmd else {
                // `None` collapses three causes; report the specific one.
                let error = if agent::actor(&msg.slug).is_none() {
                    format!("unknown agent: {}", msg.slug)
                } else if msg.session_id.trim().is_empty() {
                    "missing session id".to_string()
                } else {
                    format!("agent {} does not support resume", msg.slug)
                };
                let _ = msg
                    .tx
                    .send(AgentResumeResult {
                        terminal_id: String::new(),
                        error: Some(error),
                    })
                    .await;
                return Ok(());
            };
            match create_terminal(
                &session,
                msg.cols,
                msg.rows,
                SpawnOptions {
                    workdir,
                    launch_cmd: Some(launch_cmd),
                    color_scheme: None,
                    env: Vec::new(),
                },
            )
            .await
            {
                Ok(terminal_id) => {
                    zedra_telemetry::send(Event::HostTerminalOpen {
                        has_launch_cmd: true,
                    });
                    let terminal_count = session.terminals.lock().await.len();
                    if let Err(e) = metrics::record_terminal_created(&state.workdir, terminal_count)
                    {
                        tracing::warn!("Failed to record terminal metrics: {}", e);
                    }
                    let _ = msg
                        .tx
                        .send(AgentResumeResult {
                            terminal_id,
                            error: None,
                        })
                        .await;
                }
                Err(e) => {
                    tracing::warn!("AgentResume failed: {}", e);
                    let _ = msg
                        .tx
                        .send(AgentResumeResult {
                            terminal_id: String::new(),
                            error: Some(e.to_string()),
                        })
                        .await;
                }
            }
        }

        // -- LSP --
        ZedraMessage::LspDiagnostics(msg) => {
            let full_path = match resolve_path(&state.workdir, &msg.path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("LspDiagnostics: rejected path {:?}: {}", msg.path, e);
                    let _ = msg
                        .tx
                        .send(LspDiagnosticsResult {
                            diagnostics: vec![],
                            error: Some(e.to_string()),
                        })
                        .await;
                    return Ok(());
                }
            };
            let diagnostics = run_lsp_check(&full_path)
                .into_iter()
                .map(|d| LspDiagnostic {
                    message: d.message,
                    severity: d.severity,
                })
                .collect();
            let _ = msg
                .tx
                .send(LspDiagnosticsResult {
                    diagnostics,
                    error: None,
                })
                .await;
        }

        ZedraMessage::LspHover(msg) => {
            let _ = msg
                .tx
                .send(LspHoverResult {
                    contents: "LSP hover not yet connected to a language server.".to_string(),
                })
                .await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct DiagnosticEntry {
    message: String,
    severity: String,
}

fn run_lsp_check(path: &std::path::Path) -> Vec<DiagnosticEntry> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let (cmd, args): (&str, Vec<&str>) = match ext {
        "rs" => ("cargo", vec!["check", "--message-format=json"]),
        "ts" | "tsx" | "js" | "jsx" => ("npx", vec!["tsc", "--noEmit"]),
        "py" => (
            "python3",
            vec!["-m", "py_compile", path.to_str().unwrap_or("")],
        ),
        _ => return vec![],
    };

    let output = std::process::Command::new(cmd)
        .args(&args)
        .current_dir(path.parent().unwrap_or(std::path::Path::new(".")))
        .output();

    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.is_empty() && out.status.success() {
                vec![]
            } else {
                stderr
                    .lines()
                    .take(10)
                    .filter(|l| !l.is_empty())
                    .map(|line| DiagnosticEntry {
                        message: line.to_string(),
                        severity: "error".into(),
                    })
                    .collect()
            }
        }
        Err(e) => {
            tracing::warn!("LspDiagnostics: command {} failed: {}", cmd, e);
            vec![]
        }
    }
}

pub(crate) fn os_version_string() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(pretty) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(pretty.trim_matches('"').to_string());
                }
            }
        }
        let output = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        Some(format!(
            "macOS {}",
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
