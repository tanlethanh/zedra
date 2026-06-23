use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{stderr, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zedra_host::{agent, identity, utils};
use zedra_rpc::proto::{
    AgentInfoField, AgentInstalledListResult, AgentKind, AgentListResult, AgentResumeResult,
    AgentSessionSummary, AgentSessionsResult, AgentSummary, AgentUsageSnapshot,
    InstalledAgentEntry,
};

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List managed-agent summaries for this workspace
    List(AgentListArgs),
    /// List sessions for one managed agent
    Sessions(AgentSessionsArgs),
    /// Resume an agent session in a new phone terminal
    Resume(AgentResumeArgs),
    /// Run local host filesystem/CLI scans (benchmark and raw preview)
    Scan {
        #[command(subcommand)]
        command: AgentScanCommand,
    },
    /// Install or test local agent hook integration
    Hook {
        #[command(subcommand)]
        command: AgentHookCommand,
    },
}

#[derive(Debug, Args)]
pub struct AgentListArgs {
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print the raw JSON response
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct AgentSessionsArgs {
    /// Agent provider to inspect
    #[arg(value_enum)]
    kind: CliManagedAgentKind,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print the raw JSON response
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentScanCommand {
    /// Probe installed terminal agent CLIs on PATH
    Installed(AgentScanCommonArgs),
    /// Scan managed-agent summaries for a workspace
    List(AgentScanWorkdirArgs),
    /// Scan sessions for one managed agent
    Sessions(AgentScanSessionsArgs),
    /// Run all local scans and print benchmark timings
    Bench(AgentScanBenchArgs),
    /// Fetch live rate-limit usage from provider APIs (Claude + Codex)
    Usage(AgentScanCommonArgs),
}

#[derive(Debug, Args)]
pub struct AgentScanCommonArgs {
    /// Print JSON output
    #[arg(long)]
    json: bool,
    /// Suppress elapsed timing on stderr
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub struct AgentScanWorkdirArgs {
    /// Workspace directory to scan
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Print JSON output
    #[arg(long)]
    json: bool,
    /// Suppress elapsed timing on stderr
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub struct AgentScanSessionsArgs {
    /// Agent provider to inspect
    #[arg(value_enum)]
    kind: CliManagedAgentKind,
    /// Workspace directory to scan
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Maximum sessions to return (`0` uses host default)
    #[arg(short, long, default_value_t = 0)]
    limit: u32,
    /// Print JSON output
    #[arg(long)]
    json: bool,
    /// Suppress elapsed timing on stderr
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub struct AgentScanBenchArgs {
    /// Workspace directory to scan
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Maximum sessions per agent (`0` uses host default)
    #[arg(short, long, default_value_t = 0)]
    limit: u32,
    /// Print JSON benchmark report
    #[arg(long)]
    json: bool,
    /// Suppress elapsed timing on stderr
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub struct AgentResumeArgs {
    /// Agent provider to resume
    #[arg(value_enum)]
    kind: CliManagedAgentKind,
    /// Provider session id to resume
    session_id: String,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Initial terminal columns
    #[arg(long, default_value_t = 220)]
    cols: u16,
    /// Initial terminal rows
    #[arg(long, default_value_t = 50)]
    rows: u16,
    /// Print the raw JSON response
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentHookCommand {
    /// Write project-local hook config files for this workspace
    Install(AgentHookInstallArgs),
    /// Receive one hook payload on stdin and forward it to the daemon
    Receive(AgentHookReceiveArgs),
    /// Send a synthetic hook payload to the daemon
    Test(AgentHookTestArgs),
}

#[derive(Debug, Args)]
pub struct AgentHookInstallArgs {
    /// Working directory to write local hook files into
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Provider to install. Repeat for multiple providers. Defaults to all.
    #[arg(long = "provider", value_enum)]
    providers: Vec<CliManagedAgentKind>,
    /// Overwrite existing generated hook config files
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
pub struct AgentHookReceiveArgs {
    /// Normalized agent slug: claude, codex, opencode, pi, hermes
    #[arg(long)]
    agent: String,
    /// Working directory of the running daemon. Falls back to ZEDRA_WORKDIR env.
    #[arg(short, long, env = "ZEDRA_WORKDIR", default_value = ".")]
    workdir: String,
    /// Zedra terminal id the hook originated from. Falls back to ZEDRA_TERMINAL_ID env.
    #[arg(long, env = "ZEDRA_TERMINAL_ID")]
    terminal_id: Option<String>,
    /// Hook payload JSON. If omitted, reads from stdin.
    #[arg(long)]
    payload: Option<String>,
    /// Do not print the daemon response. Useful for provider hooks.
    #[arg(long)]
    quiet: bool,
}

#[derive(Debug, Args)]
pub struct AgentHookTestArgs {
    /// Agent provider to simulate
    #[arg(value_enum)]
    kind: CliManagedAgentKind,
    /// Provider event name to simulate
    event: String,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Bind the synthetic hook to a known Zedra terminal id
    #[arg(long)]
    terminal_id: Option<String>,
    /// Print the raw JSON response
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CliManagedAgentKind {
    Claude,
    Codex,
    #[value(name = "opencode", alias = "open-code", alias = "open_code")]
    OpenCode,
    Pi,
    Hermes,
}

impl CliManagedAgentKind {
    fn slug(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::Hermes => "hermes",
        }
    }
}

impl From<CliManagedAgentKind> for AgentKind {
    fn from(value: CliManagedAgentKind) -> Self {
        match value {
            CliManagedAgentKind::Claude => AgentKind::Claude,
            CliManagedAgentKind::Codex => AgentKind::Codex,
            CliManagedAgentKind::OpenCode => AgentKind::OpenCode,
            CliManagedAgentKind::Pi => AgentKind::Pi,
            CliManagedAgentKind::Hermes => AgentKind::Hermes,
        }
    }
}

pub async fn run(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::List(args) => list_agents(args).await,
        AgentCommand::Sessions(args) => list_sessions(args).await,
        AgentCommand::Resume(args) => resume_session(args).await,
        AgentCommand::Scan { command } => run_scan(command).await,
        AgentCommand::Hook { command } => run_hook(command).await,
    }
}

async fn list_agents(args: AgentListArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let result: AgentListResult = api_get(&workdir, "/api/agents").await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", render_agent_list(&result));
    }
    Ok(())
}

async fn list_sessions(args: AgentSessionsArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let path = format!("/api/agents/{}/sessions", args.kind.slug());
    let result: AgentSessionsResult = api_get(&workdir, &path).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", render_agent_sessions(args.kind.into(), &result));
    }
    Ok(())
}

async fn run_scan(command: AgentScanCommand) -> Result<()> {
    match command {
        AgentScanCommand::Installed(args) => scan_installed(args),
        AgentScanCommand::List(args) => scan_list(args),
        AgentScanCommand::Sessions(args) => scan_sessions(args),
        AgentScanCommand::Bench(args) => scan_bench(args),
        AgentScanCommand::Usage(args) => scan_usage(args).await,
    }
}

fn scan_installed(args: AgentScanCommonArgs) -> Result<()> {
    let started = Instant::now();
    let result = agent::scan_installed_agents();
    emit_scan(
        ScanEmit {
            scan: "installed",
            workdir: None,
            limit: None,
            elapsed: started.elapsed(),
            json: args.json,
            quiet: args.quiet,
        },
        &result,
        render_installed_agents,
    )
}

fn scan_list(args: AgentScanWorkdirArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let started = Instant::now();
    let result = agent::scan_agent_list(&workdir);
    emit_scan(
        ScanEmit {
            scan: "list",
            workdir: Some(workdir),
            limit: None,
            elapsed: started.elapsed(),
            json: args.json,
            quiet: args.quiet,
        },
        &result,
        render_agent_list,
    )
}

fn scan_sessions(args: AgentScanSessionsArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let kind: AgentKind = args.kind.into();
    let started = Instant::now();
    let result = agent::scan_agent_sessions(kind, &workdir, args.limit);
    emit_scan(
        ScanEmit {
            scan: "sessions",
            workdir: Some(workdir),
            limit: Some(args.limit),
            elapsed: started.elapsed(),
            json: args.json,
            quiet: args.quiet,
        },
        &result,
        |value| render_agent_sessions(kind, value),
    )
}

#[derive(Serialize)]
struct ScanStepTiming {
    scan: String,
    elapsed_ms: u128,
}

#[derive(Serialize)]
struct ScanBenchReport {
    workdir: String,
    limit: u32,
    effective_limit: u32,
    total_elapsed_ms: u128,
    steps: Vec<ScanStepTiming>,
    installed: AgentInstalledListResult,
    list: AgentListResult,
    sessions: std::collections::HashMap<String, AgentSessionsResult>,
}

fn scan_bench(args: AgentScanBenchArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let effective_limit = agent::agent_session_limit(args.limit) as u32;
    let bench_started = Instant::now();
    let mut steps = Vec::new();

    let started = Instant::now();
    let installed = agent::scan_installed_agents();
    steps.push(ScanStepTiming {
        scan: "installed".to_string(),
        elapsed_ms: started.elapsed().as_millis(),
    });

    let started = Instant::now();
    let list = agent::scan_agent_list(&workdir);
    steps.push(ScanStepTiming {
        scan: "list".to_string(),
        elapsed_ms: started.elapsed().as_millis(),
    });

    let mut sessions = std::collections::HashMap::new();
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::OpenCode,
        AgentKind::Pi,
        AgentKind::Hermes,
        AgentKind::Maki,
    ] {
        let started = Instant::now();
        let result = agent::scan_agent_sessions(kind, &workdir, args.limit);
        steps.push(ScanStepTiming {
            scan: format!("sessions:{kind:?}"),
            elapsed_ms: started.elapsed().as_millis(),
        });
        sessions.insert(format!("{kind:?}"), result);
    }

    let total_elapsed = bench_started.elapsed();
    if !args.quiet {
        writeln!(
            stderr(),
            "scan bench completed in {}ms",
            total_elapsed.as_millis()
        )?;
    }

    if args.json {
        let report = ScanBenchReport {
            workdir: workdir.display().to_string(),
            limit: args.limit,
            effective_limit,
            total_elapsed_ms: total_elapsed.as_millis(),
            steps,
            installed,
            list,
            sessions,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Agent Scan Benchmark\n");
    println!(
        "{}",
        utils::render_key_values(&[
            ("Workdir", workdir.display().to_string()),
            (
                "Limit",
                if args.limit == 0 {
                    format!("default ({effective_limit})")
                } else {
                    args.limit.to_string()
                },
            ),
        ])
    );
    println!();
    println!(
        "{}",
        utils::render_table(
            &["SCAN", "ELAPSED"],
            &steps
                .iter()
                .map(|step| vec![step.scan.clone(), format!("{}ms", step.elapsed_ms)])
                .collect::<Vec<_>>(),
        )
    );
    println!();
    println!("Total: {}ms", total_elapsed.as_millis());
    println!();
    println!("{}", render_installed_agents(&installed));
    println!();
    println!("{}", render_agent_list(&list));
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::OpenCode,
        AgentKind::Pi,
        AgentKind::Hermes,
        AgentKind::Maki,
    ] {
        println!();
        if let Some(result) = sessions.get(&format!("{kind:?}")) {
            println!("{}", render_agent_sessions(kind, result));
        }
    }
    Ok(())
}

async fn scan_usage(args: AgentScanCommonArgs) -> Result<()> {
    let started = Instant::now();
    let (snapshots, plans) =
        tokio::join!(agent::scan_account_usage(), agent::scan_account_plans(),);
    let elapsed = started.elapsed();
    if !args.quiet {
        writeln!(
            stderr(),
            "scan usage completed in {}ms",
            elapsed.as_millis()
        )?;
    }
    if args.json {
        #[derive(serde::Serialize)]
        struct ScanUsageOutput<'a> {
            usage: &'a HashMap<AgentKind, AgentUsageSnapshot>,
            plans: &'a HashMap<AgentKind, Vec<AgentInfoField>>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&ScanUsageOutput {
                usage: &snapshots,
                plans: &plans,
            })?
        );
    } else {
        use zedra_rpc::proto::AgentKind::*;
        for kind in [Claude, Codex, OpenCode, Pi, Hermes, Maki] {
            let label = match kind {
                Claude => "Claude",
                Codex => "Codex",
                OpenCode => "OpenCode",
                Pi => "Pi",
                Hermes => "Hermes",
                Maki => "Maki",
            };
            match snapshots.get(&kind) {
                // Hermes/Maki have no remote usage/plan endpoint; absence is expected.
                None if kind == Hermes || kind == Maki => {
                    println!("{label}: local-only (no remote usage)")
                }
                None => println!("{label}: no credentials / fetch failed"),
                Some(snap) => {
                    println!("{label}:");
                    if let Some(fields) = plans.get(&kind) {
                        for field in fields {
                            if matches!(
                                field.label.as_str(),
                                "Plan" | "Plan until" | "Account" | "Organization" | "Logged in"
                            ) {
                                println!("  {}: {}", field.label, field.value);
                            }
                        }
                    }
                    if let Some(v) = snap.rate_limit_five_hour_used_percent {
                        println!("  5h rate limit:  {v:.1}%");
                    }
                    if let Some(v) = snap.rate_limit_seven_day_used_percent {
                        println!("  7d rate limit:  {v:.1}%");
                    }
                    if let Some(v) = snap.total_cost_usd {
                        println!("  Total cost:     ${v:.2}");
                    }
                    if let Some(v) = snap.context_used_percent {
                        println!("  Spend util:     {v:.1}%");
                    }
                    if let Some(v) = snap.lines_added {
                        println!("  Lines added:    {v}");
                    }
                    if let Some(v) = snap.lines_removed {
                        println!("  Lines removed:  {v}");
                    }
                }
            }
        }
    }
    Ok(())
}

struct ScanEmit {
    scan: &'static str,
    workdir: Option<PathBuf>,
    limit: Option<u32>,
    elapsed: Duration,
    json: bool,
    quiet: bool,
}

#[derive(Serialize)]
struct ScanEnvelope<'a, T: Serialize + 'a> {
    scan: &'static str,
    elapsed_ms: u128,
    workdir: Option<String>,
    limit: Option<u32>,
    effective_limit: Option<u32>,
    result: &'a T,
}

fn emit_scan<T: Serialize>(
    meta: ScanEmit,
    result: &T,
    render: impl FnOnce(&T) -> String,
) -> Result<()> {
    if !meta.quiet {
        writeln!(
            stderr(),
            "scan {}{} completed in {}ms",
            meta.scan,
            meta.workdir
                .as_ref()
                .map(|workdir| format!(" {}", workdir.display()))
                .unwrap_or_default(),
            meta.elapsed.as_millis()
        )?;
    }

    if meta.json {
        let envelope = ScanEnvelope {
            scan: meta.scan,
            elapsed_ms: meta.elapsed.as_millis(),
            workdir: meta
                .workdir
                .as_ref()
                .map(|workdir| workdir.display().to_string()),
            limit: meta.limit,
            effective_limit: meta
                .limit
                .map(|limit| agent::agent_session_limit(limit) as u32),
            result,
        };
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        println!("{}", render(result));
    }
    Ok(())
}

async fn resume_session(args: AgentResumeArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let path = format!("/api/agents/{}/resume", args.kind.slug());
    let body = serde_json::json!({
        "session_id": args.session_id,
        "cols": args.cols,
        "rows": args.rows,
    });
    let result: AgentResumeResult = api_post(&workdir, &path, &body).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if let Some(error) = result.error {
        bail!("failed to resume agent session: {error}");
    } else {
        println!(
            "Agent Session Resumed\n\n{}",
            utils::render_key_values(&[("Terminal", result.terminal_id)])
        );
    }
    Ok(())
}

async fn run_hook(command: AgentHookCommand) -> Result<()> {
    match command {
        AgentHookCommand::Install(args) => install_hooks(args),
        AgentHookCommand::Receive(args) => receive_hook(args).await,
        AgentHookCommand::Test(args) => test_hook(args).await,
    }
}

async fn receive_hook(args: AgentHookReceiveArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let raw = match args.payload {
        Some(p) => p,
        None => {
            let mut stdin = String::new();
            std::io::stdin()
                .read_to_string(&mut stdin)
                .context("failed to read hook payload from stdin")?;
            stdin
        }
    };
    let payload = parse_hook_payload(&raw);
    let slug = zedra_host::agent::managed_kind_from_slug(&args.agent)
        .map(zedra_host::agent_utils::program_name)
        .unwrap_or(args.agent.as_str());
    let body = AgentHookForwardReq {
        event_name: None,
        terminal_id: args.terminal_id,
        session_id: None,
        turn_id: None,
        tool_name: None,
        payload,
    };
    let path = format!("/api/agent-hooks/{slug}");
    let response: serde_json::Value = match api_post(&workdir, &path, &body).await {
        Ok(r) => r,
        Err(e) => {
            // Connection errors (no daemon) are always silent.
            // --quiet suppresses all other errors so agent hook noise never
            // surfaces in the working terminal.
            if is_connection_error(&e) || is_no_daemon_error(&e) || args.quiet {
                return Ok(());
            }
            return Err(e);
        }
    };
    if !args.quiet {
        println!("{}", serde_json::to_string_pretty(&response)?);
    }
    Ok(())
}

async fn test_hook(args: AgentHookTestArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let payload = synthetic_hook_payload(args.kind, &args.event, &workdir);
    let body = AgentHookForwardReq {
        event_name: Some(args.event),
        terminal_id: args.terminal_id,
        session_id: None,
        turn_id: None,
        tool_name: Some("Bash".to_string()),
        payload,
    };
    let path = format!("/api/agent-hooks/{}", args.kind.slug());
    let response: serde_json::Value = api_post(&workdir, &path, &body).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", render_hook_response(&response));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct AgentHookForwardReq {
    event_name: Option<String>,
    terminal_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    tool_name: Option<String>,
    payload: serde_json::Value,
}

fn install_hooks(args: AgentHookInstallArgs) -> Result<()> {
    if !zedra_host::agent_setup::hooks_enabled() {
        anyhow::bail!("agent hook install is disabled in release builds");
    }
    let workdir = resolve_workdir(&args.workdir);
    let providers = if args.providers.is_empty() {
        vec![
            CliManagedAgentKind::Claude,
            CliManagedAgentKind::Codex,
            CliManagedAgentKind::OpenCode,
            CliManagedAgentKind::Pi,
            CliManagedAgentKind::Hermes,
        ]
    } else {
        args.providers
    };
    let script_path = write_hook_script(&workdir, args.force)?;
    let mut written = vec![script_path.clone()];
    for provider in providers {
        match provider {
            CliManagedAgentKind::Claude => written.push(write_claude_hook_config(
                &workdir,
                &script_path,
                args.force,
            )?),
            CliManagedAgentKind::Codex => {
                written.push(write_codex_hook_config(&workdir, &script_path, args.force)?)
            }
            CliManagedAgentKind::OpenCode => written.push(write_opencode_hook_config(
                &workdir,
                &script_path,
                args.force,
            )?),
            CliManagedAgentKind::Pi => written.push(write_pi_hook_extension(args.force)?),
            CliManagedAgentKind::Hermes => written.extend(write_hermes_hook_config(args.force)?),
        }
    }

    println!("Local Agent Hooks Prepared\n");
    println!(
        "{}",
        utils::render_key_values(&[
            ("Workdir", workdir.display().to_string()),
            ("Hook Script", script_path.display().to_string()),
        ])
    );
    println!();
    println!("Files");
    for path in written {
        println!("  {}", path.display());
    }
    Ok(())
}

fn write_hook_script(workdir: &Path, force: bool) -> Result<PathBuf> {
    let path = workdir.join(".zedra/agent-hooks/zedra-agent-hook.sh");
    write_file_checked(
        &path,
        &hook_script_contents(workdir)?,
        force,
        "agent hook script",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)?;
    }
    Ok(path)
}

fn write_claude_hook_config(workdir: &Path, script_path: &Path, force: bool) -> Result<PathBuf> {
    let path = workdir.join(".claude/settings.local.json");
    let value = claude_hook_config(script_path);
    write_json_file_checked(&path, &value, force, "Claude local hook config")?;
    Ok(path)
}

fn claude_hook_config(script_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "Setup": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "Setup", Some("*")),
            "SessionStart": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "SessionStart", None),
            "InstructionsLoaded": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "InstructionsLoaded", Some("*")),
            "UserPromptSubmit": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "UserPromptSubmit", None),
            "UserPromptExpansion": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "UserPromptExpansion", Some("*")),
            "PreToolUse": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PreToolUse", Some("*")),
            "PermissionRequest": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PermissionRequest", Some("*")),
            "PermissionDenied": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PermissionDenied", Some("*")),
            "PostToolUse": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PostToolUse", Some("*")),
            "PostToolUseFailure": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PostToolUseFailure", Some("*")),
            "PostToolBatch": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PostToolBatch", None),
            "TaskCreated": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "TaskCreated", None),
            "TaskCompleted": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "TaskCompleted", None),
            "Stop": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "Stop", None),
            "StopFailure": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "StopFailure", None),
            "TeammateIdle": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "TeammateIdle", None),
            "ConfigChange": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "ConfigChange", Some("*")),
            "CwdChanged": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "CwdChanged", None),
            "WorktreeCreate": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "WorktreeCreate", None),
            "WorktreeRemove": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "WorktreeRemove", None),
            "PreCompact": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PreCompact", Some("*")),
            "PostCompact": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "PostCompact", Some("*")),
            "Elicitation": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "Elicitation", Some("*")),
            "ElicitationResult": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "ElicitationResult", Some("*")),
            "SessionEnd": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "SessionEnd", Some("*")),
            "Notification": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "Notification", Some("*"))
        }
    })
}

fn write_codex_hook_config(workdir: &Path, script_path: &Path, force: bool) -> Result<PathBuf> {
    let path = workdir.join(".codex/hooks.json");
    let value = codex_hook_config(script_path);
    write_json_file_checked(&path, &value, force, "Codex local hook config")?;
    Ok(path)
}

fn codex_hook_config(script_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "SessionStart": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "SessionStart", None),
            "UserPromptSubmit": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "UserPromptSubmit", None),
            "PermissionRequest": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "PermissionRequest", Some("*")),
            "PostToolUse": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "PostToolUse", Some("*")),
            "Stop": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "Stop", None)
        }
    })
}

fn write_opencode_hook_config(workdir: &Path, script_path: &Path, force: bool) -> Result<PathBuf> {
    let path = workdir.join(".opencode/plugins/zedra.js");
    let contents = format!(
        r#"const hookScript = {script};

async function send(event, payload = {{}}) {{
  const proc = Bun.spawn([hookScript], {{
    stdin: "pipe",
    stdout: "ignore",
    stderr: "ignore",
    env: {{
      ...process.env,
      ZEDRA_AGENT_KIND: "opencode",
      ZEDRA_AGENT_EVENT: event,
    }},
  }});
  proc.stdin.write(JSON.stringify({{ type: event, ...payload }}));
  proc.stdin.end();
  await proc.exited;
}}

export const ZedraPlugin = async () => ({{
  event: async (input) => {{
    await send(input.event?.type ?? "unknown", input.event ?? input);
  }},
  "tool.execute.before": async (input, output) => {{
    await send("tool.execute.before", {{ input, output }});
  }},
  "tool.execute.after": async (input, output) => {{
    await send("tool.execute.after", {{ input, output }});
  }},
  "shell.env": async (input, output) => {{
    output.env.ZEDRA_AGENT_KIND = "opencode";
    await send("shell.env", {{ input }});
  }},
}});
"#,
        script = serde_json::to_string(&script_path.display().to_string())?
    );
    write_file_checked(&path, &contents, force, "OpenCode local plugin")?;
    Ok(path)
}

fn write_pi_hook_extension(force: bool) -> Result<PathBuf> {
    let path = agent::home_path(&[".pi", "agent", "extensions", "zedra-agent-hooks.ts"]);
    let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
    let contents = pi_hook_extension_contents(&cli.display().to_string());
    write_file_checked(&path, &contents, force, "Pi hook extension")?;
    Ok(path)
}

fn pi_hook_extension_contents(cli_path: &str) -> String {
    let cli_json = serde_json::to_string(cli_path).unwrap_or_else(|_| format!("\"{}\"", cli_path));
    format!(
        r#"import type {{ ExtensionAPI }} from "@mariozechner/pi-coding-agent";
import {{ spawn }} from "node:child_process";

// Zedra hook extension for pi.
// Forwards pi lifecycle events to the Zedra daemon so the mobile app can show
// Running/Completed state and send Delta push notifications.
// Activates only when running inside a Zedra terminal (ZEDRA_TERMINAL_ID set).
// Outside Zedra it is a complete no-op. Failures are always swallowed so hook
// errors never affect the agent loop.
export default function (pi: ExtensionAPI) {{
  if (!process.env.ZEDRA_TERMINAL_ID) return;

  const CLI = process.env.ZEDRA_CLI || {cli};

  const fire = (hookEventName: string, sessionId?: string) => {{
    try {{
      const child = spawn(
        CLI,
        ["agent", "hook", "receive", "--agent", "pi", "--quiet"],
        {{
          stdio: ["pipe", "ignore", "ignore"],
          detached: true,
          // ZEDRA_TERMINAL_ID and ZEDRA_WORKDIR are inherited from process.env
          // and picked up by `agent hook receive` as --terminal-id / --workdir.
        }},
      );
      child.on("error", () => {{}});
      child.stdin?.on("error", () => {{}});
      const payload: Record<string, string> = {{ hook_event_name: hookEventName }};
      if (sessionId) payload.session_id = sessionId;
      child.stdin?.end(JSON.stringify(payload));
      child.unref();
    }} catch {{
      // spawn() can throw synchronously (EACCES, ENOENT). Stay silent.
    }}
  }};

  // Gate on ctx.hasUI: skip non-interactive (print / JSON / subagent) runs.
  // Check `=== false` so older pi versions without hasUI still fire hooks.
  const skip = (ctx: {{ hasUI?: boolean }}) => ctx.hasUI === false;

  pi.on("before_agent_start", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("UserPromptSubmit", (event as any)?.sessionId);
  }});

  pi.on("tool_execution_end", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("PostToolUse", (event as any)?.sessionId);
  }});

  pi.on("agent_end", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});

  // Fires on Ctrl+C, SIGTERM, /quit, /reload, /new, /resume, /fork.
  // Ensures Running indicator clears if pi is killed mid-turn.
  pi.on("session_shutdown", (event, ctx) => {{
    if (skip(ctx)) return;
    fire("Stop", (event as any)?.sessionId);
  }});
}}
"#,
        cli = cli_json
    )
}

/// Hermes lifecycle events Zedra hooks into via shell hooks.
const HERMES_HOOK_EVENTS: &[&str] = &[
    "on_session_start",
    "pre_approval_request",
    "post_approval_response",
    "post_llm_call",
    "on_session_end",
];

/// Writes the Hermes hook script and patches `~/.hermes/config.yaml`.
/// Returns the list of paths written/modified (hook script + config.yaml).
fn write_hermes_hook_config(force: bool) -> Result<Vec<PathBuf>> {
    let hermes_home = zedra_host::agent_hermes::hermes_home();
    let script_path = hermes_home.join("agent-hooks").join("zedra-agent-hooks.sh");
    let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
    let script = hermes_hook_script_contents(&cli.display().to_string());
    write_file_checked(&script_path, &script, force, "Hermes agent hook script")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms)?;
    }

    let config_path = hermes_home.join("config.yaml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let patched = patch_hermes_config_hooks(&existing, &script_path);
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&config_path, &patched)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(vec![script_path, config_path])
}

fn hermes_hook_script_contents(cli_path: &str) -> String {
    format!(
        r#"#!/bin/sh
# Zedra hook script for Hermes agent.
# No-op outside a Zedra terminal (ZEDRA_TERMINAL_ID not set by the shell).
[ -z "${{ZEDRA_TERMINAL_ID:-}}" ] && exit 0
CLI="${{ZEDRA_CLI:-}}"
[ -n "$CLI" ] || CLI={cli}
[ -x "$CLI" ] || CLI="zedra"
exec "$CLI" agent hook receive --agent hermes --quiet
"#,
        cli = utils::shell_arg(cli_path),
    )
}

/// Idempotently patches the `hooks:` block in `~/.hermes/config.yaml`.
///
/// Strategy:
/// - Zedra owns exactly the event keys listed in `HERMES_HOOK_EVENTS`.
/// - On every run (first install or re-run) we remove those keys from the
///   existing hooks block and re-insert fresh entries — so the path and
///   timeout are always up-to-date regardless of prior runs.
/// - All other hooks (user-defined) are preserved verbatim.
/// - Handles the default empty-dict form (`hooks: {}`) and the block form.
pub fn patch_hermes_config_hooks(config: &str, script_path: &Path) -> String {
    let script = script_path.display().to_string();

    let lines: Vec<&str> = config.lines().collect();
    let trailing_newline = config.ends_with('\n');

    // Find the top-level `hooks:` key (must start at column 0).
    let hooks_idx = lines.iter().position(|l| is_hooks_key_line(l));

    let Some(hooks_idx) = hooks_idx else {
        // No hooks: key — append our block to the file.
        let mut out = config.to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("hooks:\n");
        out.push_str(&zedra_hooks_entries(&script));
        return out;
    };

    // Is it the inline-empty form `hooks: {}` ?
    let inline_empty =
        lines[hooks_idx].trim() == "hooks: {}" || lines[hooks_idx].trim() == "hooks:{}";

    // Find the end of the hooks block: first subsequent top-level non-blank,
    // non-comment line (i.e. starts at column 0).
    let hooks_block_end = lines[hooks_idx + 1..]
        .iter()
        .position(|l| {
            !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#')
        })
        .map(|i| hooks_idx + 1 + i)
        .unwrap_or(lines.len());

    let pre = &lines[..hooks_idx];
    let post = &lines[hooks_block_end..];

    // Build the new hooks block.
    let mut hooks_block = String::from("hooks:\n");
    if !inline_empty {
        // Preserve non-Zedra event entries from the existing block.
        let existing_content = &lines[hooks_idx + 1..hooks_block_end];
        let preserved = remove_zedra_event_blocks(existing_content, HERMES_HOOK_EVENTS);
        if !preserved.trim().is_empty() {
            hooks_block.push_str(&preserved);
        }
    }
    hooks_block.push_str(&zedra_hooks_entries(&script));

    // Reconstruct the file.
    let mut out = String::new();
    for l in pre {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(&hooks_block);
    for l in post {
        out.push_str(l);
        out.push('\n');
    }
    // Normalise trailing newline to match original.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if trailing_newline && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// True when `line` is the top-level `hooks:` YAML key (column-0, no indent).
fn is_hooks_key_line(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let t = line.trim();
    t == "hooks: {}"
        || t == "hooks:{}"
        || t == "hooks:"
        || (t.starts_with("hooks:")
            && matches!(t.as_bytes().get(6), Some(b' ') | Some(b'{') | None))
}

/// Remove event blocks whose key is in `remove_events` from the indented
/// hooks content lines (the lines *after* the `hooks:` key).
///
/// An event block starts at a line with exactly 2-space indent followed by
/// an identifier and `:`, and continues until the next such line or EOF.
/// All other content (blank lines, comments, deeper-indented lines) is kept
/// or skipped depending on whether we are currently inside a removed block.
fn remove_zedra_event_blocks<'a>(lines: &[&'a str], remove_events: &[&str]) -> String {
    let mut out = String::new();
    let mut skip = false;

    for line in lines {
        if let Some(key) = event_key_at_line(line) {
            skip = remove_events.contains(&key);
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// If `line` is a 2-space-indented YAML mapping key (`  identifier:`),
/// return the key name; otherwise return `None`.
fn event_key_at_line<'a>(line: &'a str) -> Option<&'a str> {
    let rest = line.strip_prefix("  ")?;
    if rest.starts_with(' ') {
        return None; // deeper indent — not a top-level event key
    }
    let name = rest.strip_suffix(':')?;
    if name.is_empty() || name.contains(' ') || name.contains('"') || name.contains('\'') {
        return None;
    }
    Some(name)
}

/// YAML text for all Zedra-managed hook entries (2-space indented, ready to
/// append directly inside the `hooks:` block).
fn zedra_hooks_entries(script: &str) -> String {
    // Double any backslashes and escape double-quotes for inline YAML double-quoted scalar.
    let script_yaml = script.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = String::new();
    for event in HERMES_HOOK_EVENTS {
        out.push_str(&format!("  {}:\n", event));
        out.push_str(&format!("    - command: \"{}\"\n", script_yaml));
        out.push_str("      timeout: 5\n");
    }
    out
}

fn hook_groups(command: &str, matcher: Option<&str>) -> serde_json::Value {
    let mut group = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 5
        }]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = serde_json::Value::String(matcher.to_string());
    }
    serde_json::Value::Array(vec![group])
}

fn hook_groups_for_event(
    script_path: &Path,
    kind: CliManagedAgentKind,
    event_name: &str,
    matcher: Option<&str>,
) -> serde_json::Value {
    hook_groups(&hook_command(script_path, kind, event_name), matcher)
}

fn hook_command(script_path: &Path, kind: CliManagedAgentKind, event_expr: &str) -> String {
    format!(
        "ZEDRA_AGENT_KIND={} ZEDRA_AGENT_EVENT={} {}",
        kind.slug(),
        event_expr,
        utils::shell_arg_path(script_path)
    )
}

fn hook_script_contents(workdir: &Path) -> Result<String> {
    let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
    Ok(format!(
        r#"#!/bin/sh
set -eu

WORKDIR={workdir}
KIND="${{ZEDRA_AGENT_KIND:-}}"
EVENT="${{ZEDRA_AGENT_EVENT:-}}"
CLI="${{ZEDRA_CLI:-{cli}}}"

if [ -z "$KIND" ]; then
  echo "ZEDRA_AGENT_KIND is required" >&2
  exit 0
fi

if [ ! -x "$CLI" ]; then
  CLI="zedra"
fi

if [ -n "$EVENT" ]; then
  exec "$CLI" agent hook receive --workdir "$WORKDIR" --kind "$KIND" --event "$EVENT" --quiet
fi

exec "$CLI" agent hook receive --workdir "$WORKDIR" --kind "$KIND" --quiet
"#,
        workdir = utils::shell_arg_path(workdir),
        cli = utils::shell_arg_path(&cli),
    ))
}

fn write_json_file_checked(
    path: &Path,
    value: &serde_json::Value,
    force: bool,
    label: &str,
) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(value)?;
    contents.push('\n');
    write_file_checked(path, &contents, force, label)
}

fn write_file_checked(path: &Path, contents: &str, force: bool, label: &str) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{label} already exists at {}. Re-run with --force to overwrite it.",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn synthetic_hook_payload(
    kind: CliManagedAgentKind,
    event_name: &str,
    workdir: &Path,
) -> serde_json::Value {
    match kind {
        CliManagedAgentKind::Claude => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "hook_event_name": event_name,
                "session_id": "zedra-test-session",
                "transcript_path": "/tmp/zedra-test-session.jsonl",
                "cwd": cwd,
                "tool_name": "Bash",
                "tool_use_id": "toolu_zedra_test"
            })
        }
        CliManagedAgentKind::Codex => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "hook_event_name": event_name,
                "session_id": "zedra-test-session",
                "cwd": cwd,
                "tool_name": "Bash"
            })
        }
        CliManagedAgentKind::OpenCode => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "event": event_name,
                "sessionID": "zedra-test-session",
                "cwd": cwd,
                "tool": "bash"
            })
        }
        CliManagedAgentKind::Pi => {
            // The pi extension normalizes to Claude-compatible names; match that
            // wire so `agent hook test --agent pi` exercises the real receiver.
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "hook_event_name": event_name,
                "session_id": "zedra-test-session",
                "cwd": cwd,
            })
        }
        CliManagedAgentKind::Hermes => {
            // Hermes shell hooks use the same wire format as Claude/Codex.
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "hook_event_name": event_name,
                "session_id": "zedra-test-session",
                "cwd": cwd,
            })
        }
    }
}

fn parse_hook_payload(stdin: &str) -> serde_json::Value {
    let trimmed = stdin.trim();
    if trimmed.is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        serde_json::from_str(trimmed).unwrap_or_else(|_| {
            serde_json::json!({
                "raw": trimmed
            })
        })
    }
}

fn is_connection_error(err: &anyhow::Error) -> bool {
    err.chain().any(|e| {
        e.downcast_ref::<reqwest::Error>()
            .is_some_and(|re| re.is_connect())
    })
}

fn is_no_daemon_error(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|e| e.to_string().starts_with("No running daemon found"))
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

fn resolve_workdir(raw: &str) -> PathBuf {
    PathBuf::from(raw)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(raw))
}

fn render_installed_agents(result: &AgentInstalledListResult) -> String {
    let rows = result
        .agents
        .iter()
        .map(installed_agent_row)
        .collect::<Vec<_>>();
    let mut sections = vec!["Installed Agents".to_string(), String::new()];
    if rows.is_empty() {
        sections.push("No installed agents found.".to_string());
    } else {
        sections.push(utils::render_table(
            &["SLUG", "NAME", "AVAILABLE", "VERSION", "LAUNCH"],
            &rows,
        ));
    }
    if let Some(error) = &result.error {
        sections.push(String::new());
        sections.push(format!("Error: {error}"));
    }
    sections.join("\n")
}

fn installed_agent_row(agent: &InstalledAgentEntry) -> Vec<String> {
    vec![
        agent.slug.clone(),
        agent.display_name.clone(),
        agent.available.to_string(),
        agent.version.clone().unwrap_or_else(|| "-".to_string()),
        agent.launch_cmd.clone().unwrap_or_else(|| "-".to_string()),
    ]
}

fn render_agent_list(result: &AgentListResult) -> String {
    let rows = result
        .agents
        .iter()
        .map(agent_summary_row)
        .collect::<Vec<_>>();
    let mut sections = vec![
        "Managed Agents".to_string(),
        String::new(),
        utils::render_table(
            &[
                "KIND", "CLI", "SETUP", "SESSIONS", "LIVE", "LATEST", "WARNINGS",
            ],
            &rows,
        ),
    ];
    if let Some(error) = &result.error {
        sections.push(String::new());
        sections.push(format!("Error: {error}"));
    }
    sections.join("\n")
}

fn agent_summary_row(agent: &AgentSummary) -> Vec<String> {
    vec![
        agent.display_name.clone(),
        if agent.cli.available {
            agent
                .cli
                .version
                .clone()
                .unwrap_or_else(|| "available".to_string())
        } else {
            format!("missing{}", suffix(agent.cli.error.as_deref()))
        },
        format!("{:?}", agent.setup.state),
        format!("{}/{}", agent.sessions.resumable, agent.sessions.total),
        "-".to_string(),
        agent
            .sessions
            .latest_session_title
            .clone()
            .or_else(|| agent.sessions.latest_session_id.clone())
            .unwrap_or_else(|| "-".to_string()),
        if agent.warnings.is_empty() {
            "-".to_string()
        } else {
            agent
                .warnings
                .iter()
                .map(|warning| warning.code.clone())
                .collect::<Vec<_>>()
                .join(",")
        },
    ]
}

fn render_agent_sessions(kind: AgentKind, result: &AgentSessionsResult) -> String {
    let rows = result
        .sessions
        .iter()
        .map(agent_session_row)
        .collect::<Vec<_>>();
    let mut sections = vec![format!("{kind:?} Sessions"), String::new()];
    sections.push(format!(
        "Showing {} of {} workspace sessions",
        result.sessions.len(),
        result.total
    ));
    sections.push(String::new());
    if rows.is_empty() {
        sections.push("No sessions found for this workspace.".to_string());
    } else {
        sections.push(utils::render_table(
            &["ID", "TITLE", "CWD", "UPDATED", "RESUME"],
            &rows,
        ));
    }
    if let Some(error) = &result.error {
        sections.push(String::new());
        sections.push(format!("Error: {error}"));
    }
    sections.join("\n")
}

fn agent_session_row(session: &AgentSessionSummary) -> Vec<String> {
    vec![
        short_id(&session.session_id),
        utils::truncate_chars(
            session.title.as_deref().unwrap_or("-"),
            SESSION_TITLE_MAX_LEN,
        ),
        strip_home_path(session.cwd.as_deref().unwrap_or("-")),
        session
            .last_activity_at
            .map(format_session_time)
            .unwrap_or_else(|| "-".to_string()),
        if session.resume.available {
            session
                .resume
                .action_id
                .as_deref()
                .unwrap_or("available")
                .to_string()
        } else {
            session
                .resume
                .unavailable_reason
                .as_deref()
                .unwrap_or("no")
                .to_string()
        },
    ]
}

const SESSION_TITLE_MAX_LEN: usize = 60;

fn format_session_time(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M").to_string()
}

fn strip_home_path(path: &str) -> String {
    let Some(home) = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        return path.to_string();
    };
    let home = home.to_string_lossy();
    if path == home.as_ref() {
        return "~".to_string();
    }
    let prefix = format!("{home}/");
    if let Some(rest) = path.strip_prefix(&prefix) {
        return format!("~/{rest}");
    }
    path.to_string()
}

fn render_hook_response(response: &serde_json::Value) -> String {
    if response["ok"].as_bool() == Some(true) {
        "Agent hook delivered to daemon.".to_string()
    } else {
        format!("Agent hook failed: {response}")
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn suffix(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_hook_payload() {
        assert_eq!(parse_hook_payload(""), serde_json::json!({}));
    }

    #[test]
    fn parses_json_hook_payload() {
        assert_eq!(
            parse_hook_payload(r#"{"hook_event_name":"Stop"}"#),
            serde_json::json!({"hook_event_name": "Stop"})
        );
    }

    #[test]
    fn hook_command_uses_provider_env() {
        let command = hook_command(
            Path::new("/tmp/zedra hook.sh"),
            CliManagedAgentKind::Claude,
            "Stop",
        );
        assert!(command.contains("ZEDRA_AGENT_KIND=claude"));
        assert!(command.contains("ZEDRA_AGENT_EVENT=Stop"));
        assert!(command.contains("'/tmp/zedra hook.sh'"));
    }

    #[test]
    fn claude_hook_config_includes_turn_tool_and_session_events() {
        let config = claude_hook_config(Path::new("/tmp/zedra-hook.sh"));
        let hooks = config["hooks"].as_object().unwrap();

        for event in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolBatch",
            "PermissionDenied",
            "SessionEnd",
        ] {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert!(!hooks.contains_key("SubagentStart"));
        assert!(!hooks.contains_key("SubagentStop"));
    }

    #[test]
    fn codex_hook_config_includes_prompt_submit() {
        let config = codex_hook_config(Path::new("/tmp/zedra-hook.sh"));
        let hooks = config["hooks"].as_object().unwrap();

        for event in [
            "UserPromptSubmit",
            "PermissionRequest",
            "PostToolUse",
            "Stop",
        ] {
            assert!(hooks.contains_key(event), "missing {event}");
        }
    }

    #[test]
    fn strip_home_path_replaces_home_prefix_with_tilde() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(strip_home_path(&home), "~");
        assert_eq!(
            strip_home_path(&format!("{home}/projects/zedra-main")),
            "~/projects/zedra-main"
        );
        assert_eq!(strip_home_path("/other/path"), "/other/path");
    }

    #[test]
    fn truncate_session_title_caps_display_width() {
        let title = "A".repeat(SESSION_TITLE_MAX_LEN + 10);
        let truncated = utils::truncate_chars(&title, SESSION_TITLE_MAX_LEN);
        assert!(truncated.ends_with('…'));
        assert_eq!(truncated.chars().count(), SESSION_TITLE_MAX_LEN + 1);
    }

    #[test]
    fn format_session_time_uses_compact_local_table_format() {
        let time = DateTime::parse_from_rfc3339("2026-05-21T08:07:35+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(format_session_time(time), "2026-05-21 08:07");
    }

    // -------------------------------------------------------------------------
    // patch_hermes_config_hooks
    // -------------------------------------------------------------------------

    #[test]
    fn patch_hermes_hooks_expands_inline_empty_dict() {
        let config = "model:\n  default: gpt-5\nhooks: {}\nhooks_auto_accept: false\n";
        let script = std::path::Path::new("/home/user/.hermes/agent-hooks/zedra-agent-hooks.sh");
        let patched = patch_hermes_config_hooks(config, script);
        assert!(patched.contains("hooks:\n"));
        assert!(!patched.contains("hooks: {}"));
        for event in HERMES_HOOK_EVENTS {
            assert!(patched.contains(event), "missing event {event}");
        }
        assert!(patched.contains("zedra-agent-hooks.sh"));
        // Non-hooks keys must be preserved verbatim.
        assert!(patched.contains("model:\n  default: gpt-5\n"));
        assert!(patched.contains("hooks_auto_accept: false\n"));
    }

    #[test]
    fn hermes_hook_script_preserves_quoted_binary_path() {
        let script = hermes_hook_script_contents("/tmp/zedra build/zedra");
        assert!(script.contains("CLI=\"${ZEDRA_CLI:-}\""));
        assert!(script.contains("[ -n \"$CLI\" ] || CLI='/tmp/zedra build/zedra'"));
        assert!(!script.contains("CLI=\"${ZEDRA_CLI:-'"));
    }

    #[test]
    fn patch_hermes_hooks_idempotent_on_rerun() {
        let config = "hooks: {}\n";
        let script = std::path::Path::new("/path/to/zedra-agent-hooks.sh");
        let first = patch_hermes_config_hooks(config, script);
        let second = patch_hermes_config_hooks(&first, script);
        // Re-running must produce the same output (same set of event keys,
        // no duplicates, script path preserved).
        assert_eq!(first, second);
    }

    #[test]
    fn patch_hermes_hooks_overrides_old_script_path() {
        let old_script = std::path::Path::new("/old/path/zedra-agent-hooks.sh");
        let new_script = std::path::Path::new("/new/path/zedra-agent-hooks.sh");
        let after_first = patch_hermes_config_hooks("hooks: {}\n", old_script);
        assert!(after_first.contains("/old/path/"));
        let after_second = patch_hermes_config_hooks(&after_first, new_script);
        // Old path gone, new path present.
        assert!(
            !after_second.contains("/old/path/"),
            "old path still present"
        );
        assert!(after_second.contains("/new/path/"));
        for event in HERMES_HOOK_EVENTS {
            let count = after_second.matches(event).count();
            assert_eq!(count, 1, "event {event} appears {count} times (want 1)");
        }
    }

    #[test]
    fn patch_hermes_hooks_preserves_user_events() {
        let config = "hooks:\n  user_event:\n    - command: user_script.sh\n      timeout: 10\n";
        let script = std::path::Path::new("/path/zedra-agent-hooks.sh");
        let patched = patch_hermes_config_hooks(config, script);
        assert!(patched.contains("user_event:"), "user event must survive");
        assert!(
            patched.contains("user_script.sh"),
            "user script must survive"
        );
        for event in HERMES_HOOK_EVENTS {
            assert!(patched.contains(event), "missing zedra event {event}");
        }
    }

    #[test]
    fn patch_hermes_hooks_no_hooks_key_appends_block() {
        let config = "model:\n  default: gpt-5\n";
        let script = std::path::Path::new("/path/zedra-agent-hooks.sh");
        let patched = patch_hermes_config_hooks(config, script);
        assert!(patched.starts_with("model:"));
        assert!(patched.contains("hooks:\n"));
        for event in HERMES_HOOK_EVENTS {
            assert!(patched.contains(event));
        }
    }
}
