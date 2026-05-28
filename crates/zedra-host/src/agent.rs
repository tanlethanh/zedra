use crate::agent_cache::AgentCache;
use crate::agent_claude;
use crate::agent_codex;
use crate::agent_installed;
use crate::agent_opencode;
use crate::agent_pi;
use crate::agent_setup::setup_summary;
use crate::agent_utils::{
    capabilities, display_name, first_non_empty_line, program_name, shell_quote,
};
use crate::session_registry::{HostShellState, HostTermMeta, ServerSession};
use chrono::Utc;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use zedra_rpc::proto::*;

pub use crate::agent_utils::home_path;

const AGENT_SESSION_DEFAULT_LIMIT: u32 = 50;
const AGENT_SESSION_MAX_LIMIT: u32 = 200;

pub const MANAGED_AGENT_KINDS: [ManagedAgentKind; 4] = [
    ManagedAgentKind::Claude,
    ManagedAgentKind::Codex,
    ManagedAgentKind::OpenCode,
    ManagedAgentKind::Pi,
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
    agent_installed::list_installed_agents()
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
        ManagedAgentKind::Pi => format!("pi --session {quoted}"),
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
        ManagedAgentKind::Claude => agent_claude::normalize_event(event_name),
        ManagedAgentKind::Codex => agent_codex::normalize_event(event_name),
        ManagedAgentKind::OpenCode => agent_opencode::normalize_event(event_name),
        ManagedAgentKind::Pi => agent_pi::normalize_event(event_name),
    }
}

pub fn managed_kind_from_slug(raw: &str) -> Option<ManagedAgentKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "claude" => Some(ManagedAgentKind::Claude),
        "codex" => Some(ManagedAgentKind::Codex),
        "opencode" | "open-code" | "open_code" => Some(ManagedAgentKind::OpenCode),
        "pi" => Some(ManagedAgentKind::Pi),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Agent summary scanning (dispatcher)
// ---------------------------------------------------------------------------

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
    last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
    provider_project_id: Option<String>,
}

fn session_counts_for_kind(
    kind: ManagedAgentKind,
    workdir: &Path,
    cli: &AgentCliSummary,
) -> Result<SessionCounts, String> {
    match kind {
        ManagedAgentKind::Claude => {
            let c = agent_claude::session_counts(workdir)?;
            Ok(SessionCounts {
                total: c.total,
                resumable: c.resumable,
                latest_session_id: c.latest_session_id,
                latest_session_title: c.latest_session_title,
                last_activity_at: c.last_activity_at,
                provider_project_id: None,
            })
        }
        ManagedAgentKind::Codex => {
            let c = agent_codex::session_counts(workdir)?;
            Ok(SessionCounts {
                total: c.total,
                resumable: c.resumable,
                latest_session_id: c.latest_session_id,
                latest_session_title: c.latest_session_title,
                last_activity_at: c.last_activity_at,
                provider_project_id: None,
            })
        }
        ManagedAgentKind::OpenCode => {
            let c = agent_opencode::session_counts(workdir, cli)?;
            Ok(SessionCounts {
                total: c.total,
                resumable: c.resumable,
                latest_session_id: c.latest_session_id,
                latest_session_title: c.latest_session_title,
                last_activity_at: c.last_activity_at,
                provider_project_id: c.provider_project_id,
            })
        }
        ManagedAgentKind::Pi => {
            let c = agent_pi::session_counts(workdir)?;
            Ok(SessionCounts {
                total: c.total,
                resumable: c.resumable,
                latest_session_id: c.latest_session_id,
                latest_session_title: c.latest_session_title,
                last_activity_at: c.last_activity_at,
                provider_project_id: None,
            })
        }
    }
}

fn sessions_for_kind_blocking(
    kind: ManagedAgentKind,
    workdir: &Path,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, u32), String> {
    let cli = match kind {
        ManagedAgentKind::OpenCode => agent_opencode::session_cli_summary(),
        _ => agent_list_cli_summary(kind, workdir),
    };
    let mut sessions = match kind {
        ManagedAgentKind::Claude => agent_claude::sessions(workdir, &cli, limit),
        ManagedAgentKind::Codex => agent_codex::sessions(workdir, &cli, limit),
        ManagedAgentKind::OpenCode => agent_opencode::sessions(workdir, &cli, limit),
        ManagedAgentKind::Pi => agent_pi::sessions(workdir, &cli, limit),
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
        ManagedAgentKind::Claude => agent_claude::cli_available(workdir),
        ManagedAgentKind::Codex => agent_codex::cli_available(),
        ManagedAgentKind::OpenCode => agent_opencode::cli_available(),
        ManagedAgentKind::Pi => agent_pi::cli_available(),
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

// ---------------------------------------------------------------------------
// Live binding (terminal -> agent session)
// ---------------------------------------------------------------------------

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
        ManagedAgentKind::Pi => "pi",
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
        ManagedAgentKind::Pi => low
            .split_whitespace()
            .next()
            .map(|first| first == "pi" || first.ends_with("/pi"))
            .unwrap_or(false),
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
        ManagedAgentKind::Pi => value_after_flag(&tokens, "--session"),
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

// ---------------------------------------------------------------------------
// Account snapshot dispatch
// ---------------------------------------------------------------------------

fn account_summary(kind: ManagedAgentKind) -> AgentAccountSummary {
    let fields = match kind {
        ManagedAgentKind::Claude => agent_claude::account_fields(),
        ManagedAgentKind::Codex => agent_codex::account_fields(),
        ManagedAgentKind::OpenCode => agent_opencode::account_fields(),
        ManagedAgentKind::Pi => agent_pi::account_fields(),
    };
    AgentAccountSummary { fields }
}

pub async fn scan_account_plans() -> HashMap<ManagedAgentKind, Vec<AgentInfoField>> {
    let claude = tokio::spawn(agent_claude::fetch_subscription_plan());
    let codex = tokio::task::spawn_blocking(agent_codex::subscription_plan_fields);
    let opencode = tokio::task::spawn_blocking(agent_opencode::subscription_plan_fields);
    let pi = tokio::task::spawn_blocking(agent_pi::subscription_plan_fields);

    let mut out = HashMap::new();
    if let Ok(Some(fields)) = claude.await {
        out.insert(ManagedAgentKind::Claude, fields);
    }
    if let Ok(Some(fields)) = codex.await {
        out.insert(ManagedAgentKind::Codex, fields);
    }
    if let Ok(Some(fields)) = opencode.await {
        out.insert(ManagedAgentKind::OpenCode, fields);
    }
    if let Ok(Some(fields)) = pi.await {
        out.insert(ManagedAgentKind::Pi, fields);
    }
    out
}

pub fn apply_cached_account_plans(
    agents: &mut [AgentSummary],
    plans: &HashMap<ManagedAgentKind, Vec<AgentInfoField>>,
) {
    for agent in agents {
        let Some(remote) = plans.get(&agent.kind) else {
            continue;
        };
        merge_account_fields(&mut agent.account.fields, remote);
    }
}

fn merge_account_fields(existing: &mut Vec<AgentInfoField>, remote: &[AgentInfoField]) {
    for field in remote {
        if let Some(slot) = existing.iter_mut().find(|entry| entry.label == field.label) {
            slot.value = field.value.clone();
        } else {
            existing.push(field.clone());
        }
    }
}

pub async fn scan_account_usage() -> HashMap<ManagedAgentKind, AgentUsageSnapshot> {
    let claude = tokio::spawn(agent_claude::fetch_account_usage());
    let codex = tokio::spawn(agent_codex::fetch_account_usage());
    let opencode = tokio::task::spawn_blocking(agent_opencode::fetch_account_usage);
    let pi = tokio::task::spawn_blocking(agent_pi::fetch_account_usage);

    let mut out = HashMap::new();
    if let Ok(Some(snap)) = claude.await {
        out.insert(ManagedAgentKind::Claude, snap);
    }
    if let Ok(Some(snap)) = codex.await {
        out.insert(ManagedAgentKind::Codex, snap);
    }
    if let Ok(Some(snap)) = opencode.await {
        out.insert(ManagedAgentKind::OpenCode, snap);
    }
    if let Ok(Some(snap)) = pi.await {
        out.insert(ManagedAgentKind::Pi, snap);
    }
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            resume_launch_command(ManagedAgentKind::Pi, "abc-def").as_deref(),
            Some("pi --session abc-def")
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
        let status = crate::agent_setup::claude_plugin_status_from_installed_plugins(json);
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

    #[test]
    fn session_title_defaults_to_unknown_without_provider_title() {
        use crate::agent_utils::session_title;
        assert_eq!(session_title(None).as_deref(), Some("Unknown"));
        assert_eq!(
            session_title(Some("Fix terminal paste".into())).as_deref(),
            Some("Fix terminal paste")
        );
    }
}
