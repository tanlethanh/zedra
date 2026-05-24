use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;
use zedra_rpc::proto::*;

use crate::agent;
use crate::session_registry::{ServerSession, SessionRegistry};

/// Cached session scan for one agent kind. `limit` is the effective (clamped)
/// limit the scan was run with, so a later request for a larger limit can
/// detect that the cache is too small and re-scan.
struct CachedSessions {
    limit: u32,
    result: AgentSessionsResult,
}

#[derive(Default)]
struct CachedAgentData {
    workdir: Option<PathBuf>,
    installed: Option<AgentInstalledListResult>,
    agents: Option<AgentListResult>,
    cli_versions: HashMap<ManagedAgentKind, AgentCliSummary>,
    account_usage: HashMap<ManagedAgentKind, AgentUsageSnapshot>,
    sessions: HashMap<ManagedAgentKind, CachedSessions>,
}

#[derive(Default)]
struct VersionRefreshCoordinator {
    running: bool,
    rerun: bool,
    pending_sessions: HashMap<String, Arc<ServerSession>>,
}

#[derive(Default)]
struct UsageRefreshCoordinator {
    running: bool,
    rerun: bool,
    pending_sessions: HashMap<String, Arc<ServerSession>>,
}

pub struct AgentCache {
    inner: Mutex<CachedAgentData>,
    version_refresh: Mutex<VersionRefreshCoordinator>,
    usage_refresh: Mutex<UsageRefreshCoordinator>,
    registry: Mutex<Option<Weak<SessionRegistry>>>,
}

impl AgentCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CachedAgentData::default()),
            version_refresh: Mutex::new(VersionRefreshCoordinator::default()),
            usage_refresh: Mutex::new(UsageRefreshCoordinator::default()),
            registry: Mutex::new(None),
        })
    }

    pub async fn set_registry(self: &Arc<Self>, registry: Weak<SessionRegistry>) {
        *self.registry.lock().await = Some(registry);
    }

    /// Trigger a background usage refresh (coordinator-deduplicated).
    /// Called by the periodic refresh task; safe to call concurrently.
    pub async fn refresh_usage(self: &Arc<Self>) {
        self.request_usage_refresh(None).await;
    }

    pub async fn preload(self: &Arc<Self>, workdir: PathBuf) {
        let cache = Arc::clone(self);
        let scan_workdir = workdir.clone();
        let result = tokio::task::spawn_blocking(move || cache.refresh_all(&scan_workdir)).await;
        if let Err(error) = result {
            tracing::warn!("agent cache preload task failed: {error}");
        }
        self.request_version_refresh(None).await;
        self.request_usage_refresh(None).await;
    }

    pub async fn installed(self: &Arc<Self>, refresh: bool) -> AgentInstalledListResult {
        if refresh {
            self.refresh_installed().await;
        } else {
            self.ensure_installed().await;
        }
        if let Some(result) = self.inner.lock().await.installed.clone() {
            return result;
        }
        AgentInstalledListResult {
            agents: Vec::new(),
            error: Some("agent cache not ready".into()),
        }
    }

    pub async fn agents(
        self: &Arc<Self>,
        workdir: &Path,
        session: Option<&Arc<ServerSession>>,
        refresh: bool,
    ) -> AgentListResult {
        if refresh {
            self.refresh_agents(workdir).await;
            self.request_version_refresh(session.cloned()).await;
            self.request_usage_refresh(session.cloned()).await;
        } else {
            self.ensure_agents(workdir).await;
            if self.needs_version_refresh().await {
                self.request_version_refresh(session.cloned()).await;
            }
            if self.needs_usage_refresh().await {
                self.request_usage_refresh(session.cloned()).await;
            }
        }
        self.agent_list_result(session).await
    }

    pub async fn sessions(
        self: &Arc<Self>,
        kind: ManagedAgentKind,
        workdir: &Path,
        session: Option<&Arc<ServerSession>>,
        limit: u32,
        refresh: bool,
    ) -> AgentSessionsResult {
        if refresh {
            self.refresh_sessions(kind, workdir, limit).await;
        } else {
            self.ensure_sessions(kind, workdir, limit).await;
        }
        if let Some(mut result) = self
            .inner
            .lock()
            .await
            .sessions
            .get(&kind)
            .map(|cached| cached.result.clone())
        {
            agent::merge_live_into_sessions(&mut result.sessions, kind, session).await;
            return result;
        }
        AgentSessionsResult {
            sessions: Vec::new(),
            total: 0,
            error: Some("agent cache not ready".into()),
        }
    }

    fn refresh_all(&self, workdir: &Path) {
        let installed = agent::scan_installed_agents();
        let agents = agent::scan_agent_list(workdir);
        let limit = agent::default_agent_session_limit() as u32;
        let mut sessions = HashMap::new();
        for kind in agent::MANAGED_AGENT_KINDS {
            sessions.insert(
                kind,
                CachedSessions {
                    limit,
                    result: agent::scan_agent_sessions(kind, workdir, limit),
                },
            );
        }

        let mut inner = self.inner.blocking_lock();
        inner.workdir = Some(workdir.to_path_buf());
        inner.installed = Some(installed);
        inner.agents = Some(agents);
        inner.sessions = sessions;
    }

    async fn refresh_installed(&self) {
        let installed = tokio::task::spawn_blocking(agent::scan_installed_agents)
            .await
            .unwrap_or_else(|error| AgentInstalledListResult {
                agents: Vec::new(),
                error: Some(error.to_string()),
            });
        self.inner.lock().await.installed = Some(installed);
    }

    async fn refresh_agents(&self, workdir: &Path) {
        let workdir = workdir.to_path_buf();
        let scan_workdir = workdir.clone();
        let agents = tokio::task::spawn_blocking(move || agent::scan_agent_list(&scan_workdir))
            .await
            .unwrap_or_else(|error| AgentListResult {
                agents: Vec::new(),
                error: Some(error.to_string()),
            });
        let mut inner = self.inner.lock().await;
        inner.invalidate_if_workdir_changed(&workdir);
        inner.workdir = Some(workdir);
        inner.agents = Some(agents);
    }

    async fn refresh_sessions(&self, kind: ManagedAgentKind, workdir: &Path, limit: u32) {
        let effective_limit = agent::agent_session_limit(limit) as u32;
        let workdir = workdir.to_path_buf();
        let scan_workdir = workdir.clone();
        let result = tokio::task::spawn_blocking(move || {
            agent::scan_agent_sessions(kind, &scan_workdir, limit)
        })
        .await
        .unwrap_or_else(|error| AgentSessionsResult {
            sessions: Vec::new(),
            total: 0,
            error: Some(error.to_string()),
        });
        let mut inner = self.inner.lock().await;
        inner.invalidate_if_workdir_changed(&workdir);
        inner.workdir = Some(workdir);
        inner.sessions.insert(
            kind,
            CachedSessions {
                limit: effective_limit,
                result,
            },
        );
    }

    async fn ensure_installed(&self) -> bool {
        if self.inner.lock().await.installed.is_some() {
            return true;
        }
        self.refresh_installed().await;
        self.inner.lock().await.installed.is_some()
    }

    async fn ensure_agents(&self, workdir: &Path) -> bool {
        {
            let inner = self.inner.lock().await;
            if inner
                .workdir
                .as_deref()
                .is_some_and(|cached| cached == workdir)
                && inner.agents.is_some()
            {
                return true;
            }
        }
        self.refresh_agents(workdir).await;
        self.inner.lock().await.agents.is_some()
    }

    async fn ensure_sessions(&self, kind: ManagedAgentKind, workdir: &Path, limit: u32) -> bool {
        // A cache hit must cover the requested limit: a scan run at a smaller
        // limit would silently truncate the result for a larger request.
        let needed = agent::agent_session_limit(limit) as u32;
        {
            let inner = self.inner.lock().await;
            if inner
                .workdir
                .as_deref()
                .is_some_and(|cached| cached == workdir)
            {
                if let Some(cached) = inner.sessions.get(&kind) {
                    if cached.limit >= needed {
                        return true;
                    }
                }
            }
        }
        self.refresh_sessions(kind, workdir, limit).await;
        self.inner.lock().await.sessions.contains_key(&kind)
    }

    async fn needs_version_refresh(&self) -> bool {
        let inner = self.inner.lock().await;
        if inner.cli_versions.is_empty() {
            return true;
        }
        inner.agents.as_ref().is_some_and(|list| {
            list.agents
                .iter()
                .any(|agent| agent.cli.available && agent.cli.version.is_none())
        })
    }

    async fn needs_usage_refresh(&self) -> bool {
        self.inner.lock().await.account_usage.is_empty()
    }

    async fn agent_list_result(
        self: &Arc<Self>,
        session: Option<&Arc<ServerSession>>,
    ) -> AgentListResult {
        let (result, versions, usage) = {
            let inner = self.inner.lock().await;
            (
                inner.agents.clone(),
                inner.cli_versions.clone(),
                inner.account_usage.clone(),
            )
        };
        let Some(mut result) = result else {
            return AgentListResult {
                agents: Vec::new(),
                error: Some("agent cache not ready".into()),
            };
        };
        agent::apply_cached_cli_versions(&mut result.agents, &versions);
        agent::apply_cached_account_usage(&mut result.agents, &usage);
        agent::merge_live_into_agent_list(&mut result.agents, session).await;
        result
    }

    async fn cached_agent_summary(
        self: &Arc<Self>,
        kind: ManagedAgentKind,
        session: Option<&Arc<ServerSession>>,
    ) -> Option<AgentSummary> {
        let (mut agents, versions, usage) = {
            let inner = self.inner.lock().await;
            let agents = inner.agents.as_ref()?.agents.clone();
            (
                agents,
                inner.cli_versions.clone(),
                inner.account_usage.clone(),
            )
        };
        agent::apply_cached_cli_versions(&mut agents, &versions);
        agent::apply_cached_account_usage(&mut agents, &usage);
        let summary = agents.into_iter().find(|agent| agent.kind == kind)?;
        let mut agents = [summary];
        agent::merge_live_into_agent_list(&mut agents, session).await;
        Some(agents.into_iter().next()?)
    }

    async fn request_version_refresh(self: &Arc<Self>, session: Option<Arc<ServerSession>>) {
        let start = {
            let mut coord = self.version_refresh.lock().await;
            if let Some(session) = session {
                coord.pending_sessions.insert(session.id.clone(), session);
            }
            if coord.running {
                coord.rerun = true;
                false
            } else {
                coord.running = true;
                true
            }
        };
        if !start {
            return;
        }

        let cache = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                cache.run_version_refresh().await;
                let rerun = {
                    let mut coord = cache.version_refresh.lock().await;
                    let rerun = coord.rerun;
                    coord.rerun = false;
                    if !rerun {
                        coord.running = false;
                    }
                    rerun
                };
                if !rerun {
                    break;
                }
            }
        });
    }

    async fn run_version_refresh(self: &Arc<Self>) {
        let versions = tokio::task::spawn_blocking(agent::scan_managed_agent_cli_versions)
            .await
            .unwrap_or_default();

        {
            let mut inner = self.inner.lock().await;
            inner.cli_versions = versions.clone();
            if let Some(agents) = inner.agents.as_mut() {
                agent::apply_cached_cli_versions(&mut agents.agents, &versions);
            }
        }

        tracing::debug!(
            "managed agent cli versions refreshed: {}",
            versions
                .iter()
                .map(|(kind, cli)| format!("{kind:?}={}", cli.version.as_deref().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let sessions = self.collect_notify_sessions().await;
        for session in sessions {
            self.push_agent_info_changed(&session).await;
        }
    }

    async fn request_usage_refresh(self: &Arc<Self>, session: Option<Arc<ServerSession>>) {
        let start = {
            let mut coord = self.usage_refresh.lock().await;
            if let Some(session) = session {
                coord.pending_sessions.insert(session.id.clone(), session);
            }
            if coord.running {
                coord.rerun = true;
                false
            } else {
                coord.running = true;
                true
            }
        };
        if !start {
            return;
        }
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                cache.run_usage_refresh().await;
                let rerun = {
                    let mut coord = cache.usage_refresh.lock().await;
                    let rerun = coord.rerun;
                    coord.rerun = false;
                    if !rerun {
                        coord.running = false;
                    }
                    rerun
                };
                if !rerun {
                    break;
                }
            }
        });
    }

    async fn run_usage_refresh(self: &Arc<Self>) {
        let snapshots = agent::scan_account_usage().await;
        {
            let mut inner = self.inner.lock().await;
            inner.account_usage = snapshots.clone();
            if let Some(agents) = inner.agents.as_mut() {
                agent::apply_cached_account_usage(&mut agents.agents, &snapshots);
            }
        }
        tracing::debug!(
            "agent account usage refreshed: {}",
            snapshots
                .iter()
                .map(|(kind, snap)| format!(
                    "{kind:?} 5h={:.0}% 7d={:.0}%",
                    snap.rate_limit_five_hour_used_percent.unwrap_or(0.0),
                    snap.rate_limit_seven_day_used_percent.unwrap_or(0.0),
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let sessions = self.collect_usage_notify_sessions().await;
        for session in sessions {
            self.push_agent_info_changed(&session).await;
        }
    }

    async fn collect_usage_notify_sessions(&self) -> Vec<Arc<ServerSession>> {
        let pending = {
            let mut coord = self.usage_refresh.lock().await;
            coord.pending_sessions.drain().collect::<Vec<_>>()
        };
        let mut by_id: HashMap<String, Arc<ServerSession>> =
            pending.into_iter().map(|(id, s)| (id, s)).collect();
        if let Some(registry) = self.registry.lock().await.as_ref().and_then(Weak::upgrade) {
            for session in registry.sessions_with_event_subscribers().await {
                by_id.insert(session.id.clone(), session);
            }
        }
        by_id.into_values().collect()
    }

    async fn collect_notify_sessions(&self) -> Vec<Arc<ServerSession>> {
        let pending = {
            let mut coord = self.version_refresh.lock().await;
            coord.pending_sessions.drain().collect::<Vec<_>>()
        };
        let mut by_id: HashMap<String, Arc<ServerSession>> =
            pending.into_iter().map(|(id, s)| (id, s)).collect();

        if let Some(registry) = self.registry.lock().await.as_ref().and_then(Weak::upgrade) {
            for session in registry.sessions_with_event_subscribers().await {
                by_id.insert(session.id.clone(), session);
            }
        }

        by_id.into_values().collect()
    }

    async fn push_agent_info_changed(self: &Arc<Self>, session: &Arc<ServerSession>) {
        for kind in agent::MANAGED_AGENT_KINDS {
            let Some(info) = self.cached_agent_summary(kind, Some(session)).await else {
                continue;
            };
            if session
                .push_event(HostEvent::AgentInfoChanged { info })
                .await
            {
                continue;
            }
            tracing::debug!(
                session_id = %session.id,
                ?kind,
                "AgentInfoChanged dropped (no subscriber or channel full)"
            );
        }
    }
}

impl CachedAgentData {
    /// Workdir-scoped caches (`agents`, `sessions`) are keyed only by agent
    /// kind, so when the workdir changes they must be dropped or a later
    /// lookup would serve results from the previous workdir.
    fn invalidate_if_workdir_changed(&mut self, workdir: &Path) {
        if self.workdir.as_deref() != Some(workdir) {
            self.agents = None;
            self.sessions.clear();
            self.cli_versions.clear();
        }
    }
}
