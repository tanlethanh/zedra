use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use zedra_rpc::proto::*;

use crate::{signer::ClientSigner, terminal::RemoteTerminal};

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
    terminals: Mutex<Vec<RemoteTerminal>>,
    user_disconnect: AtomicBool,
    observer_rpc_supported: AtomicBool,
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
            terminals: Mutex::new(Vec::new()),
            user_disconnect: AtomicBool::new(false),
            observer_rpc_supported: AtomicBool::new(true),
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

    pub fn user_disconnect(&self) -> bool {
        self.0.user_disconnect.load(Ordering::Acquire)
    }

    pub fn set_user_disconnect(&self, v: bool) {
        self.0.user_disconnect.store(v, Ordering::Release);
    }

    pub fn clear_session(&self) {
        self.set_user_disconnect(true);
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

    pub fn add_terminal(&self, terminal: RemoteTerminal) {
        if let Ok(mut t) = self.0.terminals.lock() {
            t.push(terminal);
        }
    }

    pub fn remove_terminal(&self, id: &str) {
        if let Ok(mut t) = self.0.terminals.lock() {
            t.retain(|t| t.id() != id);
        }
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
            .client()?
            .rpc(FsListReq {
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

    pub async fn fs_read(&self, path: &str) -> Result<FsReadResult> {
        Ok(self
            .client()?
            .rpc(FsReadReq {
                path: path.to_string(),
            })
            .await?)
    }

    pub async fn fs_write(&self, path: &str, content: &str) -> Result<()> {
        let _: FsWriteResult = self
            .client()?
            .rpc(FsWriteReq {
                path: path.to_string(),
                content: content.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn fs_stat(&self, path: &str) -> Result<FsStatResult> {
        let result = self
            .client()?
            .rpc(FsStatReq {
                path: path.to_string(),
            })
            .await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result)
    }

    pub async fn fs_watch(&self, path: &str) -> Result<FsWatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsWatchResult::Unsupported);
        }
        match self
            .client()?
            .rpc(FsWatchReq {
                path: path.to_string(),
            })
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => {
                if self.downgrade_observer_rpc(&e.to_string()) {
                    return Ok(FsWatchResult::Unsupported);
                }
                Err(anyhow::anyhow!(e.to_string()))
            }
        }
    }

    pub async fn fs_unwatch(&self, path: &str) -> Result<FsUnwatchResult> {
        if !self.0.observer_rpc_supported.load(Ordering::Acquire) {
            return Ok(FsUnwatchResult::Unsupported);
        }
        match self
            .client()?
            .rpc(FsUnwatchReq {
                path: path.to_string(),
            })
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => {
                if self.downgrade_observer_rpc(&e.to_string()) {
                    return Ok(FsUnwatchResult::Unsupported);
                }
                Err(anyhow::anyhow!(e.to_string()))
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

    // ─── RPC: git ────────────────────────────────────────────────────────────

    pub async fn git_status(&self) -> Result<GitStatusResult> {
        Ok(self.client()?.rpc(GitStatusReq {}).await?)
    }

    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let result: GitDiffResult = self
            .client()?
            .rpc(GitDiffReq {
                path: path.map(|s| s.to_string()),
                staged,
            })
            .await?;
        Ok(result.diff)
    }

    pub async fn git_log(&self, limit: Option<usize>) -> Result<Vec<GitLogEntry>> {
        let result: GitLogResult = self.client()?.rpc(GitLogReq { limit }).await?;
        Ok(result.entries)
    }

    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let result: GitBranchesResult = self.client()?.rpc(GitBranchesReq {}).await?;
        Ok(result.branches)
    }

    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let _: GitCheckoutResult = self
            .client()?
            .rpc(GitCheckoutReq {
                branch: branch.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let result: GitCommitResult = self
            .client()?
            .rpc(GitCommitReq {
                message: message.to_string(),
                paths: paths.to_vec(),
            })
            .await?;
        Ok(result.hash)
    }

    pub async fn git_stage(&self, paths: &[String]) -> Result<()> {
        let _: GitStageResult = self
            .client()?
            .rpc(GitStageReq {
                paths: paths.to_vec(),
            })
            .await?;
        Ok(())
    }

    pub async fn git_unstage(&self, paths: &[String]) -> Result<()> {
        let _: GitUnstageResult = self
            .client()?
            .rpc(GitUnstageReq {
                paths: paths.to_vec(),
            })
            .await?;
        Ok(())
    }

    // ─── RPC: terminals ──────────────────────────────────────────────────────

    pub async fn terminal_create(&self, cols: u16, rows: u16) -> Result<String> {
        self.terminal_create_with_cmd(cols, rows, None).await
    }

    pub async fn terminal_create_with_cmd(
        &self,
        cols: u16,
        rows: u16,
        launch_cmd: Option<String>,
    ) -> Result<String> {
        let client = self.client()?;
        let result: TermCreateResult = client
            .rpc(TermCreateReq {
                cols,
                rows,
                launch_cmd,
            })
            .await?;

        let terminal = RemoteTerminal::new(result.id.clone());
        if terminal.attach_remote(&client).await.is_ok() {
            self.add_terminal(terminal);
            tracing::info!("Terminal created: {}", result.id);
            Ok(result.id)
        } else {
            Err(anyhow::anyhow!("Failed to attach terminal"))
        }
    }

    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let _: TermResizeResult = self
            .client()?
            .rpc(TermResizeReq {
                id: id.to_string(),
                cols,
                rows,
            })
            .await?;
        Ok(())
    }

    pub async fn terminal_close(&self, id: &str) -> Result<()> {
        let _: TermCloseResult = self
            .client()?
            .rpc(TermCloseReq { id: id.to_string() })
            .await?;
        self.remove_terminal(id);
        Ok(())
    }

    pub async fn terminal_list(&self) -> Result<Vec<String>> {
        let result: TermListResult = self.client()?.rpc(TermListReq {}).await?;
        Ok(result.terminals.into_iter().map(|e| e.id).collect())
    }
}
