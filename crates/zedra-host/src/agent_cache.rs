use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;
use zedra_rpc::proto::*;

use crate::agent;
use crate::session_registry::ServerSession;

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
    sessions: HashMap<ManagedAgentKind, CachedSessions>,
}

pub struct AgentCache {
    inner: Mutex<CachedAgentData>,
}

impl AgentCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(CachedAgentData::default()),
        })
    }

    pub async fn preload(self: &Arc<Self>, workdir: PathBuf) {
        let cache = Arc::clone(self);
        let result = tokio::task::spawn_blocking(move || cache.refresh_all(&workdir)).await;
        if let Err(error) = result {
            tracing::warn!("agent cache preload task failed: {error}");
        }
    }

    pub async fn installed(&self, refresh: bool) -> AgentInstalledListResult {
        if refresh {
            self.refresh_installed().await;
        } else if self.ensure_installed().await {
            if let Some(result) = self.inner.lock().await.installed.clone() {
                return result;
            }
        }
        AgentInstalledListResult {
            agents: Vec::new(),
            error: Some("agent cache not ready".into()),
        }
    }

    pub async fn agents(
        &self,
        workdir: &Path,
        session: Option<&Arc<ServerSession>>,
        refresh: bool,
    ) -> AgentListResult {
        if refresh {
            self.refresh_agents(workdir).await;
        } else if self.ensure_agents(workdir).await {
            if let Some(mut result) = self.inner.lock().await.agents.clone() {
                agent::merge_live_into_agent_list(&mut result.agents, session).await;
                return result;
            }
        }
        AgentListResult {
            agents: Vec::new(),
            error: Some("agent cache not ready".into()),
        }
    }

    pub async fn sessions(
        &self,
        kind: ManagedAgentKind,
        workdir: &Path,
        session: Option<&Arc<ServerSession>>,
        limit: u32,
        refresh: bool,
    ) -> AgentSessionsResult {
        if refresh {
            self.refresh_sessions(kind, workdir, limit).await;
        } else if self.ensure_sessions(kind, workdir, limit).await {
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
        for kind in [
            ManagedAgentKind::Claude,
            ManagedAgentKind::Codex,
            ManagedAgentKind::OpenCode,
        ] {
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
}

impl CachedAgentData {
    /// Workdir-scoped caches (`agents`, `sessions`) are keyed only by agent
    /// kind, so when the workdir changes they must be dropped or a later
    /// lookup would serve results from the previous workdir.
    fn invalidate_if_workdir_changed(&mut self, workdir: &Path) {
        if self.workdir.as_deref() != Some(workdir) {
            self.agents = None;
            self.sessions.clear();
        }
    }
}
