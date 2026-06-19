use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use irpc::{
    Channels, RpcMessage, Service, WithChannels,
    channel::{none::NoReceiver, oneshot},
};
use zedra_rpc::proto::*;

use crate::{
    register_active_connection, signer::ClientSigner, terminal::RemoteTerminal,
    unregister_active_connection,
};

#[derive(Clone)]
pub struct SessionHandle(Arc<SessionHandleInner>);

struct SessionHandleInner {
    sid: Mutex<Option<String>>,
    endpoint_addr: Mutex<Option<iroh::EndpointAddr>>,
    endpoint_id: Mutex<Option<iroh::PublicKey>>,
    signer: Mutex<Option<Arc<dyn ClientSigner>>>,
    session_token: Mutex<Option<[u8; 32]>>,
    pending_ticket: Mutex<Option<zedra_rpc::ZedraPairingTicket>>,
    rpc_client: Mutex<Option<irpc::Client<ZedraProto>>>,
    active_connection: Mutex<Option<iroh::endpoint::Connection>>,
    terminals: Mutex<Vec<RemoteTerminal>>,
    user_disconnect: AtomicBool,
    observer_rpc_supported: AtomicBool,
    docs_tree_rpc_supported: AtomicBool,
    fs_search_rpc_supported: AtomicBool,
    set_app_state_rpc_supported: AtomicBool,
}

impl Drop for SessionHandleInner {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_connection.lock() {
            if let Some(conn) = active.take() {
                unregister_active_connection(&conn);
                conn.close(0u32.into(), b"client handle dropped");
            }
        }
    }
}

impl Default for SessionHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionHandle {
    pub fn new() -> Self {
        Self(Arc::new(SessionHandleInner {
            sid: Mutex::new(None),
            endpoint_addr: Mutex::new(None),
            endpoint_id: Mutex::new(None),
            signer: Mutex::new(None),
            session_token: Mutex::new(None),
            pending_ticket: Mutex::new(None),
            rpc_client: Mutex::new(None),
            active_connection: Mutex::new(None),
            terminals: Mutex::new(Vec::new()),
            user_disconnect: AtomicBool::new(false),
            observer_rpc_supported: AtomicBool::new(true),
            docs_tree_rpc_supported: AtomicBool::new(true),
            fs_search_rpc_supported: AtomicBool::new(true),
            set_app_state_rpc_supported: AtomicBool::new(true),
        }))
    }

    // ─── Credentials ─────────────────────────────────────────────────────────

    pub fn set_signer(&self, signer: Arc<dyn ClientSigner>) {
        *self.0.signer.lock().unwrap() = Some(signer);
    }

    pub fn signer(&self) -> Option<Arc<dyn ClientSigner>> {
        self.0.signer.lock().ok()?.clone()
    }

    pub fn set_endpoint_id(&self, id: iroh::PublicKey) {
        *self.0.endpoint_id.lock().unwrap() = Some(id);
    }

    pub fn endpoint_id(&self) -> Option<iroh::PublicKey> {
        self.0.endpoint_id.lock().ok()?.clone()
    }

    pub fn set_endpoint_addr(&self, addr: iroh::EndpointAddr) {
        *self.0.endpoint_addr.lock().unwrap() = Some(addr);
    }

    pub fn endpoint_addr(&self) -> Option<iroh::EndpointAddr> {
        self.0.endpoint_addr.lock().ok()?.clone()
    }

    pub fn set_pending_ticket(&self, ticket: zedra_rpc::ZedraPairingTicket) {
        *self.0.pending_ticket.lock().unwrap() = Some(ticket);
    }

    pub fn take_pending_ticket(&self) -> Option<zedra_rpc::ZedraPairingTicket> {
        self.0.pending_ticket.lock().ok()?.take()
    }

    pub fn set_session_token(&self, token: Option<[u8; 32]>) {
        *self.0.session_token.lock().unwrap() = token;
    }

    pub fn session_token(&self) -> Option<[u8; 32]> {
        self.0.session_token.lock().ok().and_then(|g| *g)
    }

    pub fn session_id(&self) -> Option<String> {
        self.0.sid.lock().ok()?.clone()
    }

    pub fn set_session_id(&self, session_id: Option<String>) {
        *self.0.sid.lock().unwrap() = session_id;
    }

    pub fn set_rpc_client(&self, client: irpc::Client<ZedraProto>) {
        *self.0.rpc_client.lock().unwrap() = Some(client);
    }

    pub fn set_active_connection(&self, conn: iroh::endpoint::Connection) {
        if let Ok(mut active) = self.0.active_connection.lock() {
            if let Some(previous) = active.take() {
                unregister_active_connection(&previous);
                previous.close(0u32.into(), b"client reconnect");
            }
            register_active_connection(&conn);
            *active = Some(conn);
        }
    }

    pub fn clear_active_connection(&self) {
        if let Ok(mut active) = self.0.active_connection.lock() {
            if let Some(conn) = active.take() {
                unregister_active_connection(&conn);
            }
        }
    }

    pub fn close_active_connection(&self, reason: &'static [u8]) {
        if let Ok(mut active) = self.0.active_connection.lock() {
            if let Some(conn) = active.take() {
                unregister_active_connection(&conn);
                conn.close(0u32.into(), reason);
            }
        }
    }

    pub fn clear_rpc_client(&self) {
        *self.0.rpc_client.lock().unwrap() = None;
    }

    pub fn has_client(&self) -> bool {
        self.0.rpc_client.lock().ok().map_or(false, |g| g.is_some())
    }

    fn client(&self) -> Result<irpc::Client<ZedraProto>> {
        self.0
            .rpc_client
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| anyhow::anyhow!("not connected"))
    }

    /// Single entry point for every unary RPC. Funneling all calls through here
    /// keeps error mapping (`map_rpc_error`) in exactly one place instead of a
    /// `.map_err(...)` at each call site. The bounds mirror `irpc::Client::rpc`.
    async fn call<Req, Res>(&self, msg: Req) -> Result<Res>
    where
        ZedraProto: From<Req>,
        <ZedraProto as Service>::Message: From<WithChannels<Req, ZedraProto>>,
        Req: Channels<ZedraProto, Tx = oneshot::Sender<Res>, Rx = NoReceiver>,
        Res: RpcMessage,
    {
        self.client()?.rpc(msg).await.map_err(map_rpc_error)
    }

    pub fn user_disconnect(&self) -> bool {
        self.0.user_disconnect.load(Ordering::Acquire)
    }

    pub fn set_user_disconnect(&self, v: bool) {
        self.0.user_disconnect.store(v, Ordering::Release);
    }

    pub fn clear_session(&self) {
        self.set_user_disconnect(true);
        // Send CONNECTION_CLOSE before dropping RPC handles so the host can
        // release this client's active slot without waiting for QUIC idle expiry.
        self.close_active_connection(b"client disconnect");
        self.clear_rpc_client();
        *self.0.terminals.lock().unwrap() = Vec::new();
        tracing::info!("SessionHandle: session cleared");
    }

    pub fn terminal_count(&self) -> usize {
        self.0.terminals.lock().map(|t| t.len()).unwrap_or(0)
    }

    pub fn terminal_ids(&self) -> Vec<String> {
        self.0
            .terminals
            .lock()
            .map(|t| t.iter().map(|t| t.id()).collect())
            .unwrap_or_default()
    }

    pub fn terminal(&self, id: &str) -> Option<RemoteTerminal> {
        self.0
            .terminals
            .lock()
            .ok()
            .and_then(|t| t.iter().find(|t| t.id() == id).cloned())
    }

    pub fn terminals(&self) -> Vec<RemoteTerminal> {
        self.0
            .terminals
            .lock()
            .map(|t| t.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Sync the terminal list with the given list of terminals.
    pub fn set_terminals(&self, new_terminals: Vec<RemoteTerminal>) {
        if let Ok(mut terminals_slot) = self.0.terminals.lock() {
            *terminals_slot = new_terminals;
        }
    }

    pub fn detach_terminals(&self) {
        for terminal in self.terminals() {
            terminal.detach_remote();
        }
    }

    pub fn add_terminal(&self, terminal: RemoteTerminal) {
        if let Ok(mut t) = self.0.terminals.lock() {
            let id = terminal.id();
            t.retain(|existing| existing.id() != id);
            t.push(terminal);
        }
    }

    pub fn remove_terminal(&self, id: &str) {
        if let Ok(mut t) = self.0.terminals.lock() {
            t.retain(|t| t.id() != id);
        }
    }

    pub fn reorder_terminals(&self, ordered_ids: &[String]) -> bool {
        let Ok(mut terminals) = self.0.terminals.lock() else {
            return false;
        };
        if ordered_ids.len() != terminals.len() {
            return false;
        }

        let mut by_id = terminals
            .iter()
            .map(|terminal| (terminal.id(), terminal.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let mut reordered = Vec::with_capacity(ordered_ids.len());
        for id in ordered_ids {
            let Some(terminal) = by_id.remove(id) else {
                return false;
            };
            reordered.push(terminal);
        }

        *terminals = reordered;
        true
    }

    pub async fn fs_list(&self, path: &str) -> Result<(Vec<FsEntry>, u32, bool)> {
        self.fs_list_page(path, 0, FS_LIST_DEFAULT_LIMIT).await
    }

    pub async fn fs_list_page(
        &self,
        path: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FsEntry>, u32, bool)> {
        let result: FsListResult = self
            .call(FsListReq {
                path: path.to_string(),
                offset,
                limit,
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok((result.entries, result.total, result.has_more))
    }

    pub async fn fs_search(&self, path: &str, query: &str, limit: u32) -> Result<FsSearchResult> {
        if !self.0.fs_search_rpc_supported.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!("file search RPC unsupported by host"));
        }
        let result: FsSearchResult = match self
            .call(FsSearchReq {
                path: path.to_string(),
                query: query.to_string(),
                limit,
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                if self.downgrade_fs_search_rpc(&error.to_string()) {
                    return Err(anyhow::anyhow!("file search RPC unsupported by host"));
                }
                return Err(error);
            }
        };
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result)
    }

    pub async fn fs_read(&self, path: &str) -> Result<FsReadResult> {
        Ok(self
            .call(FsReadReq {
                path: path.to_string(),
            })
            .await?)
    }

    /// Write `content` to `path` with optimistic concurrency. `expected` is the
    /// `(mtime, size)` version the client last observed for this file (`None`
    /// for a new file or a deliberate blind write). When the disk version no
    /// longer matches, the returned [`FsWriteResult`] has `conflict: true` and
    /// `current_content`; pass `force` to overwrite regardless. On success the
    /// caller must adopt the returned `(mtime, size)` as its new token.
    pub async fn fs_write(
        &self,
        path: &str,
        content: &str,
        expected: Option<(Option<u64>, u64)>,
        force: bool,
    ) -> Result<FsWriteResult> {
        let (expected_mtime, expected_size) = match expected {
            Some((mtime, size)) => (mtime, Some(size)),
            None => (None, None),
        };
        Ok(self
            .call(FsWriteReq {
                path: path.to_string(),
                content: content.to_string(),
                expected_mtime,
                expected_size,
                force,
            })
            .await?)
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        let result = self
            .call(FsStatReq {
                path: path.to_string(),
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result)
    }

    pub async fn fs_docs_tree(
        &self,
        path: &str,
        offset: u32,
        limit: u32,
        rebuild: bool,
        snapshot_id: Option<String>,
    ) -> Result<FsDocsTreeResult> {
        if !self.0.docs_tree_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsDocsTreeResult::unsupported());
        }
        match self
            .call(FsDocsTreeReq {
                path: path.to_string(),
                offset,
                limit,
                rebuild,
                snapshot_id,
            })
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => {
                if self.downgrade_docs_tree_rpc(&error.to_string()) {
                    return Ok(FsDocsTreeResult::unsupported());
                }
                Err(error)
            }
        }
    }

    pub async fn fs_watch(&self, path: &str) -> Result<FsWatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsWatchResult::Unsupported);
        }
        match self
            .call(FsWatchReq {
                path: path.to_string(),
            })
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => {
                if self.downgrade_observer_rpc(&e.to_string()) {
                    return Ok(FsWatchResult::Unsupported);
                }
                Err(e)
            }
        }
    }

    pub async fn fs_unwatch(&self, path: &str) -> Result<FsUnwatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsUnwatchResult::Unsupported);
        }
        match self
            .call(FsUnwatchReq {
                path: path.to_string(),
            })
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => {
                if self.downgrade_observer_rpc(&e.to_string()) {
                    return Ok(FsUnwatchResult::Unsupported);
                }
                Err(e)
            }
        }
    }

    fn downgrade_observer_rpc(&self, err: &str) -> bool {
        let msg = err.to_lowercase();
        let incompatible = msg.contains("unknown variant")
            || msg.contains("deserialize")
            || msg.contains("decode")
            || msg.contains("invalid type");
        if incompatible {
            self.0
                .observer_rpc_supported
                .store(false, Ordering::Release);
            tracing::warn!("observer RPC unsupported, disabling: {}", err);
        }
        incompatible
    }

    fn downgrade_docs_tree_rpc(&self, err: &str) -> bool {
        let msg = err.to_lowercase();
        let incompatible = msg.contains("unknown variant")
            || msg.contains("deserialize")
            || msg.contains("decode")
            || msg.contains("invalid type");
        if incompatible {
            self.0
                .docs_tree_rpc_supported
                .store(false, Ordering::Release);
            tracing::warn!("docs tree RPC unsupported, disabling: {}", err);
        }
        incompatible
    }

    fn downgrade_fs_search_rpc(&self, err: &str) -> bool {
        let msg = err.to_lowercase();
        let incompatible = msg.contains("unknown variant")
            || msg.contains("deserialize")
            || msg.contains("decode")
            || msg.contains("invalid type");
        if incompatible {
            self.0
                .fs_search_rpc_supported
                .store(false, Ordering::Release);
            tracing::warn!("file search RPC unsupported, disabling: {}", err);
        }
        incompatible
    }

    fn downgrade_set_app_state_rpc(&self, err: &str) -> bool {
        let msg = err.to_lowercase();
        let incompatible = msg.contains("unknown variant")
            || msg.contains("deserialize")
            || msg.contains("decode")
            || msg.contains("invalid type");
        if incompatible {
            self.0
                .set_app_state_rpc_supported
                .store(false, Ordering::Release);
            tracing::warn!("SetAppState RPC unsupported, disabling: {}", err);
        }
        incompatible
    }

    // ─── RPC: git ────────────────────────────────────────────────────────────

    pub async fn git_status(&self) -> Result<GitStatusResult> {
        let result: GitStatusResult = self.call(GitStatusReq {}).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result)
    }

    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let result: GitDiffResult = self
            .call(GitDiffReq {
                path: path.map(|s| s.to_string()),
                staged,
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.diff)
    }

    pub async fn git_log(&self, limit: Option<usize>) -> Result<Vec<GitLogEntry>> {
        let result: GitLogResult = self.call(GitLogReq { limit }).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.entries)
    }

    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let result: GitBranchesResult = self.call(GitBranchesReq {}).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.branches)
    }

    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let result: GitCheckoutResult = self
            .call(GitCheckoutReq {
                branch: branch.to_string(),
            })
            .await?;
        git_checkout_result(result, branch)
    }

    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let result: GitCommitResult = self
            .call(GitCommitReq {
                message: message.to_string(),
                paths: paths.to_vec(),
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.hash)
    }

    pub async fn git_stage(&self, paths: &[String]) -> Result<()> {
        let result: GitStageResult = self
            .call(GitStageReq {
                paths: paths.to_vec(),
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(())
    }

    pub async fn git_unstage(&self, paths: &[String]) -> Result<()> {
        let result: GitUnstageResult = self
            .call(GitUnstageReq {
                paths: paths.to_vec(),
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(())
    }

    // ─── RPC: terminals ──────────────────────────────────────────────────────

    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        self.terminal_create_with_cmd(cols, rows, None, None).await
    }

    pub async fn terminal_create_with_cmd(
        &self,
        cols: u16,
        rows: u16,
        launch_cmd: Option<String>,
        color_scheme: Option<TerminalColorScheme>,
    ) -> Result<String> {
        // Reuse one client for the create RPC and the attach; re-fetching after
        // the terminal exists could race with handle clearing and fail with
        // "not connected" while the remote terminal is already live.
        let client = self.client()?;
        let result: TermCreateResult = client
            .rpc(TermCreateReqV2 {
                cols,
                rows,
                launch_cmd,
                color_scheme,
            })
            .await
            .map_err(map_rpc_error)?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }

        let terminal = RemoteTerminal::new(result.id.clone());
        if terminal.attach_remote(&client).await.is_ok() {
            self.add_terminal(terminal);
            tracing::info!("Terminal created: {}", result.id);
            Ok(result.id)
        } else {
            Err(anyhow::anyhow!("Failed to attach terminal"))
        }
    }

    pub async fn agent_installed_list(&self, refresh: bool) -> Result<Vec<InstalledAgentEntry>> {
        let result: AgentInstalledListResult = self.call(AgentInstalledListReq { refresh }).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.agents)
    }

    pub async fn agent_list(&self, refresh: bool) -> Result<Vec<AgentSummary>> {
        let result: AgentListResult = self.call(AgentListReq { refresh }).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.agents)
    }

    /// Read-only config/memory files for an agent's detail view (Hermes today).
    pub async fn agent_files(&self, kind: AgentKind) -> Result<Vec<AgentFile>> {
        let result: AgentFilesResult = self.call(AgentFilesReq { kind }).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.files)
    }

    /// Notify the host of the app's foreground/background state.
    /// Fire-and-forget: errors are logged but not surfaced.
    pub async fn notify_app_state(&self, in_foreground: bool) {
        if !self.0.set_app_state_rpc_supported.load(Ordering::Acquire) {
            return;
        }
        let result: Result<SetAppStateResult> = self.call(SetAppStateReq { in_foreground }).await;
        if let Err(e) = result {
            // Stop re-issuing the RPC once the host proves it cannot handle it.
            if !self.downgrade_set_app_state_rpc(&e.to_string()) {
                tracing::debug!(error = %e, "notify_app_state failed");
            }
        }
    }

    pub async fn set_client_delta_info(
        &self,
        delta_url: String,
        stack_id: uuid::Uuid,
        client_node_id: uuid::Uuid,
        host_node_id: uuid::Uuid,
    ) -> Result<()> {
        let result: Result<SetClientDeltaInfoResult> = self
            .call(SetClientDeltaInfoReq {
                delta_url,
                stack_id,
                client_node_id,
                host_node_id,
            })
            .await;
        if let Err(e) = result {
            tracing::debug!(error = %e, "set_client_delta_info failed");
            return Err(e);
        }
        Ok(())
    }

    pub async fn clear_client_delta_info(&self) -> Result<()> {
        let result: Result<ClearClientDeltaInfoResult> =
            self.call(ClearClientDeltaInfoReq {}).await;
        if let Err(e) = result {
            tracing::debug!(error = %e, "clear_client_delta_info failed");
            return Err(e);
        }
        Ok(())
    }

    pub async fn agent_sessions(
        &self,
        kind: AgentKind,
        refresh: bool,
        limit: u32,
    ) -> Result<Vec<AgentSessionSummary>> {
        let result: AgentSessionsResult = self
            .call(AgentSessionsReq {
                kind,
                refresh,
                limit,
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.sessions)
    }

    pub async fn agent_resume_session(
        &self,
        kind: AgentKind,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<String> {
        // Reuse one client for the resume RPC and the attach (see
        // terminal_create_with_cmd for the race this avoids).
        let client = self.client()?;
        let result: AgentResumeResult = client
            .rpc(AgentResumeReq {
                kind,
                session_id,
                cols,
                rows,
            })
            .await
            .map_err(map_rpc_error)?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }

        let terminal = RemoteTerminal::new(result.terminal_id.clone());
        if terminal.attach_remote(&client).await.is_ok() {
            self.add_terminal(terminal);
            tracing::info!("Agent session resumed in terminal: {}", result.terminal_id);
            Ok(result.terminal_id)
        } else {
            Err(anyhow::anyhow!("Failed to attach resumed agent terminal"))
        }
    }

    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let _: TermResizeResult = self
            .call(TermResizeReq {
                id: id.to_string(),
                cols,
                rows,
            })
            .await?;
        Ok(())
    }

    pub async fn terminal_close(&self, id: &str) -> Result<()> {
        let _: TermCloseResult = self.call(TermCloseReq { id: id.to_string() }).await?;
        self.remove_terminal(id);
        Ok(())
    }

    pub async fn terminal_list(&self) -> Result<Vec<String>> {
        let result: TermListResult = self.call(TermListReq {}).await?;
        Ok(result.terminals.into_iter().map(|e| e.id).collect())
    }

    pub async fn terminal_reorder(&self, ordered_ids: Vec<String>) -> Result<()> {
        let result: TermReorderResult = self
            .call(TermReorderReq {
                ordered_ids: ordered_ids.clone(),
            })
            .await?;
        if let Some(error) = result.error {
            return Err(anyhow::anyhow!(error));
        }
        if !result.ok {
            return Err(anyhow::anyhow!("terminal reorder failed"));
        }
        if !self.reorder_terminals(&ordered_ids) {
            tracing::warn!("host accepted terminal reorder that did not match local terminals");
        }
        Ok(())
    }
}

fn git_checkout_result(result: GitCheckoutResult, branch: &str) -> Result<()> {
    if result.ok {
        Ok(())
    } else {
        Err(anyhow::anyhow!("git checkout failed for branch '{branch}'"))
    }
}

/// User-facing message for an irpc error. irpc's `Display` is shallow — variants
/// like `OneshotRecv` render as the bare label `"Oneshot recv error"` and keep
/// the real cause in a `source` field — so walk to the deepest link in the
/// `source()` chain and return that. The root cause (e.g. a postcard decode
/// message on a response we couldn't read) is the actionable part; the outer
/// transport labels are noise. Shared by every RPC surface, including the
/// connection handshake in `connect.rs`.
pub(crate) fn rpc_error_message(err: &irpc::Error) -> String {
    error_root_cause(err)
}

/// The deepest non-empty message in an error's `source()` chain, falling back
/// to the error's own `Display` when no deeper cause carries text.
fn error_root_cause(err: &dyn std::error::Error) -> String {
    let mut deepest = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !text.is_empty() {
            deepest = text;
        }
        source = cause.source();
    }
    deepest
}

pub(crate) fn map_rpc_error(err: irpc::Error) -> anyhow::Error {
    anyhow::anyhow!(rpc_error_message(&err))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal `std::error::Error` whose `Display` and `source()` we control,
    /// for testing the chain flattener against known error shapes.
    #[derive(Debug)]
    struct ChainErr {
        msg: String,
        source: Option<Box<ChainErr>>,
    }

    impl ChainErr {
        fn new(msg: &str) -> Self {
            Self {
                msg: msg.to_string(),
                source: None,
            }
        }
        /// Build a chain from outermost to innermost message.
        fn chain(messages: &[&str]) -> Self {
            let mut iter = messages.iter().rev();
            let mut current = ChainErr::new(iter.next().expect("non-empty chain"));
            for msg in iter {
                current = ChainErr {
                    msg: msg.to_string(),
                    source: Some(Box::new(current)),
                };
            }
            current
        }
    }

    impl std::fmt::Display for ChainErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.msg)
        }
    }

    impl std::error::Error for ChainErr {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_deref()
                .map(|e| e as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn error_root_cause_single_error_returns_itself() {
        let err = ChainErr::new("not connected");
        assert_eq!(error_root_cause(&err), "not connected");
    }

    #[test]
    fn error_root_cause_strips_shallow_transport_label() {
        // The exact shape that motivated this: irpc's shallow "Oneshot recv
        // error" label wrapping the real cause. We want just the cause.
        let err = ChainErr::chain(&[
            "Oneshot recv error",
            "Io error",
            "Serde Deserialization Error",
        ]);
        assert_eq!(error_root_cause(&err), "Serde Deserialization Error");
    }

    #[test]
    fn error_root_cause_returns_deepest_decode_message() {
        let err = ChainErr::chain(&[
            "Recv error",
            "Io error",
            "unknown variant 9999, expected one of `Claude`, `Codex`",
        ]);
        assert_eq!(
            error_root_cause(&err),
            "unknown variant 9999, expected one of `Claude`, `Codex`"
        );
    }

    #[test]
    fn error_root_cause_skips_empty_deepest_link() {
        // A trailing empty link must not blank out the message; fall back to the
        // deepest link that actually carries text.
        let err = ChainErr::chain(&["Request error", "broken pipe", ""]);
        assert_eq!(error_root_cause(&err), "broken pipe");
    }

    #[test]
    fn add_terminal_replaces_existing_id() {
        let handle = SessionHandle::new();
        let old_terminal = RemoteTerminal::new("term-1".to_string());
        old_terminal.update_seq(99);
        let new_terminal = RemoteTerminal::new("term-1".to_string());
        new_terminal.update_seq(1);

        handle.add_terminal(old_terminal);
        handle.add_terminal(new_terminal);

        assert_eq!(handle.terminal_ids(), vec!["term-1"]);
        assert_eq!(handle.terminal("term-1").unwrap().last_seq(), 1);
    }

    #[test]
    fn set_terminals_syncs_to_remote_active_list() {
        let handle = SessionHandle::new();
        handle.add_terminal(RemoteTerminal::new("stale-local".to_string()));

        handle.set_terminals(vec![
            RemoteTerminal::new("term-2".to_string()),
            RemoteTerminal::new("term-3".to_string()),
        ]);

        assert_eq!(handle.terminal_ids(), vec!["term-2", "term-3"]);
        assert!(handle.terminal("stale-local").is_none());
    }

    #[test]
    fn set_terminals_empty_remote_list_clears_local_terminals() {
        let handle = SessionHandle::new();
        handle.add_terminal(RemoteTerminal::new("stale-local".to_string()));

        handle.set_terminals(Vec::new());

        assert!(handle.terminal_ids().is_empty());
        assert_eq!(handle.terminal_count(), 0);
    }

    #[test]
    fn reorder_terminals_applies_exact_local_terminal_order() {
        let handle = SessionHandle::new();
        handle.add_terminal(RemoteTerminal::new("term-a".to_string()));
        handle.add_terminal(RemoteTerminal::new("term-b".to_string()));
        handle.add_terminal(RemoteTerminal::new("term-c".to_string()));

        assert!(handle.reorder_terminals(&[
            "term-c".to_string(),
            "term-a".to_string(),
            "term-b".to_string()
        ]));

        assert_eq!(handle.terminal_ids(), vec!["term-c", "term-a", "term-b"]);
    }

    #[test]
    fn reorder_terminals_rejects_unknown_or_partial_orders() {
        let handle = SessionHandle::new();
        handle.add_terminal(RemoteTerminal::new("term-a".to_string()));
        handle.add_terminal(RemoteTerminal::new("term-b".to_string()));

        assert!(!handle.reorder_terminals(&["term-a".to_string()]));
        assert!(!handle.reorder_terminals(&["term-a".to_string(), "missing".to_string()]));
        assert_eq!(handle.terminal_ids(), vec!["term-a", "term-b"]);
    }

    #[test]
    fn git_checkout_result_rejects_failed_host_result() {
        assert!(git_checkout_result(GitCheckoutResult { ok: true }, "main").is_ok());

        let err = git_checkout_result(GitCheckoutResult { ok: false }, "missing").unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_last_handle_closes_active_connection() {
        let client = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let server = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let server_addr = server.addr();
        let server_task = tokio::spawn(async move {
            let incoming = server.accept().await.expect("incoming connection");
            let conn = incoming.await.expect("accepted connection");
            conn.closed().await
        });

        let conn = client.connect(server_addr, ZEDRA_ALPN).await.unwrap();
        let handle = SessionHandle::new();
        handle.set_active_connection(conn);
        drop(handle);

        let close_reason = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .expect("timed out waiting for remote close")
            .unwrap();
        assert!(
            matches!(
                close_reason,
                iroh::endpoint::ConnectionError::ApplicationClosed(_)
            ),
            "expected application close, got {close_reason:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_close_all_closes_active_connection_without_handle_borrow() {
        let client = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let server = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .alpns(vec![ZEDRA_ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let server_addr = server.addr();
        let server_task = tokio::spawn(async move {
            let incoming = server.accept().await.expect("incoming connection");
            let conn = incoming.await.expect("accepted connection");
            conn.closed().await
        });

        let conn = client.connect(server_addr, ZEDRA_ALPN).await.unwrap();
        let handle = SessionHandle::new();
        handle.set_active_connection(conn);
        crate::close_all_active_connections_for_lifecycle(b"client lifecycle close");

        let close_reason = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .expect("timed out waiting for remote close")
            .unwrap();
        assert!(
            matches!(
                close_reason,
                iroh::endpoint::ConnectionError::ApplicationClosed(_)
            ),
            "expected application close, got {close_reason:?}",
        );
    }
}
