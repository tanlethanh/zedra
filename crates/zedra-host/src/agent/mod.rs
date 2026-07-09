use crate::session_registry::ServerSession;
use chrono::Utc;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use zedra_rpc::proto::*;

macro_rules! simple_actor {
    ($name:ident, $slug:literal, $display:literal, $icon:literal, [$($program:literal),+ $(,)?], [$($alias:literal),* $(,)?]) => {
        pub(super) struct $name;

        impl $crate::agent::AgentActor for $name {
            fn slug(&self) -> &'static str { $slug }
            fn display_name(&self) -> &'static str { $display }
            fn icon_name(&self) -> &'static str { $icon }
            fn programs(&self) -> &'static [&'static str] { &[$($program),+] }
            fn detect_aliases(&self) -> &'static [&'static str] { &[$($alias),*] }
        }
    };
}

mod amp;
pub mod cache;
pub(crate) mod claude;
mod claude_probe;
pub mod cli;
mod cline;
pub(crate) mod codex;
mod copilot;
mod cursor;
pub mod detect;
mod gemini;
mod goose;
pub mod hermes;
pub mod hook;
mod installed;
mod junie;
mod kilocode;
mod openclaw;
pub(crate) mod opencode;
mod openhands;
pub(crate) mod pi;
mod qoder;
mod qwen;
pub mod setup;
mod trae;
pub(crate) mod utils;
mod zencoder;

use cache::AgentCache;
use hook::HookContext;
use utils::{first_non_empty_line, shell_quote};

pub use setup::{SetupAction, SetupCliCtx};
pub use utils::home_path;

const AGENT_SESSION_DEFAULT_LIMIT: u32 = 50;
const AGENT_SESSION_MAX_LIMIT: u32 = 200;

pub type ActorFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub const fn hooks_enabled() -> bool {
    cfg!(debug_assertions)
}

pub(crate) fn setup_status(
    cli_available: bool,
    skills_installed: bool,
    plugin_installed: bool,
    hooks_installed: bool,
    error: Option<String>,
) -> AgentSetupSummary {
    if !cli_available {
        return AgentSetupSummary {
            state: AgentSetupState::MissingCli,
            skills_installed: false,
            plugin_installed: false,
            hooks_installed: false,
            error: None,
        };
    }
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

pub(crate) fn hook_file_mentions_zedra(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|contents| contents.contains("zedra-agent-hook") || contents.contains("agent hook"))
        .unwrap_or(false)
}

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
    installed::list_installed_agents()
}

pub fn scan_agent_cli_versions() -> HashMap<String, AgentCliSummary> {
    let mut versions = HashMap::with_capacity(actors().len());
    std::thread::scope(|scope| {
        // Only detail-view actors display a version; skip the subprocess
        // spawn for detect-only actors.
        let handles: Vec<_> = actors()
            .iter()
            .filter(|actor| actor.shows_detail())
            .map(|actor| (*actor, scope.spawn(move || actor.cli_version_summary())))
            .collect();
        for (actor, handle) in handles {
            match handle.join() {
                Ok(summary) => {
                    versions.insert(actor.slug().to_string(), summary);
                }
                Err(_) => {
                    tracing::warn!(agent = actor.slug(), "agent version probe panicked");
                }
            }
        }
    });
    versions
}

pub fn apply_cached_cli_versions(
    agents: &mut [AgentSummary],
    versions: &HashMap<String, AgentCliSummary>,
) {
    for agent in agents {
        let Some(cached) = versions.get(&agent.slug) else {
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
    let mut agents = Vec::with_capacity(actors().len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = actors()
            .iter()
            .map(|actor| {
                (
                    *actor,
                    scope.spawn(move || agent_summary_scan(*actor, workdir)),
                )
            })
            .collect();
        for (actor, handle) in handles {
            match handle.join() {
                Ok(summary) => agents.push(summary),
                Err(_) => {
                    tracing::warn!(
                        agent = actor.slug(),
                        "agent list scan thread panicked; using degraded summary"
                    );
                    agents.push(degraded_agent_summary(actor, workdir));
                }
            }
        }
    });
    AgentListResult {
        agents,
        error: None,
    }
}

fn degraded_agent_summary(actor: &dyn AgentActor, workdir: &Path) -> AgentSummary {
    let warnings = vec![AgentWarning {
        code: "session_scan_panicked".to_string(),
        message: "agent session scan crashed; counts unavailable".to_string(),
    }];
    let cli = actor.agent_list_cli_summary(workdir);
    build_agent_summary(actor, workdir, cli, SessionCounts::default(), warnings)
}

pub fn scan_agent_sessions(slug: &str, workdir: &Path, limit: u32) -> AgentSessionsResult {
    let limit = agent_session_limit(limit);
    let Some(actor) = actor(slug) else {
        return AgentSessionsResult {
            sessions: Vec::new(),
            total: 0,
            error: Some(format!("unsupported agent: {slug}")),
        };
    };
    match sessions_for_actor_blocking(actor, workdir, limit) {
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
    slug: &str,
    workdir: &Path,
    session: Option<&Arc<ServerSession>>,
    limit: u32,
    refresh: bool,
) -> AgentSessionsResult {
    cache.sessions(slug, workdir, session, limit, refresh).await
}

pub fn resume_launch_command(slug: &str, session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        return None;
    }
    actor(slug)?.resume_launch_command(&shell_quote(session_id))
}

/// Agents whose sessions ignore the workdir (Hermes); cached results stay
/// valid across workspace switches.
pub fn is_global(slug: &str) -> bool {
    actor(slug).is_some_and(AgentActor::is_global)
}

pub(crate) fn actor(raw: &str) -> Option<&'static dyn AgentActor> {
    let slug = raw.trim().to_ascii_lowercase();
    actors()
        .iter()
        .copied()
        .find(|actor| actor.matches_slug(&slug))
}

// ---------------------------------------------------------------------------
// Agent summary scanning (dispatcher)
// ---------------------------------------------------------------------------

fn agent_summary_scan(actor: &dyn AgentActor, workdir: &Path) -> AgentSummary {
    let cli = actor.agent_list_cli_summary(workdir);
    let mut warnings = Vec::new();
    let counts = match actor.session_counts(&ScanCtx { workdir, cli: &cli }) {
        Ok(counts) => counts,
        Err(error) => {
            warnings.push(AgentWarning {
                code: "session_scan_failed".to_string(),
                message: error,
            });
            SessionCounts::default()
        }
    };
    build_agent_summary(actor, workdir, cli, counts, warnings)
}

fn build_agent_summary(
    actor: &dyn AgentActor,
    workdir: &Path,
    cli: AgentCliSummary,
    counts: SessionCounts,
    warnings: Vec<AgentWarning>,
) -> AgentSummary {
    let setup = actor.setup_summary(cli.available, workdir);
    let mut data_sources = vec![AgentDataSource::Cli, AgentDataSource::Setup];
    if counts.total > 0 {
        data_sources.push(actor.scan_data_source());
    }
    AgentSummary {
        slug: actor.slug().to_string(),
        display_name: actor.display_name().to_string(),
        cli,
        setup,
        workspace: AgentWorkspaceSummary {
            workdir: workdir.to_string_lossy().into_owned(),
            provider_project_id: counts.provider_project_id,
            provider_project_key: None,
        },
        sessions: AgentSessionCounts {
            total: counts.total,
            resumable: counts.resumable,
            latest_session_id: counts.latest_session_id,
            latest_session_title: counts.latest_session_title,
        },
        last_activity_at: counts.last_activity_at,
        updated_at: Utc::now(),
        data_sources,
        warnings,
        account: account_summary(actor, workdir),
        usage: None,
        highlight: String::new(),
        shows_detail: actor.shows_detail(),
    }
}

#[derive(Default)]
pub(crate) struct SessionCounts {
    total: usize,
    resumable: usize,
    latest_session_id: Option<String>,
    latest_session_title: Option<String>,
    last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
    provider_project_id: Option<String>,
}

impl SessionCounts {
    /// Counts for an all-resumable agent, seeded from the latest session.
    /// Agents carrying a `provider_project_id` (OpenCode) build directly.
    pub(crate) fn from_latest(
        total: usize,
        latest_session_id: Option<String>,
        latest_session_title: Option<String>,
        last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Self {
        SessionCounts {
            total,
            resumable: total,
            latest_session_id,
            latest_session_title,
            last_activity_at,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Agent actor registry
// ---------------------------------------------------------------------------

/// Inputs a blocking scan needs: the active workspace and this kind's
/// already-probed CLI availability. Workspace-global actors ignore `workdir`.
pub(crate) struct ScanCtx<'a> {
    pub(crate) workdir: &'a Path,
    pub(crate) cli: &'a AgentCliSummary,
}

pub(crate) trait AgentActor: Sync {
    /// Stable cross-process identity. This is the only agent identifier sent
    /// over RPC, so adding an actor never changes postcard enum layout.
    fn slug(&self) -> &'static str;

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    fn matches_slug(&self, slug: &str) -> bool {
        slug == self.slug() || self.aliases().contains(&slug)
    }

    fn display_name(&self) -> &'static str;

    /// Bare `assets/icons/<slug>.svg` name, identical on every platform; a few
    /// differ from the slug for branding (codex -> `openai`).
    fn icon_name(&self) -> &'static str;

    /// Executables that can launch this actor, in preference order.
    fn programs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Aliases this agent is known by, matched as whole-word/phrase substrings of
    /// the foreground command (handles `amp`, `cursor-agent`, `npx @openai/codex`).
    fn detect_aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Short names that identify this agent only as the entire command, for
    /// tokens that double as common words or flag values (`pi`, `hermes`).
    fn detect_exact(&self) -> &'static [&'static str] {
        &[]
    }

    /// True for agents whose sessions are not scoped to a workspace (Hermes):
    /// they ignore the scan `workdir` and surface the same sessions everywhere.
    fn is_global(&self) -> bool {
        false
    }

    /// First launch program found on PATH. The single availability probe:
    /// `cli_available`, version probing, and the installed list all key on it.
    fn resolved_program(&self) -> Option<&'static str> {
        self.programs()
            .iter()
            .copied()
            .find(|program| utils::command_on_path(program))
    }

    fn cli_available(&self, _workdir: &Path) -> bool {
        self.resolved_program().is_some()
    }

    fn cli_version_summary(&self) -> AgentCliSummary {
        let Some(program) = self.resolved_program() else {
            return AgentCliSummary {
                available: false,
                version: None,
                error: Some(if self.programs().is_empty() {
                    "agent has no launch command".to_string()
                } else {
                    "agent CLI not found on PATH".to_string()
                }),
            };
        };
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

    fn setup_summary(&self, cli_available: bool, _workdir: &Path) -> AgentSetupSummary {
        AgentSetupSummary {
            state: if cli_available {
                AgentSetupState::NotConfigured
            } else {
                AgentSetupState::MissingCli
            },
            skills_installed: false,
            plugin_installed: false,
            hooks_installed: false,
            error: None,
        }
    }

    /// List this agent on the manage screen; detect-only actors have no detail.
    fn shows_detail(&self) -> bool {
        false
    }

    fn session_counts(&self, _ctx: &ScanCtx) -> Result<SessionCounts, String> {
        Ok(SessionCounts::default())
    }

    fn sessions(
        &self,
        _ctx: &ScanCtx,
        _limit: usize,
    ) -> Result<(Vec<AgentSessionSummary>, usize), String> {
        Ok((Vec::new(), 0))
    }

    fn account_fields(&self, _workdir: &Path) -> Vec<AgentInfoField> {
        Vec::new()
    }

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

    fn agent_list_cli_summary(&self, workdir: &Path) -> AgentCliSummary {
        let available = self.cli_available(workdir);
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
                    self.slug()
                )),
            }
        }
    }

    /// Shell command that resumes `quoted_session_id` (already shell-quoted).
    fn resume_launch_command(&self, _quoted_session_id: &str) -> Option<String> {
        None
    }

    fn subscription_plan<'a>(&'a self) -> ActorFuture<'a, Option<Vec<AgentInfoField>>> {
        Box::pin(async { None })
    }

    fn account_usage<'a>(&'a self) -> ActorFuture<'a, Option<AgentUsageSnapshot>> {
        Box::pin(async { None })
    }

    /// Gates the shared `receive_hook` driver; hooked actors opt in.
    fn supports_hooks(&self) -> bool {
        false
    }

    /// Event name and agent session id from a hook payload.
    fn hook_identity(&self, payload: &serde_json::Value) -> (String, Option<String>) {
        (
            utils::payload_string(payload, "hook_event_name").unwrap_or_default(),
            utils::payload_string(payload, "session_id"),
        )
    }

    /// Agent-state transition for a hook event.
    fn hook_state(&self, event_name: &str, _payload: &serde_json::Value) -> Option<AgentState> {
        match event_name {
            "UserPromptSubmit" | "PostToolUse" => Some(AgentState::Running),
            "PermissionRequest" => Some(AgentState::WaitingApproval),
            "Stop" => Some(AgentState::Completed),
            _ => None,
        }
    }

    /// Push-notification title for a hook event; `None` sends nothing.
    fn hook_notify_title(&self, event_name: &str) -> Option<String> {
        let name = self.display_name();
        match event_name {
            "PermissionRequest" => Some(format!("{name} requires approval")),
            "Stop" => Some(format!("{name} completed")),
            _ => None,
        }
    }

    /// Notification body lookup; runs only when a title matched.
    fn hook_notify_body(
        &self,
        _ctx: &HookContext,
        _agent_session_id: Option<String>,
    ) -> ActorFuture<'static, Option<String>> {
        Box::pin(std::future::ready(None))
    }

    /// Shared hook driver: map the payload through the `hook_*` methods,
    /// apply the state transition, then notify when a title matched.
    fn receive_hook<'a>(&'a self, ctx: HookContext) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if !self.supports_hooks() {
                anyhow::bail!("{} does not support lifecycle hooks", self.slug());
            }
            let (event_name, agent_session_id) = self.hook_identity(&ctx.payload);
            if event_name.is_empty() {
                // Do not log ctx.payload: it can carry user content (telemetry-privacy rule).
                tracing::warn!(
                    agent = self.slug(),
                    "hook payload missing or empty event name; ignoring"
                );
                return Ok(());
            }
            let state = self.hook_state(&event_name, &ctx.payload);
            ctx.apply(self.slug(), &event_name, state, agent_session_id.as_deref())
                .await;
            let Some(title) = self.hook_notify_title(&event_name) else {
                return Ok(());
            };
            let body = self.hook_notify_body(&ctx, agent_session_id);
            ctx.notify(self.display_name(), &event_name, title, body)
                .await
        })
    }

    /// Actors overriding `setup` must opt in, so default installs skip
    /// unsupported actors without swallowing real setup failures.
    fn supports_setup(&self) -> bool {
        false
    }

    fn setup(&self, _workdir: &Path, _force: bool) -> anyhow::Result<Vec<std::path::PathBuf>> {
        anyhow::bail!("{} does not support setup", self.slug())
    }

    /// Actor owns an interactive `zedra setup <slug>` flow; the setup CLI
    /// discovers actors through this flag, never a command-definition list.
    fn supports_setup_cli(&self) -> bool {
        false
    }

    /// Interactive install/remove flow; the actor owns everything, including
    /// CLI presence checks and step output.
    fn setup_cli<'a>(
        &'a self,
        _action: SetupAction,
        _ctx: SetupCliCtx,
    ) -> ActorFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { anyhow::bail!("{} has no `zedra setup` flow", self.slug()) })
    }

    fn hook_test_payload(&self, event_name: &str, workdir: &Path) -> serde_json::Value {
        serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "zedra-test-session",
            "cwd": workdir,
        })
    }
}

static ACTORS: [&dyn AgentActor; 19] = [
    &claude::ClaudeActor,
    &codex::CodexActor,
    &opencode::OpenCodeActor,
    &amp::AmpActor,
    &cline::ClineActor,
    &cursor::CursorActor,
    &copilot::CopilotActor,
    &gemini::GeminiActor,
    &goose::GooseActor,
    &hermes::HermesActor,
    &junie::JunieActor,
    &kilocode::KiloCodeActor,
    &openclaw::OpenClawActor,
    &openhands::OpenHandsActor,
    &pi::PiActor,
    &qoder::QoderActor,
    &qwen::QwenActor,
    &trae::TraeActor,
    &zencoder::ZencoderActor,
];

pub(crate) fn actors() -> &'static [&'static dyn AgentActor] {
    &ACTORS
}

fn sessions_for_actor_blocking(
    actor: &dyn AgentActor,
    workdir: &Path,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, u32), String> {
    let cli = actor.agent_list_cli_summary(workdir);
    let (mut sessions, total) = actor.sessions(&ScanCtx { workdir, cli: &cli }, limit)?;
    let total = u32::try_from(total).unwrap_or(u32::MAX);
    sessions.sort_by(|left, right| right.last_activity_at.cmp(&left.last_activity_at));
    Ok((sessions, total))
}

// ---------------------------------------------------------------------------
// Account snapshot dispatch
// ---------------------------------------------------------------------------

fn account_summary(actor: &dyn AgentActor, workdir: &Path) -> AgentAccountSummary {
    AgentAccountSummary {
        fields: actor.account_fields(workdir),
    }
}

/// Read-only config/memory files for an agent's detail view. Only Hermes
/// exposes a file set today; other agents return none.
pub fn agent_files(slug: &str) -> Result<Vec<AgentFile>, String> {
    actor(slug)
        .map(|actor| actor.config_files())
        .ok_or_else(|| format!("unsupported agent: {slug}"))
}

pub async fn scan_account_plans() -> HashMap<String, Vec<AgentInfoField>> {
    let mut tasks = tokio::task::JoinSet::new();
    for actor in actors() {
        tasks.spawn(async move { (actor.slug(), actor.subscription_plan().await) });
    }
    let mut out = HashMap::new();
    // Drain all tasks: a refutable `while let` would stop at the first `None`.
    while let Some(joined) = tasks.join_next().await {
        if let Ok((slug, Some(fields))) = joined {
            out.insert(slug.to_string(), fields);
        }
    }
    out
}

pub fn apply_cached_account_plans(
    agents: &mut [AgentSummary],
    plans: &HashMap<String, Vec<AgentInfoField>>,
) {
    for agent in agents {
        let Some(remote) = plans.get(&agent.slug) else {
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

pub async fn scan_account_usage() -> HashMap<String, AgentUsageSnapshot> {
    let mut tasks = tokio::task::JoinSet::new();
    for actor in actors() {
        tasks.spawn(async move { (actor.slug(), actor.account_usage().await) });
    }
    let mut out = HashMap::new();
    // Drain all tasks: a refutable `while let` would stop at the first `None`.
    while let Some(joined) = tasks.join_next().await {
        if let Ok((slug, Some(snapshot))) = joined {
            out.insert(slug.to_string(), snapshot);
        }
    }
    out
}

pub fn apply_cached_account_usage(
    agents: &mut Vec<AgentSummary>,
    snapshots: &HashMap<String, AgentUsageSnapshot>,
) {
    for agent in agents {
        if let Some(snap) = snapshots.get(&agent.slug) {
            // Card highlight = the usage extras joined into one line.
            agent.highlight = snap
                .extra
                .iter()
                .map(|field| format!("{}: {}", field.label, field.value))
                .collect::<Vec<_>>()
                .join(" · ");
            agent.usage = Some(snap.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Every resume command must launch the actor's own binary, embed the id
    // shell-quoted; blank ids and unknown slugs never produce a command.
    fn resume_launch_commands_are_host_owned() {
        let mut resumable = 0;
        for actor in actors() {
            let slug = actor.slug();
            let Some(command) = resume_launch_command(slug, "ses-123") else {
                continue;
            };
            resumable += 1;
            let program = command.split_whitespace().next().unwrap_or_default();
            assert!(
                actor.programs().contains(&program),
                "`{slug}` resume `{command}` does not launch one of its programs"
            );
            assert!(
                command.contains("ses-123"),
                "`{slug}` resume `{command}` drops the session id"
            );
            // Unsafe ids must arrive shell-quoted.
            let quoted = resume_launch_command(slug, "a b'c").expect("quoted resume");
            assert!(
                quoted.contains("'a b'\\''c'"),
                "`{slug}` resume `{quoted}` does not shell-quote the id"
            );
            assert_eq!(resume_launch_command(slug, "   "), None, "blank id");
        }
        assert!(resumable > 0, "no actor supports resume");
        assert_eq!(resume_launch_command("nosuchagent", "ses-123"), None);
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
        let (plugin_installed, hooks_installed, error) =
            crate::agent::claude::ClaudeActor::claude_setup_status_from_contents(json);
        assert!(plugin_installed);
        assert!(!hooks_installed);
        assert!(error.is_none());
    }

    #[test]
    fn codex_plugin_status_reads_config() {
        let config = r#"
[marketplaces.zedra]
source_type = "git"
source = "tanlethanh/zedra-plugin"

[plugins."zedra@zedra"]
enabled = true
"#;
        assert!(crate::agent::codex::CodexActor::codex_plugin_enabled(
            config
        ));
        assert!(!crate::agent::codex::CodexActor::codex_plugin_enabled(
            r#"[plugins."zedra@zedra"]
enabled = false
"#
        ));
    }

    #[test]
    fn session_title_defaults_to_unknown_without_provider_title() {
        use crate::agent::utils::session_title;
        assert_eq!(session_title(None).as_deref(), Some("Unknown"));
        assert_eq!(
            session_title(Some("Fix terminal paste".into())).as_deref(),
            Some("Fix terminal paste")
        );
    }
}
