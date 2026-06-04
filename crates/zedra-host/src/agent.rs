use crate::agent_cache::AgentCache;
use crate::agent_claude;
use crate::agent_codex;
use crate::agent_hermes;
use crate::agent_installed;
use crate::agent_opencode;
use crate::agent_pi;
use crate::agent_setup::setup_summary;
use crate::agent_utils::{
    capabilities, display_name, first_non_empty_line, program_name, shell_quote,
};
use crate::session_registry::ServerSession;
use chrono::Utc;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use zedra_rpc::proto::*;

pub use crate::agent_utils::home_path;

const AGENT_SESSION_DEFAULT_LIMIT: u32 = 50;
const AGENT_SESSION_MAX_LIMIT: u32 = 200;

pub const MANAGED_AGENT_KINDS: [AgentKind; 5] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::OpenCode,
    AgentKind::Pi,
    AgentKind::Hermes,
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

pub fn scan_managed_agent_cli_versions() -> HashMap<AgentKind, AgentCliSummary> {
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
    versions: &HashMap<AgentKind, AgentCliSummary>,
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

fn degraded_agent_summary(kind: AgentKind, workdir: &Path) -> AgentSummary {
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
            latest_session_id: None,
            latest_session_title: None,
        },
        last_activity_at: None,
        updated_at: Utc::now(),
        data_sources: vec![AgentDataSource::Cli, AgentDataSource::Setup],
        warnings: vec![AgentWarning {
            code: "session_scan_panicked".to_string(),
            message: "agent session scan crashed; counts unavailable".to_string(),
        }],
        account: account_summary(kind, workdir),
        usage: None,
    }
}

pub fn scan_agent_sessions(kind: AgentKind, workdir: &Path, limit: u32) -> AgentSessionsResult {
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
    kind: AgentKind,
    workdir: &Path,
    session: Option<&Arc<ServerSession>>,
    limit: u32,
    refresh: bool,
) -> AgentSessionsResult {
    cache.sessions(kind, workdir, session, limit, refresh).await
}

pub fn resume_launch_command(kind: AgentKind, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    Some(dispatch(kind).resume_launch_command(&shell_quote(session_id)))
}

pub fn normalize_event(
    kind: AgentKind,
    event_name: &str,
) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
    let event_name = event_name.trim();
    if event_name.is_empty() {
        return None;
    }
    dispatch(kind).normalize_event(event_name)
}

/// True for agents whose sessions are not scoped to a workspace (Hermes). Their
/// scans ignore the workdir, so callers can reuse a cached result across
/// workspace switches.
pub fn is_global(kind: AgentKind) -> bool {
    dispatch(kind).is_global()
}

pub fn managed_kind_from_slug(raw: &str) -> Option<AgentKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "claude" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        "opencode" | "open-code" | "open_code" => Some(AgentKind::OpenCode),
        "pi" => Some(AgentKind::Pi),
        "hermes" => Some(AgentKind::Hermes),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Agent summary scanning (dispatcher)
// ---------------------------------------------------------------------------

fn agent_summary_scan(kind: AgentKind, workdir: &Path) -> AgentSummary {
    let now = Utc::now();
    let cli = agent_list_cli_summary(kind, workdir);
    let setup = setup_summary(kind, cli.available, workdir);
    let mut warnings = Vec::new();
    let counts = match dispatch(kind).session_counts(&ScanCtx { workdir, cli: &cli }) {
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
        data_sources.push(dispatch(kind).scan_data_source());
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
            latest_session_id: counts.latest_session_id.clone(),
            latest_session_title: counts.latest_session_title.clone(),
        },
        last_activity_at: counts.last_activity_at,
        updated_at: now,
        data_sources,
        warnings,
        account: account_summary(kind, workdir),
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

// Most agents have no provider project id; only the OpenCode conversion below
// carries one. The four identical conversions share this macro.
macro_rules! session_counts_from {
    ($t:ty) => {
        impl From<$t> for SessionCounts {
            fn from(c: $t) -> Self {
                SessionCounts {
                    total: c.total,
                    resumable: c.resumable,
                    latest_session_id: c.latest_session_id,
                    latest_session_title: c.latest_session_title,
                    last_activity_at: c.last_activity_at,
                    provider_project_id: None,
                }
            }
        }
    };
}
session_counts_from!(agent_claude::SessionCounts);
session_counts_from!(agent_codex::SessionCounts);
session_counts_from!(agent_pi::SessionCounts);
session_counts_from!(agent_hermes::SessionCounts);

impl From<agent_opencode::SessionCounts> for SessionCounts {
    fn from(c: agent_opencode::SessionCounts) -> Self {
        SessionCounts {
            total: c.total,
            resumable: c.resumable,
            latest_session_id: c.latest_session_id,
            latest_session_title: c.latest_session_title,
            last_activity_at: c.last_activity_at,
            provider_project_id: c.provider_project_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Managed agent registry
//
// Per-kind behavior is dispatched through a single `ManagedAgent` trait object
// (`dispatch(kind)`) so each agent's logic lives in one impl instead of being
// spread across parallel `match kind { .. }` arms. The async account/usage
// fan-outs in `scan_account_plans` / `scan_account_usage` stay separate: they
// orchestrate concurrency over all kinds rather than branching per kind.
// ---------------------------------------------------------------------------

/// Inputs a blocking scan needs: the active workspace and this kind's
/// already-probed CLI availability. Workspace-global agents
/// ([`ManagedAgent::is_global`]) ignore `workdir`.
struct ScanCtx<'a> {
    workdir: &'a Path,
    cli: &'a AgentCliSummary,
}

trait ManagedAgent: Sync {
    fn kind(&self) -> AgentKind;

    /// True for agents whose sessions are not scoped to a workspace (Hermes):
    /// they ignore the scan `workdir` and surface the same sessions everywhere.
    fn is_global(&self) -> bool {
        false
    }

    fn normalize_event(&self, event_name: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)>;

    fn cli_available(&self, workdir: &Path) -> bool;

    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String>;

    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String>;

    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField>;

    /// Read-only config/memory files for the detail view; empty for agents that
    /// expose none.
    fn config_files(&self) -> Vec<AgentFile> {
        Vec::new()
    }

    /// Data source attributed to a non-empty historical scan. Defaults to a
    /// local history read; CLI-backed agents (OpenCode) override.
    fn scan_data_source(&self) -> AgentDataSource {
        AgentDataSource::HistoricalScan
    }

    /// CLI summary used for a *session list* scan. Defaults to the same probe as
    /// the agent list; agents with a dedicated probe (OpenCode) override.
    fn session_scan_cli(&self, workdir: &Path) -> AgentCliSummary {
        agent_list_cli_summary(self.kind(), workdir)
    }

    /// True if `command` (the foreground shell command) is this agent's CLI.
    fn command_matches(&self, command: &str) -> bool;

    /// Resume session id parsed from the foreground command tokens, if present.
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String>;

    /// Shell command that resumes `quoted_session_id` (already shell-quoted).
    fn resume_launch_command(&self, quoted_session_id: &str) -> String;
}

fn dispatch(kind: AgentKind) -> &'static dyn ManagedAgent {
    match kind {
        AgentKind::Claude => &ClaudeAgent,
        AgentKind::Codex => &CodexAgent,
        AgentKind::OpenCode => &OpenCodeAgent,
        AgentKind::Pi => &PiAgent,
        AgentKind::Hermes => &HermesAgent,
    }
}

struct ClaudeAgent;
impl ManagedAgent for ClaudeAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Claude
    }
    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_claude::normalize_event(event)
    }
    fn cli_available(&self, workdir: &Path) -> bool {
        agent_claude::cli_available(workdir)
    }
    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_claude::session_counts(ctx.workdir)?.into())
    }
    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_claude::sessions(ctx.workdir, ctx.cli, limit)
    }
    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        agent_claude::account_fields()
    }
    fn command_matches(&self, command: &str) -> bool {
        command.to_ascii_lowercase().contains("claude")
    }
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        value_after_flag(tokens, "--resume")
    }
    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("claude --resume {quoted}")
    }
}

struct CodexAgent;
impl ManagedAgent for CodexAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }
    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_codex::normalize_event(event)
    }
    fn cli_available(&self, _workdir: &Path) -> bool {
        agent_codex::cli_available()
    }
    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_codex::session_counts(ctx.workdir)?.into())
    }
    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_codex::sessions(ctx.workdir, ctx.cli, limit)
    }
    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        agent_codex::account_fields()
    }
    fn command_matches(&self, command: &str) -> bool {
        command.to_ascii_lowercase().contains("codex")
    }
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        let resume_index = tokens.iter().position(|token| *token == "resume")?;
        tokens
            .get(resume_index + 1)
            .filter(|value| !value.starts_with('-'))
            .map(|value| value.trim_matches('"').trim_matches('\'').to_string())
    }
    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("codex resume {quoted}")
    }
}

struct OpenCodeAgent;
impl ManagedAgent for OpenCodeAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::OpenCode
    }
    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_opencode::normalize_event(event)
    }
    fn cli_available(&self, _workdir: &Path) -> bool {
        agent_opencode::cli_available()
    }
    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_opencode::session_counts(ctx.workdir, ctx.cli)?.into())
    }
    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_opencode::sessions(ctx.workdir, ctx.cli, limit)
    }
    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        agent_opencode::account_fields()
    }
    fn scan_data_source(&self) -> AgentDataSource {
        AgentDataSource::ProviderCli
    }
    fn session_scan_cli(&self, _workdir: &Path) -> AgentCliSummary {
        agent_opencode::session_cli_summary()
    }
    fn command_matches(&self, command: &str) -> bool {
        let low = command.to_ascii_lowercase();
        low.contains("opencode") || low.contains("open-code")
    }
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        value_after_flag(tokens, "--session").or_else(|| value_after_flag(tokens, "-s"))
    }
    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("opencode --session {quoted}")
    }
}

struct PiAgent;
impl ManagedAgent for PiAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Pi
    }
    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_pi::normalize_event(event)
    }
    fn cli_available(&self, _workdir: &Path) -> bool {
        agent_pi::cli_available()
    }
    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_pi::session_counts(ctx.workdir)?.into())
    }
    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_pi::sessions(ctx.workdir, ctx.cli, limit)
    }
    fn account_fields(&self, workdir: &Path) -> Vec<AgentInfoField> {
        // Pi merges global + project (`<workdir>/.pi`) config, so it needs the workdir.
        agent_pi::account_fields(workdir)
    }
    fn command_matches(&self, command: &str) -> bool {
        // Pi shares its name with common words, so match only when it is the
        // invoked program (first token), bare or path-qualified.
        command_program_is(&command.to_ascii_lowercase(), "pi")
    }
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        value_after_flag(tokens, "--session")
    }
    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("pi --session {quoted}")
    }
}

struct HermesAgent;
impl ManagedAgent for HermesAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Hermes
    }
    fn is_global(&self) -> bool {
        true
    }
    fn normalize_event(&self, event: &str) -> Option<(AgentEventKind, AgentLifecycleStatus)> {
        agent_hermes::normalize_event(event)
    }
    fn cli_available(&self, _workdir: &Path) -> bool {
        agent_hermes::cli_available()
    }
    fn session_counts(&self, ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(agent_hermes::session_counts(ctx.workdir)?.into())
    }
    fn sessions(
        &self,
        ctx: &ScanCtx,
        limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        agent_hermes::sessions(ctx.workdir, ctx.cli, limit)
    }
    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        // Hermes is global; its config/auth is workspace-independent.
        agent_hermes::account_fields()
    }
    fn config_files(&self) -> Vec<AgentFile> {
        agent_hermes::config_files()
    }
    fn command_matches(&self, command: &str) -> bool {
        command_program_is(&command.to_ascii_lowercase(), "hermes")
    }
    fn infer_session_id(&self, tokens: &[&str]) -> Option<String> {
        value_after_flag(tokens, "--resume").or_else(|| value_after_flag(tokens, "-r"))
    }
    fn resume_launch_command(&self, quoted: &str) -> String {
        format!("hermes --resume {quoted}")
    }
}

fn sessions_for_kind_blocking(
    kind: AgentKind,
    workdir: &Path,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, u32), String> {
    let agent = dispatch(kind);
    let cli = agent.session_scan_cli(workdir);
    let (mut sessions, total) = agent.sessions(&ScanCtx { workdir, cli: &cli }, limit)?;
    let total = u32::try_from(total).unwrap_or(u32::MAX);
    sessions.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
    Ok((sessions, total))
}

fn cli_version_summary(kind: AgentKind) -> AgentCliSummary {
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

fn agent_list_cli_summary(kind: AgentKind, workdir: &Path) -> AgentCliSummary {
    let available = dispatch(kind).cli_available(workdir);
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

fn command_program_is(command: &str, program: &str) -> bool {
    command
        .split_whitespace()
        .next()
        .is_some_and(|first| first == program || first.ends_with(&format!("/{program}")))
}

// ---------------------------------------------------------------------------
// Account snapshot dispatch
// ---------------------------------------------------------------------------

fn account_summary(kind: AgentKind, workdir: &Path) -> AgentAccountSummary {
    AgentAccountSummary {
        fields: dispatch(kind).account_fields(workdir),
    }
}

/// Read-only config/memory files for an agent's detail view. Only Hermes
/// exposes a file set today; other agents return none.
pub fn agent_files(kind: AgentKind) -> Vec<AgentFile> {
    dispatch(kind).config_files()
}

pub async fn scan_account_plans() -> HashMap<AgentKind, Vec<AgentInfoField>> {
    let claude = tokio::spawn(agent_claude::fetch_subscription_plan());
    let codex = tokio::task::spawn_blocking(agent_codex::subscription_plan_fields);
    let opencode = tokio::task::spawn_blocking(agent_opencode::subscription_plan_fields);
    let pi = tokio::task::spawn_blocking(agent_pi::subscription_plan_fields);
    let hermes = tokio::task::spawn_blocking(agent_hermes::subscription_plan_fields);

    let mut out = HashMap::new();
    if let Ok(Some(fields)) = claude.await {
        out.insert(AgentKind::Claude, fields);
    }
    if let Ok(Some(fields)) = codex.await {
        out.insert(AgentKind::Codex, fields);
    }
    if let Ok(Some(fields)) = opencode.await {
        out.insert(AgentKind::OpenCode, fields);
    }
    if let Ok(Some(fields)) = pi.await {
        out.insert(AgentKind::Pi, fields);
    }
    if let Ok(Some(fields)) = hermes.await {
        out.insert(AgentKind::Hermes, fields);
    }
    out
}

pub fn apply_cached_account_plans(
    agents: &mut [AgentSummary],
    plans: &HashMap<AgentKind, Vec<AgentInfoField>>,
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

pub async fn scan_account_usage() -> HashMap<AgentKind, AgentUsageSnapshot> {
    let claude = tokio::spawn(agent_claude::fetch_account_usage());
    let codex = tokio::spawn(agent_codex::fetch_account_usage());
    let opencode = tokio::task::spawn_blocking(agent_opencode::fetch_account_usage);
    let pi = tokio::task::spawn_blocking(agent_pi::fetch_account_usage);
    let hermes = tokio::task::spawn_blocking(agent_hermes::fetch_account_usage);

    let mut out = HashMap::new();
    if let Ok(Some(snap)) = claude.await {
        out.insert(AgentKind::Claude, snap);
    }
    if let Ok(Some(snap)) = codex.await {
        out.insert(AgentKind::Codex, snap);
    }
    if let Ok(Some(snap)) = opencode.await {
        out.insert(AgentKind::OpenCode, snap);
    }
    if let Ok(Some(snap)) = pi.await {
        out.insert(AgentKind::Pi, snap);
    }
    if let Ok(Some(snap)) = hermes.await {
        out.insert(AgentKind::Hermes, snap);
    }
    out
}

pub fn apply_cached_account_usage(
    agents: &mut Vec<AgentSummary>,
    snapshots: &HashMap<AgentKind, AgentUsageSnapshot>,
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
            resume_launch_command(AgentKind::Claude, "abc").as_deref(),
            Some("claude --resume abc")
        );
        assert_eq!(
            resume_launch_command(AgentKind::Codex, "019e").as_deref(),
            Some("codex resume 019e")
        );
        assert_eq!(
            resume_launch_command(AgentKind::OpenCode, "ses_123").as_deref(),
            Some("opencode --session ses_123")
        );
        assert_eq!(
            resume_launch_command(AgentKind::Pi, "abc-def").as_deref(),
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
            normalize_event(AgentKind::Claude, "PermissionRequest"),
            Some((
                AgentEventKind::PermissionRequested,
                AgentLifecycleStatus::WaitingForPermission
            ))
        );
        assert_eq!(
            normalize_event(AgentKind::Claude, "UserPromptSubmit"),
            Some((AgentEventKind::TurnStarted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(AgentKind::Claude, "PreToolUse"),
            Some((AgentEventKind::ToolStarted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(AgentKind::Claude, "SessionEnd"),
            Some((AgentEventKind::SessionUpdated, AgentLifecycleStatus::Idle))
        );
        assert_eq!(
            normalize_event(AgentKind::Codex, "PostToolUse"),
            Some((AgentEventKind::ToolCompleted, AgentLifecycleStatus::Running))
        );
        assert_eq!(
            normalize_event(AgentKind::OpenCode, "session.error"),
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

// ---------------------------------------------------------------------------
// Hook payload helpers
// ---------------------------------------------------------------------------

pub fn hook_string(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
