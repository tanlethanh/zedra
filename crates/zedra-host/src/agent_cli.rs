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
    AgentInfoField, AgentInstalledListResult, AgentListResult, AgentResumeResult,
    AgentSessionSummary, AgentSessionsResult, AgentSummary, AgentUsageSnapshot,
    InstalledAgentEntry, AgentKind,
};

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List managed-agent summaries for this workspace
    List(AgentListArgs),
    /// List sessions for one managed agent
    Sessions(AgentSessionsArgs),
    /// Resume an agent session in a new phone terminal
    Resume(AgentResumeArgs),
    /// Listen for hook events bound to one terminal id
    Listen(AgentListenArgs),
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

#[derive(Debug, Args)]
pub struct AgentListenArgs {
    /// Zedra terminal id to listen on
    #[arg(long = "tid", alias = "terminal-id")]
    terminal_id: String,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Read the current buffered events and exit
    #[arg(long)]
    once: bool,
    /// Poll interval while following events
    #[arg(long, default_value_t = 1)]
    interval_secs: u64,
    /// Print one JSON object per event
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
    /// Agent provider that emitted the hook
    #[arg(long, value_enum)]
    kind: CliManagedAgentKind,
    /// Provider event name. If omitted, zedra tries to infer it from stdin JSON.
    #[arg(long)]
    event: Option<String>,
    /// Working directory of the running daemon
    #[arg(short, long, default_value = ".")]
    workdir: String,
    /// Override the Zedra terminal id. Defaults to ZEDRA_TERMINAL_ID.
    #[arg(long)]
    terminal_id: Option<String>,
    /// Provider session id, when the hook exposes it
    #[arg(long)]
    session_id: Option<String>,
    /// Provider turn id, when the hook exposes it
    #[arg(long)]
    turn_id: Option<String>,
    /// Tool name, when the hook exposes it
    #[arg(long)]
    tool_name: Option<String>,
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
        AgentCommand::Listen(args) => listen_events(args).await,
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
        for kind in [Claude, Codex, OpenCode, Pi, Hermes] {
            let label = match kind {
                Claude => "Claude",
                Codex => "Codex",
                OpenCode => "OpenCode",
                Pi => "Pi",
                Hermes => "Hermes",
            };
            match snapshots.get(&kind) {
                // Hermes has no remote usage/plan endpoint; absence is expected.
                None if kind == Hermes => println!("{label}: local-only (no remote usage)"),
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

async fn listen_events(args: AgentListenArgs) -> Result<()> {
    let workdir = resolve_workdir(&args.workdir);
    let mut after = 0;
    if !args.json && !args.once {
        println!(
            "Listening for agent hook events on terminal {}",
            args.terminal_id
        );
    }

    loop {
        let path = format!(
            "/api/agent-hooks/events?terminal_id={}&after={after}&limit=100",
            query_encode(&args.terminal_id)
        );
        let response: serde_json::Value = api_get(&workdir, &path).await?;
        let events = response["events"].as_array().cloned().unwrap_or_default();

        for event in &events {
            if let Some(seq) = event["seq"].as_u64() {
                after = after.max(seq);
            }
            if args.json {
                println!("{}", serde_json::to_string(event)?);
            } else {
                println!("{}", render_hook_event(event));
            }
        }

        if args.once {
            if events.is_empty() && !args.json {
                println!(
                    "No agent hook events buffered for terminal {}.",
                    args.terminal_id
                );
            }
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(args.interval_secs.max(1))).await;
    }
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
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("failed to read hook payload from stdin")?;
    let payload = parse_hook_payload(&stdin);
    let body = AgentHookForwardReq {
        event_name: args.event,
        terminal_id: args
            .terminal_id
            .or_else(|| std::env::var("ZEDRA_TERMINAL_ID").ok()),
        session_id: args.session_id,
        turn_id: args.turn_id,
        tool_name: args.tool_name,
        payload,
    };
    let path = format!("/api/agent-hooks/{}", args.kind.slug());
    let response: serde_json::Value = api_post(&workdir, &path, &body).await?;
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
            CliManagedAgentKind::Pi => {
                eprintln!("warning: Pi has no documented hook system; skipping");
            }
            CliManagedAgentKind::Hermes => {
                eprintln!("warning: Zedra hook install for Hermes is not supported; skipping");
            }
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
            "SubagentStart": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "SubagentStart", Some("*")),
            "SubagentStop": hook_groups_for_event(script_path, CliManagedAgentKind::Claude, "SubagentStop", Some("*")),
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
    let value = serde_json::json!({
        "hooks": {
            "SessionStart": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "SessionStart", None),
            "PermissionRequest": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "PermissionRequest", Some("*")),
            "PostToolUse": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "PostToolUse", Some("*")),
            "Stop": hook_groups_for_event(script_path, CliManagedAgentKind::Codex, "Stop", None)
        }
    });
    write_json_file_checked(&path, &value, force, "Codex local hook config")?;
    Ok(path)
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
                "type": event_name,
                "sessionID": "zedra-test-session",
                "cwd": cwd,
                "tool": "bash"
            })
        }
        CliManagedAgentKind::Pi => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "event": event_name,
                "sessionId": "zedra-test-session",
                "cwd": cwd,
            })
        }
        CliManagedAgentKind::Hermes => {
            let cwd = workdir.to_string_lossy();
            serde_json::json!({
                "event": event_name,
                "sessionId": "zedra-test-session",
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
        format!(
            "{} term, {} pending",
            agent.live.active_terminal_ids.len(),
            agent.live.pending_action_count
        ),
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
    let normalized = response
        .get("normalized")
        .and_then(|value| value.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let status = response
        .get("normalized")
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let rows = vec![
        (
            "Seq",
            response["seq"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        (
            "Provider",
            response["kind"].as_str().unwrap_or("-").to_string(),
        ),
        (
            "Event",
            response["provider_event_name"]
                .as_str()
                .unwrap_or("-")
                .to_string(),
        ),
        (
            "Provider IDs",
            render_provider_ids(&response["provider_ids"]),
        ),
        ("Normalized", normalized.to_string()),
        ("Status", status.to_string()),
        (
            "Terminal Bound",
            response["terminal_bound"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        (
            "Warning",
            response["warning"].as_str().unwrap_or("-").to_string(),
        ),
    ];
    format!("Agent Hook Received\n\n{}", utils::render_key_values(&rows))
}

fn render_hook_event(event: &serde_json::Value) -> String {
    let normalized = event.get("normalized");
    let kind = normalized
        .and_then(|value| value.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let status = normalized
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let session = normalized
        .and_then(|value| value.get("session_id"))
        .and_then(serde_json::Value::as_str);
    let turn = normalized
        .and_then(|value| value.get("turn_id"))
        .and_then(serde_json::Value::as_str);
    let tool = normalized
        .and_then(|value| value.get("tool_name"))
        .and_then(serde_json::Value::as_str);
    let provider_ids = render_provider_ids(&event["provider_ids"]);
    let warning = event["warning"].as_str();
    let mut suffix = Vec::new();
    push_label(&mut suffix, "session", session);
    push_label(&mut suffix, "turn", turn);
    push_label(&mut suffix, "tool", tool);
    if provider_ids != "-" {
        suffix.push(format!("ids={provider_ids}"));
    }
    push_label(&mut suffix, "warning", warning);

    let base = format!(
        "[{}] {} {} -> {} ({})",
        event["seq"]
            .as_u64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        event["kind"].as_str().unwrap_or("-"),
        event["provider_event_name"].as_str().unwrap_or("-"),
        kind,
        status
    );
    if suffix.is_empty() {
        base
    } else {
        format!("{base} {}", suffix.join(" "))
    }
}

fn push_label(parts: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        parts.push(format!("{label}={value}"));
    }
}

fn render_provider_ids(provider_ids: &serde_json::Value) -> String {
    let Some(provider_ids) = provider_ids.as_object() else {
        return "-".to_string();
    };
    let mut parts = Vec::new();
    for (key, label) in [
        ("turn_id", "turn"),
        ("tool_use_id", "tool_use"),
        ("task_id", "task"),
        ("agent_id", "agent"),
        ("elicitation_id", "elicitation"),
        ("transcript_id", "transcript"),
    ] {
        if let Some(value) = provider_ids
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("{label}={value}"));
        }
    }
    if let Some(values) = provider_ids
        .get("batch_tool_use_ids")
        .and_then(serde_json::Value::as_array)
    {
        let values = values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if !values.is_empty() {
            parts.push(format!("batch_tools={}", values.join(",")));
        }
    }

    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(" ")
    }
}

fn query_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
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
            "SubagentStart",
            "SessionEnd",
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

    #[test]
    fn hook_event_output_includes_terminal_bound_event_fields() {
        let event = serde_json::json!({
            "seq": 9,
            "kind": "Claude",
            "provider_event_name": "TaskCompleted",
            "normalized": {
                "kind": "TaskCompleted",
                "status": "Completed",
                "session_id": "session-1",
                "turn_id": "turn-1",
                "tool_name": "Bash"
            },
            "provider_ids": {
                "task_id": "task-1",
                "batch_tool_use_ids": []
            },
            "warning": null
        });

        let output = render_hook_event(&event);
        assert!(output.contains("[9] Claude TaskCompleted -> TaskCompleted (Completed)"));
        assert!(output.contains("session=session-1"));
        assert!(output.contains("ids=task=task-1"));
        assert!(!output.contains("corr="));
        assert!(!output.contains("warning=-"));
    }
}
