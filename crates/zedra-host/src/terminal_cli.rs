use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use zedra_host::{identity, utils};

#[derive(Debug, Args)]
pub struct TerminalArgs {
    #[command(subcommand)]
    pub command: Option<TerminalCommand>,
    #[command(flatten)]
    pub open: TerminalOpenArgs,
}

#[derive(Debug, Subcommand)]
pub enum TerminalCommand {
    /// List active terminals for this workspace
    List(TerminalListArgs),
    /// Show details for one active terminal
    #[command(alias = "detail")]
    Details(TerminalDetailsArgs),
    /// Open a terminal on the connected phone
    Open(TerminalOpenArgs),
}

#[derive(Debug, Args)]
pub struct TerminalOpenArgs {
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,

    /// Command to run in the terminal on startup (e.g. "claude --resume <id>")
    #[arg(long)]
    launch_cmd: Option<String>,
}

#[derive(Debug, Args)]
pub struct TerminalListArgs {
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print terminal list JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct TerminalDetailsArgs {
    /// Zedra terminal id to inspect
    #[arg(long = "tid", alias = "terminal-id")]
    terminal_id: String,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print terminal detail JSON
    #[arg(long)]
    json: bool,
}

pub async fn run(args: TerminalArgs) -> Result<()> {
    match args.command {
        Some(TerminalCommand::List(args)) => list(args).await,
        Some(TerminalCommand::Details(args)) => details(args).await,
        Some(TerminalCommand::Open(args)) => open(args).await,
        None => open(args.open).await,
    }
}

async fn list(args: TerminalListArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let status: serde_json::Value = api_get(&workdir, "/api/status").await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(terminal_values(&status))?
        );
    } else {
        println!("{}", render_terminal_list(&status));
    }
    Ok(())
}

async fn details(args: TerminalDetailsArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let status: serde_json::Value = api_get(&workdir, "/api/status").await?;
    let terminal = find_terminal(&status, &args.terminal_id)
        .with_context(|| format!("terminal id not found: {}", args.terminal_id))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(terminal)?);
    } else {
        println!("{}", render_terminal_details(terminal));
    }
    Ok(())
}

async fn open(args: TerminalOpenArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let body = serde_json::json!({ "launch_cmd": args.launch_cmd.as_deref() });
    let response: serde_json::Value = api_post(&workdir, "/api/terminal", &body).await?;
    println!(
        "{}",
        render_terminal_created_output(&response, args.launch_cmd.as_deref())
    );
    Ok(())
}

fn render_terminal_list(status: &serde_json::Value) -> String {
    let terminals = terminal_values(status);
    let mut sections = vec!["Active Terminals".to_string(), String::new()];
    if terminals.is_empty() {
        sections.push("No active terminals.".to_string());
        return sections.join("\n");
    }

    let rows = terminals.iter().map(terminal_row).collect::<Vec<_>>();
    sections.push(utils::render_table(
        &["ID", "TITLE", "CWD", "UPTIME"],
        &rows,
    ));
    sections.join("\n")
}

fn terminal_row(terminal: &serde_json::Value) -> Vec<String> {
    vec![
        terminal["id"].as_str().unwrap_or("-").to_string(),
        non_empty_str(&terminal["title"])
            .unwrap_or("(untitled)")
            .to_string(),
        non_empty_str(&terminal["cwd"]).unwrap_or("-").to_string(),
        terminal["uptime_secs"]
            .as_u64()
            .map(utils::format_duration)
            .unwrap_or_else(|| "-".to_string()),
    ]
}

fn render_terminal_details(terminal: &serde_json::Value) -> String {
    let session = non_empty_str(&terminal["session_name"]).unwrap_or("-");
    let rows = vec![
        ("ID", terminal["id"].as_str().unwrap_or("-").to_string()),
        (
            "Title",
            non_empty_str(&terminal["title"])
                .unwrap_or("(untitled)")
                .to_string(),
        ),
        (
            "CWD",
            non_empty_str(&terminal["cwd"]).unwrap_or("-").to_string(),
        ),
        (
            "Uptime",
            terminal["uptime_secs"]
                .as_u64()
                .map(utils::format_duration)
                .unwrap_or_else(|| "-".to_string()),
        ),
        (
            "Created",
            terminal["created_at_elapsed_secs"]
                .as_u64()
                .map(|secs| format!("{} ago", utils::format_duration(secs)))
                .unwrap_or_else(|| "-".to_string()),
        ),
        ("Session", session.to_string()),
        (
            "Session ID",
            terminal["session_id"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Icon",
            non_empty_str(&terminal["icon_name"])
                .unwrap_or("-")
                .to_string(),
        ),
    ];

    format!("Terminal Details\n\n{}", utils::render_key_values(&rows))
}

fn render_terminal_created_output(
    response: &serde_json::Value,
    launch_cmd: Option<&str>,
) -> String {
    let id = response["id"].as_str().unwrap_or("-");
    let session_id = response["session_id"].as_str().unwrap_or("-");
    let mut rows = vec![("ID", id.to_string()), ("Session", session_id.to_string())];
    if let Some(launch_cmd) = launch_cmd.filter(|command| !command.is_empty()) {
        rows.push(("Command", launch_cmd.to_string()));
    }

    format!("Terminal Opened\n\n{}", utils::render_key_values(&rows))
}

async fn api_get<T: DeserializeOwned>(workdir: &Path, path: &str) -> Result<T> {
    let (addr, token) = daemon_api(workdir)?;
    let url = format!("http://{}{}", addr.trim(), path);
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token.trim())
        .send()
        .await
        .context("failed to reach daemon")?;
    decode_response(response).await
}

pub(crate) async fn api_post<T: DeserializeOwned, B: Serialize>(
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
    decode_response(response).await
}

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
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

pub(crate) fn resolve_workdir(raw: &str) -> PathBuf {
    PathBuf::from(raw)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(raw))
}

fn non_empty_str(value: &serde_json::Value) -> Option<&str> {
    value.as_str().filter(|value| !value.is_empty())
}

fn terminal_values(status: &serde_json::Value) -> &[serde_json::Value] {
    status["terminals"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn find_terminal<'a>(
    status: &'a serde_json::Value,
    terminal_id: &str,
) -> Option<&'a serde_json::Value> {
    terminal_values(status)
        .iter()
        .find(|terminal| terminal["id"].as_str() == Some(terminal_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_list_renders_full_ids() {
        let status = serde_json::json!({
            "terminals": [{
                "id": "terminal-full-id",
                "title": "claude",
                "cwd": "/repo",
                "uptime_secs": 5,
                "session_name": "zedra-main",
                "icon_name": "claude"
            }]
        });

        let output = render_terminal_list(&status);
        assert!(output.contains("terminal-full-id"));
        assert!(output.contains("/repo"));
        assert!(output.contains("ID"));
        assert!(output.contains("TITLE"));
        assert!(output.contains("CWD"));
        assert!(output.contains("UPTIME"));
        assert!(!output.contains("SESSION"));
        assert!(!output.contains("ICON"));
    }

    #[test]
    fn terminal_details_renders_extra_fields_for_one_terminal() {
        let terminal = serde_json::json!({
            "id": "terminal-full-id",
            "title": "claude",
            "cwd": "/repo",
            "uptime_secs": 65,
            "created_at_elapsed_secs": 70,
            "session_id": "session-full-id",
            "session_name": "zedra-main",
            "icon_name": "claude"
        });

        let output = render_terminal_details(&terminal);
        assert!(output.contains("Terminal Details"));
        assert!(output.contains("terminal-full-id"));
        assert!(output.contains("1m5s"));
        assert!(output.contains("session-full-id"));
    }

    #[test]
    fn terminal_created_output_summarizes_api_response() {
        let response = serde_json::json!({
            "id": "terminal-id",
            "session_id": "session-id"
        });

        let output = render_terminal_created_output(&response, Some("claude"));

        assert!(output.contains("Terminal Opened"));
        assert!(output.contains("  ID       terminal-id"));
        assert!(output.contains("  Session  session-id"));
        assert!(output.contains("  Command  claude"));
    }
}
