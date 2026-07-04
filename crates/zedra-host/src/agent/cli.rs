use crate::agent;
use crate::{identity, utils};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{stderr, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zedra_rpc::proto::{
    AgentInfoField, AgentInstalledListResult, AgentListResult, AgentResumeResult,
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
    kind: String,
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
    kind: String,
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
    kind: String,
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
    #[arg(long = "provider")]
    providers: Vec<String>,
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
    kind: String,
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

fn actor_slug(raw: &str) -> Result<&'static str> {
    agent::actor(raw)
        .map(|actor| actor.slug())
        .ok_or_else(|| anyhow::anyhow!("unsupported agent: {raw}"))
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
    let slug = actor_slug(&args.kind)?;
    let path = format!("/api/agents/{slug}/sessions");
    let result: AgentSessionsResult = api_get(&workdir, &path).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", render_agent_sessions(slug, &result));
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
    let slug = actor_slug(&args.kind)?;
    let started = Instant::now();
    let result = agent::scan_agent_sessions(slug, &workdir, args.limit);
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
        |value| render_agent_sessions(slug, value),
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
    for actor in agent::actors() {
        let slug = actor.slug();
        let started = Instant::now();
        let result = agent::scan_agent_sessions(slug, &workdir, args.limit);
        steps.push(ScanStepTiming {
            scan: format!("sessions:{slug}"),
            elapsed_ms: started.elapsed().as_millis(),
        });
        sessions.insert(slug.to_string(), result);
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
    for actor in agent::actors() {
        let slug = actor.slug();
        println!();
        if let Some(result) = sessions.get(slug) {
            println!("{}", render_agent_sessions(slug, result));
        }
    }
    Ok(())
}

/// `scan usage`: probe each actor concurrently, time usage and plan separately,
/// report per agent keyed by slug.
async fn scan_usage(args: AgentScanCommonArgs) -> Result<()> {
    let started = Instant::now();

    let mut tasks = tokio::task::JoinSet::new();
    for actor in agent::actors() {
        tasks.spawn(async move {
            // Time usage and plan independently while running them concurrently.
            let usage_fut = async {
                let at = Instant::now();
                (actor.account_usage().await, at.elapsed())
            };
            let plan_fut = async {
                let at = Instant::now();
                (actor.subscription_plan().await, at.elapsed())
            };
            let ((usage, usage_ms), (plan, plan_ms)) = tokio::join!(usage_fut, plan_fut);
            (
                actor.slug(),
                actor.display_name(),
                usage,
                plan,
                usage_ms,
                plan_ms,
            )
        });
    }

    let mut agents: std::collections::BTreeMap<&'static str, ScanUsageAgent> =
        std::collections::BTreeMap::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((slug, display_name, usage, plan, usage_ms, plan_ms)) = joined else {
            continue;
        };
        let plan = plan.unwrap_or_default();
        // Skip actors with nothing to report (the detect-only majority).
        if usage.is_none() && plan.is_empty() {
            continue;
        }
        agents.insert(
            slug,
            ScanUsageAgent {
                display_name,
                usage,
                plan,
                timings_ms: ScanUsageTimings {
                    usage: usage_ms.as_millis() as u64,
                    plan: plan_ms.as_millis() as u64,
                },
            },
        );
    }

    let total_ms = started.elapsed().as_millis();
    if !args.quiet {
        writeln!(stderr(), "scan usage completed in {total_ms}ms")?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ScanUsageOutput {
                scan: "usage",
                agents: &agents,
                total_ms,
            })?
        );
    } else {
        for (slug, agent) in &agents {
            println!(
                "{} ({slug})  [usage {}ms · plan {}ms]",
                agent.display_name, agent.timings_ms.usage, agent.timings_ms.plan
            );
            for field in &agent.plan {
                if matches!(
                    field.label.as_str(),
                    "Plan" | "Plan until" | "Account" | "Organization" | "Logged in"
                ) {
                    println!("  {}: {}", field.label, field.value);
                }
            }
            match &agent.usage {
                None => println!("  no remote usage available"),
                Some(snap) => {
                    if let Some(v) = snap.rate_limit_five_hour_used_percent {
                        println!("  5h rate limit:  {v:.1}%");
                    }
                    if let Some(v) = snap.rate_limit_seven_day_used_percent {
                        println!("  7d rate limit:  {v:.1}%");
                    }
                    for field in &snap.extra {
                        println!("  {}: {}", field.label, field.value);
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ScanUsageTimings {
    usage: u64,
    plan: u64,
}

#[derive(Serialize)]
struct ScanUsageAgent {
    display_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<AgentUsageSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    plan: Vec<AgentInfoField>,
    timings_ms: ScanUsageTimings,
}

#[derive(Serialize)]
struct ScanUsageOutput<'a> {
    scan: &'static str,
    agents: &'a std::collections::BTreeMap<&'static str, ScanUsageAgent>,
    total_ms: u128,
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
    let slug = actor_slug(&args.kind)?;
    let path = format!("/api/agents/{slug}/resume");
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
    let slug = agent::actor(&args.agent)
        .map(|actor| actor.slug())
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
    let actor = agent::actor(&args.kind)
        .ok_or_else(|| anyhow::anyhow!("unsupported agent: {}", args.kind))?;
    let slug = actor.slug();
    let payload = actor.hook_test_payload(&args.event, &workdir);
    let body = AgentHookForwardReq {
        event_name: Some(args.event),
        terminal_id: args.terminal_id,
        session_id: None,
        turn_id: None,
        tool_name: Some("Bash".to_string()),
        payload,
    };
    let path = format!("/api/agent-hooks/{slug}");
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
    if !agent::hooks_enabled() {
        anyhow::bail!("agent hook install is disabled in release builds");
    }
    let workdir = resolve_workdir(&args.workdir);
    let script_path = workdir.join(".zedra/agent-hooks/zedra-agent-hook.sh");
    let install_all = args.providers.is_empty();
    let actors = if install_all {
        agent::actors().to_vec()
    } else {
        args.providers
            .iter()
            .map(|slug| {
                actor_slug(slug).and_then(|slug| {
                    agent::actor(slug).ok_or_else(|| anyhow::anyhow!("unsupported agent: {slug}"))
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    let written = install_hook_actors(&actors, &workdir, args.force, install_all)?;
    // Show the workspace hook script line only when an actor prepared it (pi/hermes don't).
    let has_local_script = written.iter().any(|path| path == &script_path);

    println!("Local Agent Hooks Prepared\n");
    let summary = if has_local_script {
        utils::render_key_values(&[
            ("Workdir", workdir.display().to_string()),
            ("Hook Script", script_path.display().to_string()),
        ])
    } else {
        utils::render_key_values(&[("Workdir", workdir.display().to_string())])
    };
    println!("{summary}");
    println!();
    println!("Files");
    for path in written {
        println!("  {}", path.display());
    }
    Ok(())
}

fn install_hook_actors(
    actors: &[&dyn agent::AgentActor],
    workdir: &Path,
    force: bool,
    skip_unsupported: bool,
) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for actor in actors {
        if !actor.supports_setup() {
            if skip_unsupported {
                tracing::debug!(agent = actor.slug(), "agent has no hook installer");
                continue;
            }
            bail!("{} does not support setup", actor.slug());
        }

        // Default install mode skips unsupported actors, but supported actor setup
        // failures mean hooks were requested and not installed.
        let mut paths = actor.setup(workdir, force)?;
        written.append(&mut paths);
    }
    Ok(written)
}

pub(crate) fn write_hook_script(workdir: &Path, force: bool) -> Result<PathBuf> {
    let path = workdir.join(".zedra/agent-hooks/zedra-agent-hook.sh");
    // Skip the rewrite when the script already exists, but still repair the exec
    // bit below so a script that lost +x doesn't leave a non-runnable hook.
    if !path.exists() || force {
        super::utils::write_file_checked(
            &path,
            &hook_script_contents(workdir)?,
            force,
            "agent hook script",
        )?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)?;
    }
    Ok(path)
}

fn hook_script_contents(workdir: &Path) -> Result<String> {
    let cli = std::env::current_exe().context("failed to resolve current zedra binary")?;
    Ok(format!(
        r#"#!/bin/sh
set -eu

WORKDIR={workdir}
KIND="${{ZEDRA_AGENT_KIND:-}}"
CLI="${{ZEDRA_CLI:-{cli}}}"

if [ -z "$KIND" ]; then
  echo "ZEDRA_AGENT_KIND is required" >&2
  exit 0
fi

if [ ! -x "$CLI" ]; then
  CLI="zedra"
fi

# The receive CLI only accepts --agent; event names ride in the stdin payload.
exec "$CLI" agent hook receive --workdir "$WORKDIR" --agent "$KIND" --quiet
"#,
        workdir = utils::shell_arg_path(workdir),
        cli = utils::shell_arg_path(&cli),
    ))
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

fn render_agent_sessions(slug: &str, result: &AgentSessionsResult) -> String {
    let rows = result
        .sessions
        .iter()
        .map(agent_session_row)
        .collect::<Vec<_>>();
    let mut sections = vec![format!("{slug} Sessions"), String::new()];
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

    struct UnsupportedSetupActor;

    impl agent::AgentActor for UnsupportedSetupActor {
        fn slug(&self) -> &'static str {
            "unsupported"
        }

        fn display_name(&self) -> &'static str {
            "Unsupported"
        }

        fn icon_name(&self) -> &'static str {
            "unsupported"
        }
    }

    struct FailingSetupActor;

    impl agent::AgentActor for FailingSetupActor {
        fn slug(&self) -> &'static str {
            "failing"
        }

        fn display_name(&self) -> &'static str {
            "Failing"
        }

        fn icon_name(&self) -> &'static str {
            "failing"
        }

        fn supports_setup(&self) -> bool {
            true
        }

        fn setup(&self, _workdir: &Path, _force: bool) -> anyhow::Result<Vec<PathBuf>> {
            bail!("setup failed")
        }
    }

    #[test]
    fn install_hook_actors_reports_supported_setup_failures() {
        let temp = tempfile::tempdir().unwrap();
        let unsupported = UnsupportedSetupActor;
        let failing = FailingSetupActor;
        let actors: [&dyn agent::AgentActor; 2] = [&unsupported, &failing];

        let error = install_hook_actors(&actors, temp.path(), false, true).unwrap_err();

        assert!(
            error.to_string().contains("setup failed"),
            "unexpected error: {error:#}"
        );
    }

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
}
