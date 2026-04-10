use std::collections::HashSet;
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
    terminals: Mutex<Vec<Arc<RemoteTerminal>>>,
    user_disconnect: AtomicBool,
    git_needs_refresh: AtomicBool,
    fs_changed_paths: Mutex<HashSet<String>>,
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
            git_needs_refresh: AtomicBool::new(false),
            fs_changed_paths: Mutex::new(HashSet::new()),
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
            .map(|t| t.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default()
    }

    pub fn terminal(&self, id: &str) -> Option<Arc<RemoteTerminal>> {
        self.0
            .terminals
            .lock()
            .ok()
            .and_then(|t| t.iter().find(|t| t.id == id).cloned())
    }

    pub fn sync_terminals(&self, entries: &[TerminalSyncEntry]) {
        let Ok(mut terminals) = self.0.terminals.lock() else {
            return;
        };

        let mut by_id: std::collections::HashMap<_, _> =
            terminals.drain(..).map(|t| (t.id.clone(), t)).collect();

        for entry in entries {
            let terminal = by_id
                .remove(&entry.id)
                .unwrap_or_else(|| RemoteTerminal::new(entry.id.clone()));
            terminal.update_seq(entry.last_seq);
            if let Ok(mut meta) = terminal.meta.lock() {
                meta.title = entry.title.clone();
                meta.cwd = entry.cwd.clone();
            }
            terminals.push(terminal);
        }
    }

    pub fn add_terminal(&self, terminal: Arc<RemoteTerminal>) {
        if let Ok(mut t) = self.0.terminals.lock() {
            t.push(terminal);
        }
    }

    pub fn remove_terminal(&self, id: &str) {
        if let Ok(mut t) = self.0.terminals.lock() {
            t.retain(|t| t.id != id);
        }
    }

    pub fn take_git_refresh(&self) -> bool {
        self.0.git_needs_refresh.swap(false, Ordering::AcqRel)
    }

    pub fn set_git_needs_refresh(&self) {
        self.0.git_needs_refresh.store(true, Ordering::Release);
    }

    pub fn take_fs_changed(&self) -> Vec<String> {
        self.0
            .fs_changed_paths
            .lock()
            .map(|mut c| c.drain().collect())
            .unwrap_or_default()
    }

    pub fn add_fs_changed(&self, path: String) {
        if let Ok(mut c) = self.0.fs_changed_paths.lock() {
            c.insert(path);
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
        Ok(self
            .client()?
            .rpc(FsStatReq {
                path: path.to_string(),
            })
            .await?)
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
        self.attach_terminal(&client, &terminal).await?;
        self.add_terminal(terminal);

        tracing::info!("Terminal created: {}", result.id);
        Ok(result.id)
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

    pub async fn attach_terminal(
        &self,
        client: &irpc::Client<ZedraProto>,
        terminal: &Arc<RemoteTerminal>,
    ) -> Result<()> {
        let (irpc_input_tx, mut irpc_output_rx) = client
            .bidi_streaming::<TermAttachReq, TermInput, TermOutput>(
                TermAttachReq {
                    id: terminal.id.clone(),
                    last_seq: terminal.last_seq(),
                },
                256,
                256,
            )
            .await?;

        let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        terminal.set_input_tx(bridge_tx);

        tokio::spawn(async move {
            while let Some(data) = bridge_rx.recv().await {
                if irpc_input_tx.send(TermInput { data }).await.is_err() {
                    break;
                }
            }
        });

        let terminal_pump = terminal.clone();
        let last_seq = terminal.last_seq();
        tokio::spawn(async move {
            let mut first_msg = true;
            loop {
                match irpc_output_rx.recv().await {
                    Ok(Some(output)) => {
                        if output.seq == 0 {
                            terminal_pump.push_output(output.data);
                            terminal_pump.signal_needs_render();
                            continue;
                        }
                        if first_msg {
                            first_msg = false;
                            if last_seq > 0 && output.seq > last_seq + 1 {
                                terminal_pump.reset_osc_scanner();
                                terminal_pump.push_output(b"\x1bc".to_vec());
                            }
                        }
                        terminal_pump.update_seq(output.seq);
                        terminal_pump.push_output(output.data);
                        terminal_pump.signal_needs_render();
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });

        tracing::info!("Terminal {} attached (last_seq={})", terminal.id, last_seq);
        Ok(())
    }

    pub async fn reattach_terminals(&self) -> Result<()> {
        let client = self.client()?;
        let terminals: Vec<_> = self
            .0
            .terminals
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();

        if terminals.is_empty() {
            return Ok(());
        }

        let server_ids: HashSet<String> = match client.rpc(TermListReq {}).await {
            Ok(result) => result.terminals.into_iter().map(|e| e.id).collect(),
            Err(_) => HashSet::new(),
        };

        if let Ok(mut t) = self.0.terminals.lock() {
            t.retain(|term| server_ids.contains(&term.id));
        }

        let terminals: Vec<_> = self
            .0
            .terminals
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        for terminal in &terminals {
            if let Err(e) = self.attach_terminal(&client, terminal).await {
                tracing::warn!("failed to reattach terminal {}: {}", terminal.id, e);
            }
        }

        Ok(())
    }
}
