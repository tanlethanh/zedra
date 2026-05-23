use crate::agent_cache::AgentCache;
use crate::claude;
use crate::installed_agents;
use crate::session_registry::{HostShellState, HostTermMeta, ServerSession};
use crate::sqlite_readonly;
use crate::utils;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use zedra_rpc::proto::*;

const SKILL_NAMES: &[&str] = &[
    "zedra-start",
    "zedra-status",
    "zedra-stop",
    "zedra-terminal",
];

const AGENT_SESSION_DEFAULT_LIMIT: u32 = 50;
const AGENT_SESSION_MAX_LIMIT: u32 = 200;

pub const MANAGED_AGENT_KINDS: [ManagedAgentKind; 3] = [
    ManagedAgentKind::Claude,
    ManagedAgentKind::Codex,
    ManagedAgentKind::OpenCode,
];

pub fn default_agent_session_limit() -> usize {
    agent_session_limit(0)
}

pub fn agent_session_limit(limit: u32) -> usize {
    let configured = std::env::var("ZEDRA_AGENT_SESSION_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(AGENT_SESSION_DEFAULT_LIMIT);
    let limit = if limit == 0 { configured } else { limit };
    limit.clamp(1, AGENT_SESSION_MAX_LIMIT) as usize
}

pub fn scan_installed_agents() -> AgentInstalledListResult {
    installed_agents::list_installed_agents()
}

pub fn scan_managed_agent_cli_versions() -> HashMap<ManagedAgentKind, AgentCliSummary> {
    let mut versions = HashMap::with_capacity(MANAGED_AGENT_KINDS.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = MANAGED_AGENT_KINDS
            .iter()
            .copied()
            .map(|kind| (kind, scope.spawn(move || cli_version_summary(kind))))
            .collect();
        for (kind, handle) in handles {
            match handle.join() {
                Ok(summary) => {
                    versions.insert(kind, summary);
                }
                Err(_) => {
                    tracing::warn!("managed agent version probe panicked for {kind:?}");
                }
            }
        }
    });
    versions
}

pub fn apply_cached_cli_versions(
    agents: &mut [AgentSummary],
    versions: &HashMap<ManagedAgentKind, AgentCliSummary>,
) {
    for agent in agents {
        let Some(cached) = versions.get(&agent.kind) else {
            continue;
        };
        if !agent.cli.available {
            continue;
        }
        agent.cli.version = cached.version.clone();
        if let Some(error) = cached.error.as_ref() {
            agent.cli.error = Some(error.clone());
        }
    }
}

pub fn scan_agent_list(workdir: &Path) -> AgentListResult {
    let mut agents = Vec::with_capacity(MANAGED_AGENT_KINDS.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = MANAGED_AGENT_KINDS
            .iter()
            .copied()
            .map(|kind| (kind, scope.spawn(move || agent_summary_scan(kind, workdir))))
            .collect();
        for (kind, handle) in handles {
            match handle.join() {
                Ok(summary) => agents.push(summary),
                // A panic while scanning one provider must not abort the whole
                // list; degrade that provider and keep the rest.
                Err(_) => {
                    tracing::warn!(
                        "agent list scan thread panicked for {kind:?}; using degraded summary"
                    );
                    agents.push(degraded_agent_summary(kind, workdir));
                }
            }
        }
    });
    AgentListResult {
        agents,
        error: None,
    }
}

/// Minimal `AgentSummary` used when a provider's session scan panics. Reuses
/// only the cheap CLI/setup probes and reports zero counts with a warning.
fn degraded_agent_summary(kind: ManagedAgentKind, workdir: &Path) -> AgentSummary {
    let cli = agent_list_cli_summary(kind, workdir);
    let setup = setup_summary(kind, cli.available, workdir);
    AgentSummary {
        kind,
        display_name: display_name(kind).to_string(),
        cli,
        setup,
        capabilities: capabilities(kind),
        workspace: AgentWorkspaceSummary {
            workdir: workdir.to_string_lossy().into_owned(),
            provider_project_id: None,
            provider_project_key: None,
        },
        sessions: AgentSessionCounts {
            total: 0,
            resumable: 0,
            active_live: 0,
            latest_session_id: None,
            latest_session_title: None,
        },
        live: AgentLiveSummary {
            active_terminal_ids: Vec::new(),
            pending_action_count: 0,
            latest_event: None,
        },
        last_activity_at: None,
        updated_at: Utc::now(),
        data_sources: vec![AgentDataSource::Cli, AgentDataSource::Setup],
        warnings: vec![AgentWarning {
            code: "session_scan_panicked".to_string(),
            message: "agent session scan crashed; counts unavailable".to_string(),
        }],
        account: account_summary(kind),
        usage: None,
    }
}

pub fn scan_agent_sessions(
    kind: ManagedAgentKind,
    workdir: &Path,
    limit: u32,
) -> AgentSessionsResult {
    let limit = agent_session_limit(limit);
    match sessions_for_kind_blocking(kind, workdir, limit) {
        Ok((sessions, total)) => AgentSessionsResult {
            sessions,
            total,
            error: None,
        },
        Err(error) => AgentSessionsResult {
            sessions: Vec::new(),
            total: 0,
            error: Some(error),
        },
    }
}

pub async fn list_installed_agents(
    cache: &Arc<AgentCache>,
    refresh: bool,
) -> AgentInstalledListResult {
    cache.installed(refresh).await
}

pub async fn list_agents(
    cache: &Arc<AgentCache>,
    workdir: &Path,
    session: Option<&Arc<ServerSession>>,
    refresh: bool,
) -> AgentListResult {
    cache.agents(workdir, session, refresh).await
}

pub async fn list_agent_sessions(
    cache: &Arc<AgentCache>,
    kind: ManagedAgentKind,
    workdir: &Path,
    session: Option<&Arc<ServerSession>>,
    limit: u32,
    refresh: bool,
) -> AgentSessionsResult {
    cache.sessions(kind, workdir, session, limit, refresh).await
}

pub async fn merge_live_into_agent_list(
    agents: &mut [AgentSummary],
    session: Option<&Arc<ServerSession>>,
) {
    for agent in agents {
        agent.live = live_summary(agent.kind, session).await;
        agent.sessions.active_live = agent.live.active_terminal_ids.len();
    }
}

pub async fn merge_live_into_sessions(
    sessions: &mut [AgentSessionSummary],
    kind: ManagedAgentKind,
    session: Option<&Arc<ServerSession>>,
) {
    let live = live_sessions(kind, session).await;
    for summary in sessions {
        if let Some(live_state) = live.by_session.get(&summary.session_id) {
            summary.live = live_state.clone();
            summary.flags.live_bound = true;
            summary.flags.historical_only = false;
            if !summary
                .data_sources
                .contains(&AgentDataSource::TerminalMetadata)
            {
                summary.data_sources.push(AgentDataSource::TerminalMetadata);
            }
        }
    }
}

pub fn resume_launch_command(kind: ManagedAgentKind, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    let quoted = shell_quote(session_id);
    Some(match kind {
        ManagedAgentKind::Claude => format!("claude --resume {quoted}"),
        ManagedAgentKind::Codex => format!("codex resume {quoted}"),
        ManagedAgentKind::OpenCode => format!("opencode --session {quoted}"),
    })
}

pub fn normalize_event(
    kind: ManagedAgentKind,
    event_name: &str,
) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    let event_name = event_name.trim();
    if event_name.is_empty() {
        return None;
    }
    match kind {
        ManagedAgentKind::Claude => normalize_claude_event(event_name),
        ManagedAgentKind::Codex => normalize_codex_event(event_name),
        ManagedAgentKind::OpenCode => normalize_opencode_event(event_name),
    }
}

pub fn managed_kind_from_slug(raw: &str) -> Option<ManagedAgentKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "claude" => Some(ManagedAgentKind::Claude),
        "codex" => Some(ManagedAgentKind::Codex),
        "opencode" | "open-code" | "open_code" => Some(ManagedAgentKind::OpenCode),
        _ => None,
    }
}

fn normalize_claude_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "Setup" | "InstructionsLoaded" => (
            AgentEventKind::SessionUpdated,
            AgentLifecycleStatus::Starting,
        ),
        "SessionStart" => (
            AgentEventKind::SessionStarted,
            AgentLifecycleStatus::Starting,
        ),
        "SessionEnd" => (AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle),
        "UserPromptSubmit" | "UserPromptExpansion" => {
            (AgentEventKind::TurnStarted, AgentLifecycleStatus::Running)
        }
        "PreToolUse" => (AgentEventKind::ToolStarted, AgentLifecycleStatus::Running),
        "PermissionRequest" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForPermission,
        ),
        "PermissionDenied" => (
            AgentEventKind::PermissionResolved,
            AgentLifecycleStatus::Failed,
        ),
        "TaskCreated" | "SubagentStart" => {
            (AgentEventKind::TaskCreated, AgentLifecycleStatus::Running)
        }
        "TaskCompleted" | "SubagentStop" => (
            AgentEventKind::TaskCompleted,
            AgentLifecycleStatus::Completed,
        ),
        "PostToolUse" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        "PostToolUseFailure" => (AgentEventKind::ToolFailed, AgentLifecycleStatus::Failed),
        "PostToolBatch" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        "Stop" => (
            AgentEventKind::TurnCompleted,
            AgentLifecycleStatus::Completed,
        ),
        "StopFailure" => (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed),
        "TeammateIdle" => (AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle),
        "PreCompact" => (
            AgentEventKind::SessionUpdated,
            AgentLifecycleStatus::Running,
        ),
        "PostCompact" => (AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle),
        "ConfigChange" | "CwdChanged" | "WorktreeCreate" | "WorktreeRemove" => (
            AgentEventKind::SessionUpdated,
            AgentLifecycleStatus::Running,
        ),
        "Elicitation" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForUser,
        ),
        "ElicitationResult" => (
            AgentEventKind::PermissionResolved,
            AgentLifecycleStatus::Running,
        ),
        "Notification" => (AgentEventKind::Notification, AgentLifecycleStatus::Idle),
        _ => return None,
    })
}

fn normalize_codex_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "SessionStart" => (
            AgentEventKind::SessionStarted,
            AgentLifecycleStatus::Starting,
        ),
        "PermissionRequest" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForPermission,
        ),
        "PostToolUse" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        "Stop" => (
            AgentEventKind::TurnCompleted,
            AgentLifecycleStatus::Completed,
        ),
        name if name.contains("Failure") || name.contains("Failed") => {
            (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed)
        }
        _ => return None,
    })
}

fn normalize_opencode_event(event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    Some(match event_name {
        "session.status" => (
            AgentEventKind::SessionUpdated,
            AgentLifecycleStatus::Running,
        ),
        "session.idle" => (AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle),
        "session.error" => (AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed),
        "permission.asked" => (
            AgentEventKind::PermissionRequested,
            AgentLifecycleStatus::WaitingForPermission,
        ),
        "permission.replied" => (
            AgentEventKind::PermissionResolved,
            AgentLifecycleStatus::Running,
        ),
        "tool.execute.before" => (AgentEventKind::ToolStarted, AgentLifecycleStatus::Running),
        "tool.execute.after" => (AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running),
        name if name.starts_with("tool.") && name.ends_with(".error") => {
            (AgentEventKind::ToolFailed, AgentLifecycleStatus::Failed)
        }
        _ => return None,
    })
}

fn agent_summary_scan(kind: ManagedAgentKind, workdir: &Path) -> AgentSummary {
    let now = Utc::now();
    let cli = agent_list_cli_summary(kind, workdir);
    let setup = setup_summary(kind, cli.available, workdir);
    let mut warnings = Vec::new();
    let counts = match session_counts_for_kind(kind, workdir, &cli) {
        Ok(counts) => counts,
        Err(error) => {
            warnings.push(AgentWarning {
                code: "session_scan_failed".to_string(),
                message: error,
            });
            SessionCounts::default()
        }
    };

    let mut data_sources = vec![AgentDataSource::Cli, AgentDataSource::Setup];
    if counts.total > 0 {
        data_sources.push(match kind {
            ManagedAgentKind::OpenCode => AgentDataSource::ProviderCli,
            _ => AgentDataSource::HistoricalScan,
        });
    }

    AgentSummary {
        kind,
        display_name: display_name(kind).to_string(),
        cli,
        setup,
        capabilities: capabilities(kind),
        workspace: AgentWorkspaceSummary {
            workdir: workdir.to_string_lossy().into_owned(),
            provider_project_id: counts.provider_project_id.clone(),
            provider_project_key: None,
        },
        sessions: AgentSessionCounts {
            total: counts.total,
            resumable: counts.resumable,
            active_live: 0,
            latest_session_id: counts.latest_session_id.clone(),
            latest_session_title: counts.latest_session_title.clone(),
        },
        live: AgentLiveSummary {
            active_terminal_ids: Vec::new(),
            pending_action_count: 0,
            latest_event: None,
        },
        last_activity_at: counts.last_activity_at,
        updated_at: now,
        data_sources,
        warnings,
        account: account_summary(kind),
        usage: None,
    }
}

#[derive(Default)]
struct SessionCounts {
    total: usize,
    resumable: usize,
    latest_session_id: Option<String>,
    latest_session_title: Option<String>,
    last_activity_at: Option<DateTime<Utc>>,
    provider_project_id: Option<String>,
}

fn session_counts_for_kind(
    kind: ManagedAgentKind,
    workdir: &Path,
    cli: &AgentCliSummary,
) -> Result<SessionCounts, String> {
    match kind {
        ManagedAgentKind::Claude => claude_session_counts(workdir),
        ManagedAgentKind::Codex => codex_session_counts(workdir),
        ManagedAgentKind::OpenCode => opencode_session_counts(workdir, cli),
    }
}

fn claude_session_counts(workdir: &Path) -> Result<SessionCounts, String> {
    let (total, latest) =
        claude::session_count_summary(workdir).map_err(|error| error.to_string())?;
    Ok(SessionCounts {
        total,
        resumable: total,
        latest_session_id: latest.as_ref().map(|session| session.session_id.clone()),
        latest_session_title: latest.as_ref().and_then(|session| session.title.clone()),
        last_activity_at: latest
            .and_then(|session| parse_rfc3339(session.last_activity_at.as_deref())),
        provider_project_id: None,
    })
}

fn codex_session_counts(workdir: &Path) -> Result<SessionCounts, String> {
    let threads = codex_threads_for_workdir(workdir)?;
    let latest = threads.first();
    Ok(SessionCounts {
        total: threads.len(),
        resumable: threads.len(),
        latest_session_id: latest.map(|thread| thread.id.clone()),
        latest_session_title: latest.and_then(codex_title_from_thread),
        last_activity_at: latest.and_then(codex_thread_updated_at),
        provider_project_id: None,
    })
}

fn opencode_session_counts(
    workdir: &Path,
    _cli: &AgentCliSummary,
) -> Result<SessionCounts, String> {
    if !opencode_sessions_available() {
        return Ok(SessionCounts::default());
    }
    let summary = opencode_session_count_summary(workdir)?;
    Ok(SessionCounts {
        total: summary.total,
        resumable: summary.total,
        latest_session_id: summary.latest.as_ref().map(|session| session.id.clone()),
        latest_session_title: summary
            .latest
            .as_ref()
            .and_then(|session| session_title(session.title.clone())),
        last_activity_at: summary
            .latest
            .as_ref()
            .and_then(|session| session.updated)
            .and_then(DateTime::<Utc>::from_timestamp_millis),
        provider_project_id: summary
            .latest
            .and_then(|session| session.project_id.clone()),
    })
}

fn opencode_session_count_summary(workdir: &Path) -> Result<OpenCodeSessionCountSummary, String> {
    let (json, _) = fetch_opencode_sessions_json()?;
    let raw: Vec<OpenCodeSessionJson> =
        serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    let mut git_cache = HashMap::new();
    let mut matched = raw
        .into_iter()
        .filter(|session| opencode_workdir_matches(workdir, session, &mut git_cache))
        .collect::<Vec<_>>();
    matched.sort_by(|left, right| {
        right
            .updated
            .unwrap_or(0)
            .cmp(&left.updated.unwrap_or(0))
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(OpenCodeSessionCountSummary {
        total: matched.len(),
        latest: matched.into_iter().next(),
    })
}

fn sessions_for_kind_blocking(
    kind: ManagedAgentKind,
    workdir: &Path,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, u32), String> {
    let cli = match kind {
        ManagedAgentKind::OpenCode => opencode_session_cli_summary(),
        _ => agent_list_cli_summary(kind, workdir),
    };
    let mut sessions = match kind {
        ManagedAgentKind::Claude => claude_sessions(workdir, &cli, limit),
        ManagedAgentKind::Codex => codex_sessions(workdir, &cli, limit),
        ManagedAgentKind::OpenCode => opencode_sessions_limited(workdir, &cli, limit),
    }?;
    let total = u32::try_from(sessions.1).unwrap_or(u32::MAX);
    sessions
        .0
        .sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
    Ok((sessions.0, total))
}

fn cli_version_summary(kind: ManagedAgentKind) -> AgentCliSummary {
    let program = program_name(kind);
    match Command::new(program).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).into_owned()
            } else {
                String::from_utf8_lossy(&output.stdout).into_owned()
            };
            AgentCliSummary {
                available: true,
                version: first_non_empty_line(&text),
                error: None,
            }
        }
        Ok(output) => AgentCliSummary {
            available: false,
            version: None,
            error: Some(format!(
                "`{program} --version` exited with {}",
                output.status
            )),
        },
        Err(error) => AgentCliSummary {
            available: false,
            version: None,
            error: Some(error.to_string()),
        },
    }
}

fn agent_list_cli_summary(kind: ManagedAgentKind, workdir: &Path) -> AgentCliSummary {
    let available = match kind {
        ManagedAgentKind::Claude => {
            command_on_path("claude")
                || claude::project_dir_for_workdir(&home_path(&[".claude"]), workdir).is_dir()
        }
        ManagedAgentKind::Codex => codex_sessions_available(),
        ManagedAgentKind::OpenCode => opencode_sessions_available(),
    };
    if available {
        AgentCliSummary {
            available: true,
            version: None,
            error: None,
        }
    } else {
        AgentCliSummary {
            available: false,
            version: None,
            error: Some(format!(
                "{} CLI and local session data not found",
                program_name(kind)
            )),
        }
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn setup_summary(kind: ManagedAgentKind, cli_available: bool, workdir: &Path) -> AgentSetupSummary {
    if !cli_available {
        return AgentSetupSummary {
            state: AgentSetupState::MissingCli,
            skills_installed: false,
            plugin_installed: false,
            hooks_installed: false,
            error: None,
        };
    }

    let mut error = None;
    let (skills_installed, plugin_installed, hooks_installed) = match kind {
        ManagedAgentKind::Claude => {
            let status = claude_plugin_status();
            error = status.error;
            (
                false,
                status.plugin_installed,
                status.hooks_installed || claude_local_hooks_installed(workdir),
            )
        }
        ManagedAgentKind::Codex => (
            skills_installed_at(&home_path(&[".agents", "skills"])),
            false,
            codex_hooks_installed() || codex_local_hooks_installed(workdir),
        ),
        ManagedAgentKind::OpenCode => (
            skills_installed_at(&home_path(&[".config", "opencode", "skills"])),
            opencode_plugin_installed(),
            opencode_hooks_installed() || opencode_local_hooks_installed(workdir),
        ),
    };
    let state = if error.is_some() {
        AgentSetupState::Error
    } else if hooks_installed {
        AgentSetupState::HooksReady
    } else if skills_installed || plugin_installed {
        AgentSetupState::SkillsOnly
    } else {
        AgentSetupState::NotConfigured
    };

    AgentSetupSummary {
        state,
        skills_installed,
        plugin_installed,
        hooks_installed,
        error,
    }
}

#[derive(Default)]
struct ClaudePluginStatus {
    plugin_installed: bool,
    hooks_installed: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeInstalledPluginsFile {
    plugins: HashMap<String, Vec<ClaudeInstalledPluginEntry>>,
}

#[derive(Deserialize)]
struct ClaudeInstalledPluginEntry {
    #[serde(rename = "installPath")]
    install_path: String,
}

const CLAUDE_ZEDRA_PLUGIN_ID: &str = "zedra@zedra";

fn claude_plugin_status() -> ClaudePluginStatus {
    let path = home_path(&[".claude", "plugins", "installed_plugins.json"]);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ClaudePluginStatus::default();
        }
        Err(error) => {
            return ClaudePluginStatus {
                error: Some(error.to_string()),
                ..ClaudePluginStatus::default()
            };
        }
    };
    claude_plugin_status_from_installed_plugins(&contents)
}

fn claude_plugin_status_from_installed_plugins(contents: &str) -> ClaudePluginStatus {
    let installed: ClaudeInstalledPluginsFile = match serde_json::from_str(contents) {
        Ok(installed) => installed,
        Err(error) => {
            return ClaudePluginStatus {
                error: Some(error.to_string()),
                ..ClaudePluginStatus::default()
            };
        }
    };
    let Some(entry) = installed
        .plugins
        .get(CLAUDE_ZEDRA_PLUGIN_ID)
        .and_then(|entries| entries.first())
    else {
        return ClaudePluginStatus::default();
    };
    let install_path = Path::new(&entry.install_path);
    let hooks_installed = install_path.join("hooks").join("hooks.json").is_file();
    ClaudePluginStatus {
        plugin_installed: true,
        hooks_installed,
        error: None,
    }
}

fn skills_installed_at(base: &Path) -> bool {
    SKILL_NAMES
        .iter()
        .all(|skill| base.join(skill).join("SKILL.md").is_file())
}

fn codex_hooks_installed() -> bool {
    std::fs::read_to_string(home_path(&[".codex", "config.toml"]))
        .map(|contents| contents.contains("zedra") && contents.contains("hook"))
        .unwrap_or(false)
}

fn claude_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".claude/settings.local.json"))
}

fn codex_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".codex/hooks.json"))
}

fn opencode_plugin_installed() -> bool {
    let plugin_dir = home_path(&[".config", "opencode", "plugins"]);
    ["zedra.js", "zedra.ts", "zedra.mjs"]
        .iter()
        .any(|name| plugin_dir.join(name).is_file())
}

fn opencode_hooks_installed() -> bool {
    opencode_plugin_installed()
}

fn opencode_local_hooks_installed(workdir: &Path) -> bool {
    hook_file_mentions_zedra(&workdir.join(".opencode/plugins/zedra.js"))
}

fn hook_file_mentions_zedra(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|contents| contents.contains("zedra-agent-hook") || contents.contains("agent hook"))
        .unwrap_or(false)
}

fn capabilities(kind: ManagedAgentKind) -> AgentCapabilities {
    AgentCapabilities {
        list_sessions: true,
        resume_session: true,
        live_binding: true,
        confirm_action: true,
        select_action: true,
        lifecycle_events: true,
        usage_snapshot: matches!(kind, ManagedAgentKind::Claude),
    }
}

fn claude_sessions(
    workdir: &Path,
    cli: &AgentCliSummary,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, usize), String> {
    let list = claude::list_sessions_limited(workdir, limit).map_err(|error| error.to_string())?;
    let sessions = list
        .sessions
        .iter()
        .map(|session| claude_session_summary(session, cli))
        .collect();
    Ok((sessions, list.total))
}

fn claude_session_summary(
    session: &claude::ClaudeSessionMetadata,
    cli: &AgentCliSummary,
) -> AgentSessionSummary {
    let pr = session.pr_links.first();
    AgentSessionSummary {
        kind: ManagedAgentKind::Claude,
        session_id: session.session_id.clone(),
        title: session_title(session.title.clone()),
        cwd: session.cwd.clone(),
        created_at: parse_rfc3339(session.created_at.as_deref()),
        last_activity_at: parse_rfc3339(session.last_activity_at.as_deref()),
        resume: resume_summary(ManagedAgentKind::Claude, &session.session_id),
        live: empty_session_live(),
        provider: AgentProviderSessionInfo {
            model: None,
            permission_mode: session.permission_mode.clone(),
            cli_version: session
                .claude_version
                .clone()
                .or_else(|| cli.version.clone()),
            origin: session.user_type.clone(),
            source: None,
            entrypoint: session.entrypoint.clone(),
            native_project_id: None,
            model_provider: Some("anthropic".to_string()),
        },
        git: Some(AgentGitSummary {
            branch: session.git_branch.clone(),
            worktree: session.worktree.clone(),
            commit_hash: None,
            repository_url: None,
            pr_number: pr.and_then(|pr| pr.number),
            pr_url: pr.and_then(|pr| pr.url.clone()),
            pr_repository: pr.and_then(|pr| pr.repository.clone()),
        }),
        usage: None,
        counters: AgentSessionCounters {
            record_count: session.message_count as u64,
            message_count: session.message_count as u64,
            turn_count: 0,
            tool_count: 0,
            tool_failure_count: session.task_failed_count as u64,
            hook_success_count: session.hook_success_count as u64,
            hook_failure_count: session.hook_failure_count as u64,
            malformed_record_count: session.malformed_line_count as u64,
        },
        flags: AgentSessionFlags {
            is_sidechain: session.is_sidechain,
            is_subagent: false,
            is_archived: false,
            historical_only: true,
            live_bound: false,
        },
        data_sources: vec![AgentDataSource::HistoricalScan],
        warnings: malformed_warning(session.malformed_line_count),
        transcript_size_bytes: file_size_bytes(&session.transcript_path),
    }
}

fn codex_sessions(
    workdir: &Path,
    cli: &AgentCliSummary,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, usize), String> {
    let threads = codex_threads_for_workdir(workdir)?;
    let total = threads.len();
    let summaries = threads
        .into_iter()
        .take(limit)
        .map(|thread| codex_session_summary_from_thread(&thread, cli))
        .collect();
    Ok((summaries, total))
}

#[derive(Debug, Deserialize)]
struct CodexThreadRow {
    id: String,
    cwd: String,
    title: String,
    rollout_path: String,
    source: String,
    model_provider: String,
    #[serde(default)]
    cli_version: String,
    created_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
    #[serde(default)]
    first_user_message: String,
    #[serde(default)]
    preview: String,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    git_branch: Option<String>,
    git_sha: Option<String>,
    git_origin_url: Option<String>,
    #[serde(default)]
    approval_mode: String,
    model: Option<String>,
}

fn codex_sessions_available() -> bool {
    command_on_path("codex") || codex_state_db_path().is_some()
}

fn codex_state_db_path() -> Option<PathBuf> {
    let dir = home_path(&[".codex"]);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return None;
    };
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("state_") || !name.ends_with(".sqlite") {
            continue;
        }
        let Some(version) = name
            .strip_prefix("state_")
            .and_then(|suffix| suffix.strip_suffix(".sqlite"))
            .and_then(|version| version.parse::<u64>().ok())
        else {
            continue;
        };
        match best {
            Some((current, _)) if current >= version => {}
            _ => best = Some((version, entry.path())),
        }
    }
    best.map(|(_, path)| path)
}

fn fetch_codex_threads_from_db(workdir: &Path) -> Result<Vec<CodexThreadRow>, String> {
    let db_path =
        codex_state_db_path().ok_or_else(|| "Codex state database not found".to_string())?;
    let cwd_keys = codex_workdir_keys(workdir);
    let cwd_filter = cwd_keys
        .iter()
        .map(|cwd| sql_string_literal(cwd))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        r#"
        SELECT
            id,
            cwd,
            title,
            rollout_path,
            source,
            model_provider,
            cli_version,
            created_at_ms,
            updated_at_ms,
            first_user_message,
            preview,
            agent_nickname,
            agent_role,
            git_branch,
            git_sha,
            git_origin_url,
            approval_mode,
            model
        FROM threads
        WHERE archived = 0 AND cwd IN ({cwd_filter})
        ORDER BY updated_at_ms DESC
    "#
    );
    let stdout = sqlite_readonly::query_json(&db_path, &query)?;
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&stdout).map_err(|error| error.to_string())
}

fn codex_threads_for_workdir(workdir: &Path) -> Result<Vec<CodexThreadRow>, String> {
    fetch_codex_threads_from_db(workdir)
}

fn codex_workdir_keys(workdir: &Path) -> Vec<String> {
    let canonical = normalize_path(workdir).to_string_lossy().into_owned();
    let raw = workdir.to_string_lossy().trim_end_matches('/').to_string();
    if raw == canonical {
        vec![canonical]
    } else {
        vec![canonical, raw]
    }
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn codex_thread_updated_at(thread: &CodexThreadRow) -> Option<DateTime<Utc>> {
    thread
        .updated_at_ms
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .or_else(|| {
            thread
                .created_at_ms
                .and_then(DateTime::<Utc>::from_timestamp_millis)
        })
}

fn codex_title_from_thread(thread: &CodexThreadRow) -> Option<String> {
    sanitize_codex_title_field(&thread.title)
        .or_else(|| sanitize_codex_title_field(&thread.preview))
        .or_else(|| sanitize_codex_prompt_fallback(&thread.first_user_message))
        .or_else(|| {
            codex_title_from_agent_identity(
                thread.agent_nickname.as_deref(),
                thread.agent_role.as_deref(),
            )
        })
}

fn sanitize_codex_title_field(raw: &str) -> Option<String> {
    let mut line = raw.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("continue ") {
        let rest = rest.trim();
        if rest.starts_with('/') || rest.starts_with('~') {
            line = codex_title_from_path(rest).unwrap_or(rest);
        }
    } else if line.starts_with('/') || line.starts_with('~') {
        line = codex_title_from_path(line).unwrap_or(line);
    }
    finalize_codex_title(line)
}

fn sanitize_codex_prompt_fallback(raw: &str) -> Option<String> {
    let mut line = raw.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("CWD:") {
        line = rest.trim();
        if let Some((_, after_path)) = line.split_once(". ") {
            line = after_path.trim();
        }
    }
    sanitize_codex_title_field(line)
}

fn finalize_codex_title(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(utils::truncate_chars(&collapsed, 80))
}

fn codex_title_from_path(path: &str) -> Option<&str> {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
}

fn codex_session_summary_from_thread(
    thread: &CodexThreadRow,
    cli: &AgentCliSummary,
) -> AgentSessionSummary {
    let rollout_path = PathBuf::from(&thread.rollout_path);
    let cli_version = (!thread.cli_version.is_empty()).then_some(thread.cli_version.clone());
    AgentSessionSummary {
        kind: ManagedAgentKind::Codex,
        session_id: thread.id.clone(),
        title: session_title(codex_title_from_thread(thread)),
        cwd: Some(thread.cwd.clone()),
        created_at: thread
            .created_at_ms
            .and_then(DateTime::<Utc>::from_timestamp_millis),
        last_activity_at: codex_thread_updated_at(thread),
        resume: resume_summary(ManagedAgentKind::Codex, &thread.id),
        live: empty_session_live(),
        provider: AgentProviderSessionInfo {
            model: thread.model.clone(),
            permission_mode: (!thread.approval_mode.is_empty())
                .then_some(thread.approval_mode.clone()),
            cli_version: cli_version.or_else(|| cli.version.clone()),
            origin: None,
            source: Some(thread.source.clone()),
            entrypoint: None,
            native_project_id: None,
            model_provider: Some(thread.model_provider.clone()),
        },
        git: Some(AgentGitSummary {
            branch: thread.git_branch.clone(),
            worktree: None,
            commit_hash: thread.git_sha.clone(),
            repository_url: thread.git_origin_url.clone(),
            pr_number: None,
            pr_url: None,
            pr_repository: None,
        }),
        usage: None,
        counters: AgentSessionCounters {
            record_count: 0,
            message_count: 0,
            turn_count: 0,
            tool_count: 0,
            tool_failure_count: 0,
            hook_success_count: 0,
            hook_failure_count: 0,
            malformed_record_count: 0,
        },
        flags: AgentSessionFlags {
            is_sidechain: false,
            is_subagent: false,
            is_archived: false,
            historical_only: true,
            live_bound: false,
        },
        data_sources: vec![AgentDataSource::HistoricalScan],
        warnings: Vec::new(),
        transcript_size_bytes: file_size_bytes(&rollout_path),
    }
}

fn codex_title_from_agent_identity(nickname: Option<&str>, role: Option<&str>) -> Option<String> {
    let nickname = nickname?.trim();
    if nickname.is_empty() {
        return None;
    }
    let title = match role.map(str::trim).filter(|role| !role.is_empty()) {
        Some(role) => format!("{nickname} ({role})"),
        None => nickname.to_string(),
    };
    Some(utils::truncate_chars(&title, 80))
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OpenCodeSessionJson {
    id: String,
    title: Option<String>,
    updated: Option<i64>,
    created: Option<i64>,
    project_id: Option<String>,
    /// Per-session working directory (OpenCode `session.directory`).
    directory: Option<String>,
    /// Project root worktree (`project.worktree`), used for workdir matching only.
    #[serde(default, alias = "worktree")]
    project_worktree: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    workspace_branch: Option<String>,
    #[serde(default)]
    workspace_directory: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    transcript_size_bytes: Option<i64>,
}

struct OpenCodeSessionCountSummary {
    total: usize,
    latest: Option<OpenCodeSessionJson>,
}

fn opencode_db_path() -> PathBuf {
    home_path(&[".local", "share", "opencode", "opencode.db"])
}

fn command_on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

fn opencode_sessions_available() -> bool {
    opencode_db_path().is_file() || command_on_path("opencode")
}

fn opencode_session_cli_summary() -> AgentCliSummary {
    if opencode_sessions_available() {
        AgentCliSummary {
            available: true,
            version: None,
            error: None,
        }
    } else {
        AgentCliSummary {
            available: false,
            version: None,
            error: Some("OpenCode CLI and database not found".to_string()),
        }
    }
}

fn fetch_opencode_sessions_json() -> Result<(Vec<u8>, &'static str), String> {
    if let Ok(json) = fetch_opencode_sessions_json_from_db() {
        return Ok((json, "opencode sqlite"));
    }

    let output = Command::new("opencode")
        .args(["session", "list", "--format", "json", "--pure"])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() && !output.stdout.is_empty() {
        return Ok((output.stdout, "opencode session list"));
    }

    let cli_error = if output.stderr.is_empty() {
        format!("`opencode session list` exited with {}", output.status)
    } else {
        format!(
            "`opencode session list` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
    };

    fetch_opencode_sessions_json_from_db()
        .map(|json| (json, "opencode sqlite"))
        .map_err(|fallback_error| format!("{cli_error}; sqlite fallback failed: {fallback_error}"))
}

fn fetch_opencode_sessions_json_from_db() -> Result<Vec<u8>, String> {
    let db_path = opencode_db_path();
    if !db_path.is_file() {
        return Err(format!(
            "OpenCode database not found at {}",
            db_path.display()
        ));
    }

    const QUERY: &str = r#"
        SELECT
            s.id AS id,
            s.title AS title,
            s.directory AS directory,
            s.path AS path,
            s.workspace_id AS workspaceId,
            w.branch AS workspaceBranch,
            w.directory AS workspaceDirectory,
            s.time_created AS created,
            s.time_updated AS updated,
            s.project_id AS projectId,
            p.worktree AS projectWorktree,
            (
                SELECT CAST(COALESCE(SUM(LENGTH(m.data)), 0) AS INTEGER)
                FROM message m
                WHERE m.session_id = s.id
            ) AS transcriptSizeBytes
        FROM session s
        JOIN project p ON p.id = s.project_id
        LEFT JOIN workspace w ON w.id = s.workspace_id
        WHERE s.time_archived IS NULL
        ORDER BY s.time_updated DESC
    "#;

    sqlite_readonly::query_json(&db_path, QUERY)
}

fn opencode_workdir_matches(
    workdir: &Path,
    session: &OpenCodeSessionJson,
    git_cache: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> bool {
    let candidates = [
        session.directory.as_deref(),
        session.workspace_directory.as_deref(),
        session.project_worktree.as_deref(),
    ];
    for candidate in candidates.into_iter().flatten() {
        if cwd_matches(workdir, Some(candidate)) {
            return true;
        }
        if share_git_repository(workdir, Path::new(candidate), git_cache) {
            return true;
        }
    }
    false
}

fn share_git_repository(
    left: &Path,
    right: &Path,
    cache: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> bool {
    let Some(left_common) = git_common_dir(left, cache) else {
        return false;
    };
    let Some(right_common) = git_common_dir(right, cache) else {
        return false;
    };
    left_common == right_common
}

fn git_common_dir(path: &Path, cache: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
    let key = normalize_path(path);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let resolved = resolve_git_common_dir(path);
    cache.insert(key, resolved.clone());
    resolved
}

fn resolve_git_common_dir(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let relative = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if relative.is_empty() {
        return None;
    }
    let git_dir = PathBuf::from(&relative);
    let absolute = if git_dir.is_absolute() {
        git_dir
    } else {
        path.join(git_dir)
    };
    Some(normalize_path(&absolute))
}

fn opencode_sessions_limited(
    workdir: &Path,
    cli: &AgentCliSummary,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, usize), String> {
    if !cli.available {
        return Ok((Vec::new(), 0));
    }
    let (json, source) = fetch_opencode_sessions_json()?;
    let mut sessions = opencode_sessions_from_json(workdir, &json, cli, source)?;
    let total = sessions.len();
    sessions.truncate(limit);
    Ok((sessions, total))
}

fn opencode_transcript_sizes_from_db() -> HashMap<String, u64> {
    let db_path = opencode_db_path();
    if !db_path.is_file() {
        return HashMap::new();
    }

    const QUERY: &str = r#"
        SELECT session_id, CAST(COALESCE(SUM(LENGTH(data)), 0) AS INTEGER) AS bytes
        FROM message
        GROUP BY session_id
    "#;

    let stdout = match sqlite_readonly::query_json(&db_path, QUERY) {
        Ok(stdout) => stdout,
        Err(_) => return HashMap::new(),
    };
    if stdout.is_empty() {
        return HashMap::new();
    }

    #[derive(Deserialize)]
    struct Row {
        session_id: String,
        bytes: i64,
    }

    serde_json::from_slice::<Vec<Row>>(&stdout)
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    let bytes = row.bytes.max(0) as u64;
                    (bytes > 0).then_some((row.session_id, bytes))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn git_branch_at(path: &Path, cache: &mut HashMap<PathBuf, Option<String>>) -> Option<String> {
    let key = normalize_path(path);
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let branch = resolve_git_branch(path);
    cache.insert(key, branch.clone());
    branch
}

fn resolve_git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

fn opencode_git_summary(
    session: &OpenCodeSessionJson,
    branch_cache: &mut HashMap<PathBuf, Option<String>>,
) -> Option<AgentGitSummary> {
    let directory = session.directory.as_deref()?;
    let branch = session
        .workspace_branch
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| git_branch_at(Path::new(directory), branch_cache));
    Some(AgentGitSummary {
        branch,
        worktree: Some(directory.to_string()),
        commit_hash: None,
        repository_url: None,
        pr_number: None,
        pr_url: None,
        pr_repository: None,
    })
}

fn opencode_transcript_size_bytes(
    session: &OpenCodeSessionJson,
    transcript_sizes: &HashMap<String, u64>,
) -> Option<u64> {
    session
        .transcript_size_bytes
        .and_then(|size| (size > 0).then_some(size as u64))
        .or_else(|| {
            session
                .path
                .as_deref()
                .map(Path::new)
                .and_then(file_size_bytes)
        })
        .or_else(|| transcript_sizes.get(&session.id).copied())
        .filter(|size| *size > 0)
}

fn opencode_sessions_from_json(
    workdir: &Path,
    json: &[u8],
    cli: &AgentCliSummary,
    source: &str,
) -> Result<Vec<AgentSessionSummary>, String> {
    let raw: Vec<OpenCodeSessionJson> =
        serde_json::from_slice(json).map_err(|error| error.to_string())?;
    let transcript_sizes = opencode_transcript_sizes_from_db();
    let mut git_common_cache = HashMap::new();
    let mut git_branch_cache = HashMap::new();
    let mut sessions = Vec::new();
    for session in raw {
        if !opencode_workdir_matches(workdir, &session, &mut git_common_cache) {
            continue;
        }
        let git = opencode_git_summary(&session, &mut git_branch_cache);
        let transcript_size_bytes = opencode_transcript_size_bytes(&session, &transcript_sizes);
        sessions.push(AgentSessionSummary {
            kind: ManagedAgentKind::OpenCode,
            session_id: session.id.clone(),
            title: session_title(session.title.clone()),
            cwd: session
                .directory
                .clone()
                .or(session.workspace_directory.clone())
                .or(session.project_worktree.clone()),
            created_at: session
                .created
                .and_then(DateTime::<Utc>::from_timestamp_millis),
            last_activity_at: session
                .updated
                .and_then(DateTime::<Utc>::from_timestamp_millis),
            resume: resume_summary(ManagedAgentKind::OpenCode, &session.id),
            live: empty_session_live(),
            provider: AgentProviderSessionInfo {
                model: None,
                permission_mode: None,
                cli_version: cli.version.clone(),
                origin: None,
                source: Some(source.to_string()),
                entrypoint: None,
                native_project_id: session.project_id,
                model_provider: None,
            },
            git,
            usage: None,
            counters: AgentSessionCounters {
                record_count: 0,
                message_count: 0,
                turn_count: 0,
                tool_count: 0,
                tool_failure_count: 0,
                hook_success_count: 0,
                hook_failure_count: 0,
                malformed_record_count: 0,
            },
            flags: AgentSessionFlags {
                is_sidechain: false,
                is_subagent: false,
                is_archived: false,
                historical_only: true,
                live_bound: false,
            },
            data_sources: vec![AgentDataSource::ProviderCli],
            warnings: Vec::new(),
            transcript_size_bytes,
        });
    }
    sessions.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
    Ok(sessions)
}

#[derive(Default)]
struct LiveAgentSessions {
    active_terminal_ids: Vec<String>,
    by_session: HashMap<String, AgentSessionLiveSummary>,
}

async fn live_summary(
    kind: ManagedAgentKind,
    session: Option<&Arc<ServerSession>>,
) -> AgentLiveSummary {
    let live = live_sessions(kind, session).await;
    AgentLiveSummary {
        active_terminal_ids: live.active_terminal_ids,
        pending_action_count: 0,
        latest_event: None,
    }
}

async fn live_sessions(
    kind: ManagedAgentKind,
    session: Option<&Arc<ServerSession>>,
) -> LiveAgentSessions {
    let Some(session) = session else {
        return LiveAgentSessions::default();
    };
    let terms = session.terminals.lock().await;
    let mut live = LiveAgentSessions::default();
    for (terminal_id, term) in terms.iter() {
        let Some(meta) = term.host_meta.lock().ok().map(snapshot_meta) else {
            continue;
        };
        if !terminal_matches(kind, &meta) {
            continue;
        }
        live.active_terminal_ids.push(terminal_id.clone());
        if let Some(session_id) = infer_session_id(kind, &meta) {
            live.by_session.insert(
                session_id.clone(),
                AgentSessionLiveSummary {
                    terminal_id: Some(terminal_id.clone()),
                    status: lifecycle_from_shell(meta.shell_state),
                    pending_action_count: 0,
                    current_turn_id: None,
                    latest_event: Some(AgentEventSummary {
                        kind: AgentEventKind::SessionUpdated,
                        status: lifecycle_from_shell(meta.shell_state),
                        at: None,
                        terminal_id: Some(terminal_id.clone()),
                        session_id: Some(session_id),
                        turn_id: None,
                        tool_name: None,
                    }),
                },
            );
        }
    }
    live.active_terminal_ids.sort();
    live
}

fn snapshot_meta(meta: std::sync::MutexGuard<'_, HostTermMeta>) -> HostTermMetaSnapshot {
    HostTermMetaSnapshot {
        icon_name: meta.icon_name.clone(),
        current_command: meta.current_command.clone(),
        shell_state: meta.shell_state,
    }
}

struct HostTermMetaSnapshot {
    icon_name: Option<String>,
    current_command: Option<String>,
    shell_state: HostShellState,
}

fn terminal_matches(kind: ManagedAgentKind, meta: &HostTermMetaSnapshot) -> bool {
    let needle = match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
    };
    meta.icon_name
        .as_deref()
        .is_some_and(|icon| icon.to_ascii_lowercase().contains(needle))
        || meta
            .current_command
            .as_deref()
            .is_some_and(|command| command_mentions(kind, command))
}

fn command_mentions(kind: ManagedAgentKind, command: &str) -> bool {
    let low = command.to_ascii_lowercase();
    match kind {
        ManagedAgentKind::Claude => low.contains("claude"),
        ManagedAgentKind::Codex => low.contains("codex"),
        ManagedAgentKind::OpenCode => low.contains("opencode") || low.contains("open-code"),
    }
}

fn infer_session_id(kind: ManagedAgentKind, meta: &HostTermMetaSnapshot) -> Option<String> {
    let command = meta.current_command.as_deref()?;
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    match kind {
        ManagedAgentKind::Claude => value_after_flag(&tokens, "--resume"),
        ManagedAgentKind::Codex => {
            let resume_index = tokens.iter().position(|token| *token == "resume")?;
            tokens
                .get(resume_index + 1)
                .filter(|value| !value.starts_with('-'))
                .map(|value| value.trim_matches('"').trim_matches('\'').to_string())
        }
        ManagedAgentKind::OpenCode => {
            value_after_flag(&tokens, "--session").or_else(|| value_after_flag(&tokens, "-s"))
        }
    }
}

fn value_after_flag(tokens: &[&str], flag: &str) -> Option<String> {
    let prefix = format!("{flag}=");
    for (index, token) in tokens.iter().enumerate() {
        if *token == flag {
            return tokens
                .get(index + 1)
                .map(|value| value.trim_matches('"').trim_matches('\'').to_string());
        }
        if let Some(value) = token.strip_prefix(&prefix) {
            return Some(value.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn lifecycle_from_shell(state: HostShellState) -> AgentLifecycleStatus {
    match state {
        HostShellState::Unknown => AgentLifecycleStatus::Unknown,
        HostShellState::Idle => AgentLifecycleStatus::Idle,
        HostShellState::Running => AgentLifecycleStatus::Running,
    }
}

fn resume_summary(kind: ManagedAgentKind, session_id: &str) -> AgentResumeSummary {
    let available = !session_id.trim().is_empty();
    AgentResumeSummary {
        available,
        unavailable_reason: (!available).then(|| "missing session id".to_string()),
        action_id: available.then(|| format!("{}:{session_id}", kind_slug(kind))),
    }
}

fn empty_session_live() -> AgentSessionLiveSummary {
    AgentSessionLiveSummary {
        terminal_id: None,
        status: AgentLifecycleStatus::Unknown,
        pending_action_count: 0,
        current_turn_id: None,
        latest_event: None,
    }
}

fn malformed_warning(count: usize) -> Vec<AgentWarning> {
    if count == 0 {
        Vec::new()
    } else {
        vec![AgentWarning {
            code: "malformed_records".to_string(),
            message: format!("{count} malformed records were ignored"),
        }]
    }
}

fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn cwd_matches(workdir: &Path, cwd: Option<&str>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    paths_equal(workdir, Path::new(cwd))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if normalize_path(left) == normalize_path(right) {
        return true;
    }
    left.to_string_lossy().trim_end_matches('/') == right.to_string_lossy().trim_end_matches('/')
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn home_path(parts: &[&str]) -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for part in parts {
        path.push(part);
    }
    path
}

fn display_name(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "Claude",
        ManagedAgentKind::Codex => "Codex",
        ManagedAgentKind::OpenCode => "OpenCode",
    }
}

fn session_title(stored: Option<String>) -> Option<String> {
    stored
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .or_else(|| Some("Unknown".to_string()))
}

fn file_size_bytes(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

fn account_summary(kind: ManagedAgentKind) -> AgentAccountSummary {
    let fields = match kind {
        ManagedAgentKind::Claude => claude_account_fields(),
        ManagedAgentKind::Codex => codex_account_fields(),
        ManagedAgentKind::OpenCode => opencode_account_fields(),
    };
    AgentAccountSummary { fields }
}

fn claude_account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    let settings_path = home_path(&[".claude", "settings.json"]);
    if let Ok(value) = read_json_file(&settings_path) {
        push_json_string(&mut fields, "Model", &value, &["model"]);
        push_json_string(&mut fields, "Effort", &value, &["effortLevel"]);
        push_json_string(
            &mut fields,
            "Permission mode",
            &value,
            &["permissions", "defaultMode"],
        );
    }
    let stats_path = home_path(&[".claude", "stats-cache.json"]);
    if let Ok(value) = read_json_file(&stats_path) {
        if let Some(total_cost) = claude_total_cost_usd(&value) {
            fields.push(AgentInfoField {
                label: "Total cost (USD)".to_string(),
                value: format!("{total_cost:.4}"),
            });
        }
        if let Some((today, week)) = claude_daily_usage(&value) {
            let today_str = Utc::now().format("%Y-%m-%d").to_string();
            if let Some(last) = value.get("lastComputedDate").and_then(Value::as_str) {
                if last == today_str {
                    fields.push(AgentInfoField {
                        label: "Today msgs".to_string(),
                        value: today.messages.to_string(),
                    });
                }
            }
            fields.push(AgentInfoField {
                label: "Week msgs".to_string(),
                value: week.messages.to_string(),
            });
            fields.push(AgentInfoField {
                label: "Week sessions".to_string(),
                value: week.sessions.to_string(),
            });
            fields.push(AgentInfoField {
                label: "Week tools".to_string(),
                value: week.tools.to_string(),
            });
        }
    }
    fields
}

#[derive(Default)]
struct DailyTotals {
    messages: u64,
    sessions: u64,
    tools: u64,
}

fn claude_daily_usage(value: &Value) -> Option<(DailyTotals, DailyTotals)> {
    let activity = value.get("dailyActivity")?.as_array()?;
    let today_str = Utc::now().format("%Y-%m-%d").to_string();
    let week_start = (Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    let mut today = DailyTotals::default();
    let mut week = DailyTotals::default();
    for entry in activity {
        let date = entry.get("date")?.as_str()?;
        let msgs = entry
            .get("messageCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let sessions = entry
            .get("sessionCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let tools = entry
            .get("toolCallCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if date == today_str {
            today.messages += msgs;
            today.sessions += sessions;
            today.tools += tools;
        }
        if date >= week_start.as_str() {
            week.messages += msgs;
            week.sessions += sessions;
            week.tools += tools;
        }
    }
    Some((today, week))
}

fn codex_account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    let auth_path = home_path(&[".codex", "auth.json"]);
    let logged_in = auth_path.is_file();
    fields.push(AgentInfoField {
        label: "Logged in".to_string(),
        value: if logged_in { "yes" } else { "no" }.to_string(),
    });
    if logged_in {
        if let Ok(contents) = std::fs::read_to_string(&auth_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&contents) {
                if let Some(profile) = codex_jwt_profile(&value) {
                    fields.push(AgentInfoField {
                        label: "Account".to_string(),
                        value: profile.name,
                    });
                    fields.push(AgentInfoField {
                        label: "Plan".to_string(),
                        value: profile.plan,
                    });
                    if !profile.plan_until.is_empty() {
                        fields.push(AgentInfoField {
                            label: "Plan until".to_string(),
                            value: profile.plan_until,
                        });
                    }
                }
            }
        }
    }
    let config_path = home_path(&[".codex", "config.toml"]);
    if let Ok(contents) = std::fs::read_to_string(&config_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("model ") {
                fields.push(AgentInfoField {
                    label: "Model".to_string(),
                    value: toml_value(line),
                });
            } else if line.starts_with("personality ") {
                fields.push(AgentInfoField {
                    label: "Personality".to_string(),
                    value: toml_value(line),
                });
            } else if line.starts_with("model_reasoning_effort ") {
                fields.push(AgentInfoField {
                    label: "Reasoning effort".to_string(),
                    value: toml_value(line),
                });
            }
        }
    }
    if let Some(counts) = codex_thread_counts() {
        fields.push(AgentInfoField {
            label: "Week threads".to_string(),
            value: counts.week.to_string(),
        });
        fields.push(AgentInfoField {
            label: "Total threads".to_string(),
            value: counts.total.to_string(),
        });
    }
    fields
}

struct CodexProfile {
    name: String,
    plan: String,
    plan_until: String,
}

fn codex_jwt_profile(auth: &Value) -> Option<CodexProfile> {
    let token = auth
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(Value::as_str)?;
    let payload_seg = token.split('.').nth(1)?;
    // base64url decode without padding
    let bytes = base64_url::decode(payload_seg).ok()?;
    let payload: Value = serde_json::from_slice(&bytes).ok()?;
    let openai = payload.get("https://api.openai.com/auth")?;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let plan = openai
        .get("chatgpt_plan_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // e.g. "2026-06-23T03:09:46+00:00" → take first 10 chars
    let plan_until = openai
        .get("chatgpt_subscription_active_until")
        .and_then(Value::as_str)
        .map(|s| s.get(..10).unwrap_or(s).to_string())
        .unwrap_or_default();
    Some(CodexProfile {
        name,
        plan,
        plan_until,
    })
}

struct CodexThreadCounts {
    week: u64,
    total: u64,
}

fn codex_thread_counts() -> Option<CodexThreadCounts> {
    let db_path = home_path(&[".codex", "state_5.sqlite"]);
    if !db_path.is_file() {
        return None;
    }
    let week_start = (Utc::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    // created_at is stored as unix millis in this DB
    let week_ts = chrono::NaiveDate::parse_from_str(&week_start, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)?
        .and_utc()
        .timestamp();
    let query = format!(
        "SELECT \
            (SELECT COUNT(*) FROM threads) AS total, \
            (SELECT COUNT(*) FROM threads WHERE created_at/1000 >= {week_ts}) AS week;"
    );
    let bytes = sqlite_readonly::query_json(&db_path, &query).ok()?;
    let rows: Vec<Value> = serde_json::from_slice(&bytes).ok()?;
    let row = rows.first()?;
    let total = row.get("total").and_then(Value::as_u64).unwrap_or(0);
    let week = row.get("week").and_then(Value::as_u64).unwrap_or(0);
    Some(CodexThreadCounts { week, total })
}

fn opencode_account_fields() -> Vec<AgentInfoField> {
    let mut fields = Vec::new();
    let config_dir = home_path(&[".config", "opencode"]);
    fields.push(AgentInfoField {
        label: "Config dir".to_string(),
        value: if config_dir.is_dir() {
            "present".to_string()
        } else {
            "missing".to_string()
        },
    });
    fields
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

fn push_json_string(fields: &mut Vec<AgentInfoField>, label: &str, value: &Value, path: &[&str]) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(text) = value_to_string(raw) else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: text,
    });
}

fn push_json_u64(fields: &mut Vec<AgentInfoField>, label: &str, value: &Value, path: &[&str]) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(number) = raw.as_u64() else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: number.to_string(),
    });
}

fn push_json_bool(fields: &mut Vec<AgentInfoField>, label: &str, value: &Value, path: &[&str]) {
    let Some(raw) = json_path(value, path) else {
        return;
    };
    let Some(flag) = raw.as_bool() else {
        return;
    };
    fields.push(AgentInfoField {
        label: label.to_string(),
        value: if flag { "yes" } else { "no" }.to_string(),
    });
}

fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "yes" } else { "no" }.to_string()),
        _ => None,
    }
}

fn claude_total_cost_usd(value: &Value) -> Option<f64> {
    let usage = value.get("modelUsage")?.as_object()?;
    let mut total = 0.0;
    for model in usage.values() {
        if let Some(cost) = model.get("costUSD").and_then(Value::as_f64) {
            total += cost;
        }
    }
    (total > 0.0).then_some(total)
}

fn toml_value(line: &str) -> String {
    line.split_once('=')
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| line.to_string())
}

fn program_name(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
    }
}

// ---------------------------------------------------------------------------
// Account-level usage probes (async, calls provider APIs)
// ---------------------------------------------------------------------------

/// Fetch live rate-limit snapshots for all managed agents that have credentials.
pub async fn scan_account_usage() -> HashMap<ManagedAgentKind, AgentUsageSnapshot> {
    let claude = tokio::spawn(fetch_claude_account_usage());
    let codex = tokio::spawn(fetch_codex_account_usage());

    let mut out = HashMap::new();
    if let Ok(Some(snap)) = claude.await {
        out.insert(ManagedAgentKind::Claude, snap);
    }
    if let Ok(Some(snap)) = codex.await {
        out.insert(ManagedAgentKind::Codex, snap);
    }
    out
}

/// Apply previously fetched usage snapshots onto cached agent summaries.
pub fn apply_cached_account_usage(
    agents: &mut Vec<AgentSummary>,
    snapshots: &HashMap<ManagedAgentKind, AgentUsageSnapshot>,
) {
    for agent in agents {
        if let Some(snap) = snapshots.get(&agent.kind) {
            agent.usage = Some(snap.clone());
        }
    }
}

async fn fetch_claude_account_usage() -> Option<AgentUsageSnapshot> {
    let token = read_claude_oauth_token()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        tracing::debug!("claude usage API returned {}", resp.status());
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let five_hour = body
        .get("five_hour")
        .and_then(|w| w.get("utilization"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let seven_day = body
        .get("seven_day")
        .and_then(|w| w.get("utilization"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    // extra_usage spend credit utilization (Claude Max / extra)
    let extra = body.get("extra_usage");
    let extra_used = extra
        .and_then(|e| e.get("used_credits"))
        .and_then(Value::as_f64);
    let extra_limit = extra
        .and_then(|e| e.get("monthly_limit"))
        .and_then(Value::as_f64);
    let extra_util = extra_used.zip(extra_limit).and_then(|(used, limit)| {
        if limit > 0.0 {
            Some((used / limit * 100.0) as f32)
        } else {
            None
        }
    });
    Some(AgentUsageSnapshot {
        context_used_percent: extra_util,
        total_cost_usd: extra_used,
        total_duration_ms: None,
        total_api_duration_ms: None,
        lines_added: None,
        lines_removed: None,
        rate_limit_five_hour_used_percent: five_hour,
        rate_limit_seven_day_used_percent: seven_day,
    })
}

async fn fetch_codex_account_usage() -> Option<AgentUsageSnapshot> {
    let auth_path = home_path(&[".codex", "auth.json"]);
    let contents = std::fs::read_to_string(&auth_path).ok()?;
    let auth: Value = serde_json::from_str(&contents).ok()?;
    let access_token = auth
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(Value::as_str)?
        .to_string();
    let account_id = auth
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "Zedra");
    if let Some(ref id) = account_id {
        req = req.header("ChatGPT-Account-Id", id.as_str());
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        tracing::debug!("codex usage API returned {}", resp.status());
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    let rl = body.get("rate_limit");
    let five_hour = rl
        .and_then(|r| r.get("primary_window"))
        .and_then(|w| w.get("used_percent"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    let seven_day = rl
        .and_then(|r| r.get("secondary_window"))
        .and_then(|w| w.get("used_percent"))
        .and_then(Value::as_f64)
        .map(|v| v as f32);
    Some(AgentUsageSnapshot {
        context_used_percent: None,
        total_cost_usd: None,
        total_duration_ms: None,
        total_api_duration_ms: None,
        lines_added: None,
        lines_removed: None,
        rate_limit_five_hour_used_percent: five_hour,
        rate_limit_seven_day_used_percent: seven_day,
    })
}

/// Read the Claude OAuth access token from `~/.claude/.credentials.json`.
/// Returns `None` if the file is missing, malformed, or the token is expired.
fn read_claude_oauth_token() -> Option<String> {
    let path = home_path(&[".claude", ".credentials.json"]);
    let contents = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&contents).ok()?;
    let oauth = root.get("claudeAiOauth")?;
    if let Some(expires_ms) = oauth.get("expiresAt").and_then(Value::as_f64) {
        let expires = std::time::UNIX_EPOCH + std::time::Duration::from_millis(expires_ms as u64);
        if std::time::SystemTime::now() > expires {
            tracing::debug!("claude oauth token expired; skipping usage probe");
            return None;
        }
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn kind_slug(kind: ManagedAgentKind) -> &'static str {
    match kind {
        ManagedAgentKind::Claude => "claude",
        ManagedAgentKind::Codex => "codex",
        ManagedAgentKind::OpenCode => "opencode",
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(version: &str) -> AgentCliSummary {
        AgentCliSummary {
            available: true,
            version: Some(version.to_string()),
            error: None,
        }
    }

    #[test]
    fn session_title_defaults_to_unknown_without_provider_title() {
        assert_eq!(session_title(None).as_deref(), Some("Unknown"));
        assert_eq!(
            session_title(Some("Fix terminal paste".into())).as_deref(),
            Some("Fix terminal paste")
        );
    }

    #[test]
    fn opencode_json_maps_safe_summary_fields() {
        let json = br#"[
          {
            "id": "ses_123",
            "title": "Fix terminal paste",
            "updated": 1777805877635,
            "created": 1777805707332,
            "projectId": "project-hash",
            "directory": "/repo",
            "transcriptSizeBytes": 8192
          }
        ]"#;

        let sessions = opencode_sessions_from_json(
            Path::new("/repo"),
            json,
            &cli("1.14.33"),
            "opencode sqlite",
        )
        .expect("parse opencode sessions");

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.kind, ManagedAgentKind::OpenCode);
        assert_eq!(session.session_id, "ses_123");
        assert_eq!(session.title.as_deref(), Some("Fix terminal paste"));
        assert_eq!(
            session.provider.native_project_id.as_deref(),
            Some("project-hash")
        );
        assert_eq!(session.provider.cli_version.as_deref(), Some("1.14.33"));
        assert!(session.resume.available);
        assert_eq!(
            session.resume.action_id.as_deref(),
            Some("opencode:ses_123")
        );
        assert_eq!(session.transcript_size_bytes, Some(8192));
        assert!(session.git.is_some());
    }

    #[test]
    fn opencode_session_summary_resolves_git_branch_and_transcript_size() {
        let workdir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root");
        let directory = workdir.display().to_string();
        let json = format!(
            r#"[{{
              "id": "ses_git",
              "title": "Branch test",
              "directory": "{directory}",
              "projectWorktree": "{directory}",
              "transcriptSizeBytes": 4096,
              "updated": 1777805877635,
              "created": 1777805707332
            }}]"#
        );

        let sessions =
            opencode_sessions_from_json(workdir, json.as_bytes(), &cli("1.14.33"), "test")
                .expect("parse opencode sessions");
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.transcript_size_bytes, Some(4096));
        let branch = session
            .git
            .as_ref()
            .and_then(|git| git.branch.as_deref())
            .unwrap_or_default();
        assert!(!branch.is_empty());
    }

    #[test]
    fn opencode_workdir_matches_linked_git_worktree_via_common_dir() {
        let workdir = Path::new("/Users/me/projects/zedra-main");
        let linked = Path::new("/Users/me/projects/zedra");
        let common = PathBuf::from("/Users/me/projects/zedra/.git");
        let session = OpenCodeSessionJson {
            id: "ses_linked".into(),
            title: Some("Linked worktree session".into()),
            updated: Some(1777805877635),
            created: Some(1777805707332),
            project_id: Some("project-hash".into()),
            directory: Some(linked.display().to_string()),
            project_worktree: Some(linked.display().to_string()),
            workspace_id: None,
            workspace_branch: None,
            workspace_directory: None,
            path: None,
            transcript_size_bytes: None,
        };

        let mut git_cache = HashMap::new();
        git_cache.insert(normalize_path(workdir), Some(common.clone()));
        git_cache.insert(normalize_path(linked), Some(common));

        assert!(opencode_workdir_matches(workdir, &session, &mut git_cache));
    }

    #[test]
    fn opencode_db_json_parses_sqlite_shape() {
        let json = br#"[
          {
            "id": "ses_db",
            "title": "From sqlite",
            "directory": "/repo",
            "created": 1777805707332,
            "updated": 1777805877635,
            "projectId": "project-hash",
            "projectWorktree": "/repo",
            "workspaceBranch": "feature/opencode",
            "transcriptSizeBytes": 2048
          }
        ]"#;
        let sessions = opencode_sessions_from_json(
            Path::new("/repo"),
            json,
            &cli("1.14.33"),
            "opencode sqlite",
        )
        .expect("parse");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("From sqlite"));
        assert_eq!(sessions[0].transcript_size_bytes, Some(2048));
        assert_eq!(
            sessions[0]
                .git
                .as_ref()
                .and_then(|git| git.branch.as_deref()),
            Some("feature/opencode")
        );
    }

    #[test]
    fn opencode_git_summary_prefers_workspace_branch_over_live_git() {
        let session = OpenCodeSessionJson {
            id: "ses_ws".into(),
            title: Some("Workspace branch".into()),
            updated: None,
            created: None,
            project_id: None,
            directory: Some(env!("CARGO_MANIFEST_DIR").to_string()),
            project_worktree: None,
            workspace_id: Some("ws_1".into()),
            workspace_branch: Some("stored-branch".into()),
            workspace_directory: Some("/repo/worktree".into()),
            path: None,
            transcript_size_bytes: None,
        };
        let mut branch_cache = HashMap::new();
        let git = opencode_git_summary(&session, &mut branch_cache).expect("git summary");
        assert_eq!(git.branch.as_deref(), Some("stored-branch"));
        assert_eq!(git.worktree.as_deref(), Some(env!("CARGO_MANIFEST_DIR")));
        assert!(branch_cache.is_empty());
    }

    #[test]
    fn codex_thread_db_json_parses_sqlite_shape() {
        let json = br#"[
          {
            "id": "019e251d-03ed-76a1-87f6-eecda6eb88a8",
            "cwd": "/repo",
            "title": "Research live activity ios",
            "rollout_path": "/home/.codex/sessions/2026/05/14/rollout.jsonl",
            "source": "vscode",
            "model_provider": "openai",
            "cli_version": "0.130.0",
            "created_at_ms": 1778746700000,
            "updated_at_ms": 1778746704000,
            "first_user_message": "research live activity",
            "agent_nickname": null,
            "agent_role": null,
            "git_branch": "main",
            "git_sha": "abc",
            "git_origin_url": "https://example.com/repo.git",
            "approval_mode": "on-request",
            "model": "gpt-5.3-codex"
          }
        ]"#;
        let threads: Vec<CodexThreadRow> = serde_json::from_slice(json).expect("parse");
        assert_eq!(threads.len(), 1);
        let summary = codex_session_summary_from_thread(&threads[0], &cli("0.130.0"));
        assert_eq!(summary.session_id, "019e251d-03ed-76a1-87f6-eecda6eb88a8");
        assert_eq!(summary.title.as_deref(), Some("Research live activity ios"));
        assert_eq!(summary.provider.source.as_deref(), Some("vscode"));
    }

    #[test]
    fn codex_thread_matches_exact_workdir_only() {
        let workdir = PathBuf::from("/Users/me/projects/zedra-main");
        let matching = CodexThreadRow {
            id: "019e".into(),
            cwd: "/Users/me/projects/zedra-main".into(),
            title: "Main session".into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: String::new(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        let sibling = CodexThreadRow {
            id: "019f".into(),
            cwd: "/Users/me/projects/zedra".into(),
            title: "Sibling session".into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: String::new(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert!(paths_equal(&workdir, Path::new(&matching.cwd)));
        assert!(!paths_equal(&workdir, Path::new(&sibling.cwd)));
    }

    #[test]
    fn codex_title_from_thread_prefers_db_title() {
        let thread = CodexThreadRow {
            id: "019e".into(),
            cwd: "/repo".into(),
            title: "Final title".into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: "initial prompt".into(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert_eq!(
            codex_title_from_thread(&thread).as_deref(),
            Some("Final title")
        );
    }

    #[test]
    fn codex_title_from_thread_prefers_db_title_over_cwd_message() {
        let thread = CodexThreadRow {
            id: "019e".into(),
            cwd: "/Users/me/projects/zedra-main".into(),
            title: "Research Gemini CLI integration".into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message:
                "CWD: /Users/me/projects/zedra-main. Research Gemini CLI integration opportunities"
                    .into(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert_eq!(
            codex_title_from_thread(&thread).as_deref(),
            Some("Research Gemini CLI integration")
        );
    }

    #[test]
    fn codex_title_from_thread_falls_back_to_preview_before_first_user_message() {
        let thread = CodexThreadRow {
            id: "019e".into(),
            cwd: "/repo".into(),
            title: String::new(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            preview: "Preview title".into(),
            first_user_message: "CWD: /repo. Raw prompt body".into(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert_eq!(
            codex_title_from_thread(&thread).as_deref(),
            Some("Preview title")
        );
    }

    #[test]
    fn codex_title_from_thread_falls_back_to_first_user_message() {
        let thread = CodexThreadRow {
            id: "019e".into(),
            cwd: "/repo".into(),
            title: String::new(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message: "research how to implement live activity ios for Zedra\n".into(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert_eq!(
            codex_title_from_thread(&thread).as_deref(),
            Some("research how to implement live activity ios for Zedra")
        );
    }

    #[test]
    fn sanitize_codex_prompt_fallback_strips_subagent_cwd_prefix() {
        assert_eq!(
            sanitize_codex_prompt_fallback(
                "CWD: /repo. Research Gemini CLI integration opportunities for Zedra"
            )
            .as_deref(),
            Some("Research Gemini CLI integration opportunities for Zedra")
        );
    }

    #[test]
    fn sanitize_codex_title_field_keeps_db_title_without_cwd_strip() {
        assert_eq!(
            sanitize_codex_title_field("Research Gemini CLI integration").as_deref(),
            Some("Research Gemini CLI integration")
        );
    }

    #[test]
    fn codex_title_from_thread_sanitizes_continue_path_db_titles() {
        let thread = CodexThreadRow {
            id: "019e".into(),
            cwd: "/Users/me/projects/zedra-main".into(),
            title: "continue /Users/me/projects/zedra-main/docs/CLAUDE_HOST_INTEGRATION_PLAN.md"
                .into(),
            rollout_path: "/home/.codex/sessions/rollout.jsonl".into(),
            source: "vscode".into(),
            model_provider: "openai".into(),
            cli_version: String::new(),
            created_at_ms: None,
            updated_at_ms: None,
            first_user_message:
                "continue /Users/me/projects/zedra-main/docs/CLAUDE_HOST_INTEGRATION_PLAN.md".into(),
            preview: String::new(),
            agent_nickname: None,
            agent_role: None,
            git_branch: None,
            git_sha: None,
            git_origin_url: None,
            approval_mode: String::new(),
            model: None,
        };
        assert_eq!(
            codex_title_from_thread(&thread).as_deref(),
            Some("CLAUDE_HOST_INTEGRATION_PLAN")
        );
    }

    #[test]
    fn codex_title_from_agent_identity_formats_role() {
        assert_eq!(
            codex_title_from_agent_identity(Some("Aquinas"), Some("explorer")).as_deref(),
            Some("Aquinas (explorer)")
        );
    }

    #[test]
    fn resume_launch_commands_are_host_owned() {
        assert_eq!(
            resume_launch_command(ManagedAgentKind::Claude, "abc").as_deref(),
            Some("claude --resume abc")
        );
        assert_eq!(
            resume_launch_command(ManagedAgentKind::Codex, "019e").as_deref(),
            Some("codex resume 019e")
        );
        assert_eq!(
            resume_launch_command(ManagedAgentKind::OpenCode, "ses_123").as_deref(),
            Some("opencode --session ses_123")
        );
    }

    #[test]
    fn claude_plugin_status_reads_installed_plugins_file() {
        let json = r#"{
          "version": 2,
          "plugins": {
            "zedra@zedra": [
              {
                "installPath": "/tmp/zedra-plugin"
              }
            ]
          }
        }"#;
        let status = claude_plugin_status_from_installed_plugins(json);
        assert!(status.plugin_installed);
        assert!(!status.hooks_installed);
        assert!(status.error.is_none());
    }

    #[test]
    fn normalizes_supported_hook_events() {
        assert_eq!(
            normalize_event(ManagedAgentKind::Claude, "PermissionRequest"),
            Some((
                AgentEventKind::PermissionRequested,
                AgentLifecycleStatus::WaitingForPermission
            ))
        );
        assert_eq!(
            normalize_event(ManagedAgentKind::Claude, "UserPromptSubmit"),
            Some((AgentEventKind::TurnStarted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(ManagedAgentKind::Claude, "PreToolUse"),
            Some((AgentEventKind::ToolStarted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(ManagedAgentKind::Claude, "SessionEnd"),
            Some((AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle))
        );
        assert_eq!(
            normalize_event(ManagedAgentKind::Codex, "PostToolUse"),
            Some((AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(ManagedAgentKind::OpenCode, "session.error"),
            Some((AgentEventKind::TurnFailed, AgentLifecycleStatus::Failed))
        );
    }
}
