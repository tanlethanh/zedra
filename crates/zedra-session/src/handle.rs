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
        let result: GitStatusResult = self.client()?.rpc(GitStatusReq {}).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result)
    }

    pub async fn git_diff(&self, path: Option<&str>, staged: bool) -> Result<String> {
        let result: GitDiffResult = self
            .client()?
            .rpc(GitDiffReq {
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
        let result: GitLogResult = self.client()?.rpc(GitLogReq { limit }).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.entries)
    }

    pub async fn git_branches(&self) -> Result<Vec<GitBranchEntry>> {
        let result: GitBranchesResult = self.client()?.rpc(GitBranchesReq {}).await?;
        if let Some(e) = result.error {
            return Err(anyhow::anyhow!(e));
        }
        Ok(result.branches)
    }

    pub async fn git_checkout(&self, branch: &str) -> Result<()> {
        let result: GitCheckoutResult = self
            .client()?
            .rpc(GitCheckoutReq {
                branch: branch.to_string(),
            })
            .await?;
        git_checkout_result(result, branch)
    }

    pub async fn git_commit(&self, message: &str, paths: &[String]) -> Result<String> {
        let result: GitCommitResult = self
            .client()?
            .rpc(GitCommitReq {
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
            .client()?
            .rpc(GitStageReq {
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
            .client()?
            .rpc(GitUnstageReq {
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

    pub async fn terminal_reorder(&self, ordered_ids: Vec<String>) -> Result<()> {
        let result: TermReorderResult = self
            .client()?
            .rpc(TermReorderReq {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
