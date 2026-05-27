// CLI subcommands for the opt-in LSP control plane.
//
// All commands talk to the running daemon over its loopback REST API. When
// the daemon is not running, `status` falls back to reading the persisted
// `lsp.json`, and mutating commands fail with a clear error rather than
// editing the file behind the daemon's back (which would diverge from the
// in-memory `LspManager` if the daemon came back up).

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use zedra_host::{identity, utils};

#[derive(Debug, Args)]
pub struct LspArgs {
    #[command(subcommand)]
    pub command: LspCommand,
}

#[derive(Debug, Subcommand)]
pub enum LspCommand {
    /// Show LSP subsystem state for a workspace.
    Status(LspStatusArgs),
    /// Enable a language server for the active workspace.
    Enable(LspToggleArgs),
    /// Disable a language server for the active workspace.
    Disable(LspToggleArgs),
}

#[derive(Debug, Args)]
pub struct LspStatusArgs {
    /// Working directory of the running daemon.
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print the LSP block as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct LspToggleArgs {
    /// Language identifier: rust, go, typescript, javascript, python.
    language: String,
    /// Working directory of the running daemon.
    #[arg(short, long, default_value = ".")]
    workdir: String,
}

pub async fn run(args: LspArgs) -> Result<()> {
    match args.command {
        LspCommand::Status(a) => status(a).await,
        LspCommand::Enable(a) => enable(a).await,
        LspCommand::Disable(a) => disable(a).await,
    }
}

async fn status(args: LspStatusArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let lsp = match daemon_api(&workdir) {
        Ok((addr, token)) => fetch_lsp_block(&addr, &token).await?,
        Err(_) => offline_lsp_block(&workdir),
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&lsp)?);
    } else {
        println!("{}", render_lsp_status(&lsp));
    }
    Ok(())
}

async fn enable(args: LspToggleArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let body = serde_json::json!({ "language": args.language });
    let resp: serde_json::Value = api_post(&workdir, "/api/lsp/enable", &body)
        .await
        .context("LSP enable failed (is the daemon running?)")?;
    let lang = resp["language"].as_str().unwrap_or(&args.language);
    println!("Enabled LSP for {lang}");
    Ok(())
}

async fn disable(args: LspToggleArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let body = serde_json::json!({ "language": args.language });
    let resp: serde_json::Value = api_post(&workdir, "/api/lsp/disable", &body)
        .await
        .context("LSP disable failed (is the daemon running?)")?;
    let lang = resp["language"].as_str().unwrap_or(&args.language);
    println!("Disabled LSP for {lang}");
    Ok(())
}

/// Read the persisted enablement list when the daemon is not running.
/// Server-state fields stay zeroed since nothing is supervising them.
fn offline_lsp_block(workdir: &Path) -> serde_json::Value {
    let path = match identity::workspace_config_dir(workdir) {
        Ok(dir) => dir.join("lsp.json"),
        Err(_) => return serde_json::json!({ "enabled": false, "servers": [], "offline": true }),
    };
    let state = zedra_lsp::persistence::load(&path);
    let servers: Vec<serde_json::Value> = state
        .enabled_languages
        .iter()
        .map(|l| {
            serde_json::json!({
                "language": language_label(*l),
                "state": "idle",
                "pid": null_value(),
                "rss_bytes": 0,
                "uptime_secs": 0,
                "diagnostic_errors": 0,
                "diagnostic_warnings": 0,
                "last_request_ms": null_value(),
                "last_kill_reason": null_value(),
                "peak_rss_bytes": 0,
            })
        })
        .collect();
    serde_json::json!({
        "enabled": !state.enabled_languages.is_empty(),
        "servers": servers,
        "aggregate_rss_bytes": 0,
        "aggregate_rss_cap_bytes": 0,
        "concurrent_cap": 0,
        "offline": true,
    })
}

fn null_value() -> serde_json::Value {
    serde_json::Value::Null
}

fn language_label(language: zedra_rpc::proto::LspLanguage) -> &'static str {
    use zedra_rpc::proto::LspLanguage::*;
    match language {
        Rust => "rust",
        Go => "go",
        TypeScript => "typescript",
        JavaScript => "javascript",
        Python => "python",
    }
}

async fn fetch_lsp_block(addr: &str, token: &str) -> Result<serde_json::Value> {
    let url = format!("http://{}/api/status", addr.trim());
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token.trim())
        .send()
        .await
        .context("failed to reach daemon")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("daemon returned HTTP {status}: {text}");
    }
    let mut value: serde_json::Value =
        serde_json::from_str(&text).context("failed to decode daemon status response")?;
    Ok(value["lsp"].take())
}

fn render_lsp_status(lsp: &serde_json::Value) -> String {
    let offline = lsp
        .get("offline")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let enabled = lsp
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut sections = Vec::new();
    if offline {
        sections.push("Zedra daemon not running — showing persisted config.".to_string());
        sections.push(String::new());
    }
    if !enabled {
        sections.push("LSP: no languages enabled for this workspace.".to_string());
        sections.push(
            "Enable one with `zedra lsp enable <language>` while the daemon is running."
                .to_string(),
        );
        return sections.join("\n");
    }

    let servers = lsp
        .get("servers")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let running = servers
        .iter()
        .filter(|s| {
            matches!(
                s.get("state").and_then(|v| v.as_str()),
                Some("starting") | Some("ready")
            )
        })
        .count();
    let cap = lsp
        .get("concurrent_cap")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    sections.push(format!("LSP servers ({running} running, cap {cap})"));
    let rows: Vec<Vec<String>> = servers.iter().map(server_row).collect();
    sections.push(utils::render_table(
        &["LANG", "STATE", "PID", "RSS", "UPTIME", "DIAGS", "LAST"],
        &rows,
    ));

    let agg = lsp
        .get("aggregate_rss_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let agg_cap = lsp
        .get("aggregate_rss_cap_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if agg_cap > 0 {
        let pct = ((agg as f64 / agg_cap as f64) * 100.0).round() as u64;
        sections.push(format!(
            "Aggregate RSS: {} / {} ({}%)",
            format_bytes_mb(agg),
            format_bytes_mb(agg_cap),
            pct,
        ));
    }
    sections.join("\n")
}

fn server_row(server: &serde_json::Value) -> Vec<String> {
    let lang = server
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let state = server.get("state").and_then(|v| v.as_str()).unwrap_or("?");
    let pid = server
        .get("pid")
        .and_then(|v| v.as_u64())
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let rss = server
        .get("rss_bytes")
        .and_then(|v| v.as_u64())
        .map(format_bytes_mb)
        .unwrap_or_else(|| "-".to_string());
    let uptime = server
        .get("uptime_secs")
        .and_then(|v| v.as_u64())
        .map(utils::format_duration)
        .unwrap_or_else(|| "-".to_string());
    let errors = server
        .get("diagnostic_errors")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let warnings = server
        .get("diagnostic_warnings")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let last = server
        .get("last_request_ms")
        .and_then(|v| v.as_u64())
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "-".to_string());
    vec![
        lang.to_string(),
        state.to_string(),
        pid,
        rss,
        uptime,
        format!("{errors}E {warnings}W"),
        last,
    ]
}

fn format_bytes_mb(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_string();
    }
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

async fn api_post<T: DeserializeOwned, B: Serialize>(
    workdir: &Path,
    path: &str,
    body: &B,
) -> Result<T> {
    let (addr, token) = daemon_api(workdir)?;
    let url = format!("http://{}{}", addr.trim(), path);
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(token.trim())
        .json(body)
        .send()
        .await
        .context("failed to reach daemon")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("daemon returned HTTP {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("failed to decode daemon response: {text}"))
}

fn daemon_api(workdir: &Path) -> Result<(String, String)> {
    let config_dir = identity::workspace_config_dir(workdir)?;
    let addr = std::fs::read_to_string(config_dir.join("api-addr")).unwrap_or_default();
    let token = std::fs::read_to_string(config_dir.join("api-token")).unwrap_or_default();
    if addr.trim().is_empty() {
        bail!("No running daemon found for: {}", workdir.display());
    }
    Ok((addr, token))
}

fn resolve_workdir(raw: &str) -> PathBuf {
    PathBuf::from(raw)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(raw))
}
