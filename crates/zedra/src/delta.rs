use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use futures::channel::oneshot;
use gpui::{AsyncApp, Context, Entity, EventEmitter};
use gpui_tokio::Tokio;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use zedra_session::signer::ClientSigner;

use crate::{platform_bridge, workspace_state::WorkspaceState};

const DEFAULT_BASE_URL: &str = "https://delta.zedra.dev";
const STORE_DIR: &str = "zedra";
const STATE_FILE: &str = "delta.json";
const CLIENT_KEY_FILE: &str = "client.key";

#[derive(Serialize)]
struct OAuthRequest {
    id_token: String,
}

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    expires_at: String,
    user: UserSummary,
    stack: StackSummary,
}

#[derive(Deserialize)]
struct UserSummary {
    id: Uuid,
}

#[derive(Deserialize)]
struct StackSummary {
    id: Uuid,
}

#[derive(Serialize)]
struct NodeRegistrationRequest {
    public_key: String,
    kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    metadata: Value,
    receive_notifications: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NodeKind {
    Ios,
    Android,
    Host,
    Agent,
    External,
}

impl NodeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
            Self::Host => "host",
            Self::Agent => "agent",
            Self::External => "external",
        }
    }
}

#[derive(Deserialize)]
struct NodeRegistrationResponse {
    node: NodeSummary,
    created: bool,
}

#[derive(Deserialize)]
struct NodeSummary {
    id: Uuid,
    #[serde(default)]
    alias: Option<String>,
    kind: NodeKind,
    display_name: Option<String>,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    joined_at: Option<String>,
}

#[derive(Deserialize)]
struct NodeListResponse {
    nodes: Vec<NodeSummary>,
}

/// One registered node in the stack, cached in `delta.json` so the Account
/// screen paints immediately on a cold start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredNode {
    pub id: Uuid,
    #[serde(default)]
    pub alias: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub joined_at: Option<String>,
}

impl StoredNode {
    /// Alias, then display name, then a short id — never empty.
    pub fn name(&self) -> String {
        self.alias
            .as_deref()
            .or(self.display_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.id.to_string().chars().take(8).collect())
    }

    /// `2026-08-18` from the stored RFC 3339 timestamp.
    pub fn joined_date(&self) -> Option<String> {
        let joined = self.joined_at.as_deref()?;
        joined.split('T').next().map(str::to_string)
    }
}

#[derive(Deserialize)]
struct NodeDetailResponse {
    node: NodeSummary,
}

#[derive(Serialize)]
struct NodeUpdateRequest {
    alias: Option<String>,
}

#[derive(Deserialize)]
struct NodeUpdateResponse {
    #[allow(dead_code)]
    node: NodeSummary,
}

#[derive(Serialize)]
struct PushTokenRequest {
    provider: PushProvider,
    token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PushProvider {
    Apns,
    Fcm,
    Mock,
}

#[derive(Deserialize)]
struct PushTokenResponse {
    #[allow(dead_code)]
    id: Uuid,
}

#[derive(Clone, Debug)]
pub struct DeltaStatus {
    pub base_url: String,
    pub signed_in: bool,
    pub email: Option<String>,
    pub stack_id: Option<Uuid>,
    pub node_id: Option<Uuid>,
    pub push_provider: Option<String>,
    pub push_environment: Option<String>,
    pub push_registered: bool,
    pub nodes: Vec<StoredNode>,
    /// `true` when the cached node list is missing or past its TTL.
    pub nodes_stale: bool,
}

/// A missing email never means signed out: Apple hidden-relay sign-in has no email,
/// so fall back to the sign-in flag rather than the "Not signed in" label.
pub fn account_label(status: &DeltaStatus) -> String {
    status.email.clone().unwrap_or_else(|| {
        if status.signed_in {
            "Signed in".to_string()
        } else {
            "Not signed in".to_string()
        }
    })
}

pub fn account_initial(status: &DeltaStatus) -> String {
    status
        .email
        .as_deref()
        .and_then(|email| email.chars().next())
        .unwrap_or('Z')
        .to_ascii_uppercase()
        .to_string()
}

#[derive(Clone)]
pub struct ClientDeltaInfo {
    pub delta_url: String,
    pub stack_id: Uuid,
    pub node_id: Uuid,
}

#[derive(Clone, Debug)]
pub enum DeltaStateEvent {
    DeltaStateChanged,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeltaHostNode {
    host_node_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_version: Option<String>,
}

impl DeltaHostNode {
    fn from_response(node: &NodeSummary) -> Self {
        let metadata = node.metadata.as_object();
        Self {
            host_node_id: node.id,
            display_name: node.display_name.clone().or_else(|| node.alias.clone()),
            hostname: metadata
                .and_then(|value| value.get("hostname"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            username: metadata
                .and_then(|value| value.get("username"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            workdir: metadata
                .and_then(|value| value.get("workdir"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            os_version: metadata
                .and_then(|value| value.get("os_version"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            host_version: metadata
                .and_then(|value| value.get("host_version"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
        }
    }

    pub fn from_host_node_id(host_node_id: Uuid) -> Self {
        Self {
            host_node_id,
            display_name: None,
            hostname: None,
            username: None,
            workdir: None,
            os_version: None,
            host_version: None,
        }
    }

    pub fn host_node_id(&self) -> Uuid {
        self.host_node_id
    }
}

/// Canonical Delta client state. The live copy lives in a GPUI entity
/// (`Entity<DeltaState>`) shared across features; async network calls take a
/// `clone()` snapshot in and return the mutated value, which the caller applies
/// back to the entity via [`DeltaState::apply`]. Also serialized to disk.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaState {
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    user_id: Option<Uuid>,
    #[serde(default)]
    stack_id: Option<Uuid>,
    #[serde(default)]
    node_id: Option<Uuid>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    push_token: Option<StoredPushToken>,
    #[serde(default)]
    host_nodes_by_pubkey: HashMap<String, DeltaHostNode>,
    #[serde(default)]
    nodes: Vec<StoredNode>,
    #[serde(default)]
    nodes_fetched_at: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct StoredPushToken {
    provider: String,
    token: String,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default)]
    registered: bool,
}

impl Default for DeltaState {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            access_token: None,
            refresh_token: None,
            expires_at: None,
            user_id: None,
            stack_id: None,
            node_id: None,
            email: None,
            push_token: None,
            host_nodes_by_pubkey: HashMap::new(),
            nodes: Vec::new(),
            nodes_fetched_at: None,
        }
    }
}

/// The groups of persisted Delta state an operation may change. Built by
/// diffing what an operation started from against what it returned, so network
/// functions keep returning a whole `DeltaState` and only the merge is precise.
#[derive(Default)]
pub struct DeltaPatch {
    base_url: Option<String>,
    session: Option<SessionFields>,
    push_token: Option<Option<StoredPushToken>>,
    host_nodes: Option<HashMap<String, DeltaHostNode>>,
    nodes: Option<(Option<Uuid>, Vec<StoredNode>, Option<String>)>,
}

struct SessionFields {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_at: Option<String>,
    user_id: Option<Uuid>,
    stack_id: Option<Uuid>,
    node_id: Option<Uuid>,
    email: Option<String>,
}

impl DeltaPatch {
    /// The groups that differ between the state an operation started from and
    /// the state it produced.
    pub fn between(before: &DeltaState, after: &DeltaState) -> Self {
        let session_changed = before.access_token != after.access_token
            || before.refresh_token != after.refresh_token
            || before.expires_at != after.expires_at
            || before.user_id != after.user_id
            || before.stack_id != after.stack_id
            || before.node_id != after.node_id
            || before.email != after.email;
        Self {
            base_url: (before.base_url != after.base_url).then(|| after.base_url.clone()),
            session: session_changed.then(|| SessionFields {
                access_token: after.access_token.clone(),
                refresh_token: after.refresh_token.clone(),
                expires_at: after.expires_at.clone(),
                user_id: after.user_id,
                stack_id: after.stack_id,
                node_id: after.node_id,
                email: after.email.clone(),
            }),
            push_token: (before.push_token != after.push_token).then(|| after.push_token.clone()),
            host_nodes: (before.host_nodes_by_pubkey != after.host_nodes_by_pubkey)
                .then(|| after.host_nodes_by_pubkey.clone()),
            nodes: (before.nodes != after.nodes).then(|| {
                (
                    after.stack_id,
                    after.nodes.clone(),
                    after.nodes_fetched_at.clone(),
                )
            }),
        }
    }

    fn is_empty(&self) -> bool {
        self.base_url.is_none()
            && self.session.is_none()
            && self.push_token.is_none()
            && self.host_nodes.is_none()
            && self.nodes.is_none()
    }
}

impl DeltaState {
    /// Load the persisted state from disk, falling back to defaults. Used to
    /// seed the shared entity at app launch.
    pub fn load() -> Self {
        load_state_from_disk().unwrap_or_else(|error| {
            tracing::warn!("Delta state load failed; using defaults: {error:#}");
            DeltaState::default()
        })
    }

    /// Snapshot for handing to async network operations off the UI thread.
    pub fn snapshot(&self) -> DeltaState {
        self.clone()
    }

    /// Merge the groups an async operation actually changed. This is the only
    /// way persisted state changes: whole-state assignment let two operations in
    /// flight overwrite each other's untouched fields.
    ///
    /// Returns `false` when the patch was dropped as stale.
    pub fn merge(&mut self, patch: DeltaPatch, cx: &mut Context<Self>) -> bool {
        if patch.is_empty() {
            return true;
        }
        let applied = self.merge_fields(patch);
        self.persist();
        cx.emit(DeltaStateEvent::DeltaStateChanged);
        cx.notify();
        applied
    }

    /// The merge rules themselves, free of the entity plumbing so they can be
    /// tested directly. `false` means the node group was dropped as stale.
    fn merge_fields(&mut self, patch: DeltaPatch) -> bool {
        if let Some(base_url) = patch.base_url {
            self.base_url = base_url;
        }
        if let Some(session) = patch.session {
            self.access_token = session.access_token;
            self.refresh_token = session.refresh_token;
            self.expires_at = session.expires_at;
            self.user_id = session.user_id;
            self.stack_id = session.stack_id;
            self.node_id = session.node_id;
            self.email = session.email;
        }
        // A signed-out account owns no stack-scoped data, so a late result from
        // the previous session must not repopulate it.
        if !self.is_signed_in() {
            self.push_token = None;
            self.nodes.clear();
            self.nodes_fetched_at = None;
            self.host_nodes_by_pubkey.clear();
            return true;
        }
        if let Some(push_token) = patch.push_token {
            self.push_token = push_token;
        }
        if let Some(host_nodes) = patch.host_nodes {
            self.host_nodes_by_pubkey = host_nodes;
        }
        if let Some((stack_id, nodes, fetched_at)) = patch.nodes {
            // The cache belongs to one stack; a result for another is stale.
            if stack_id.is_some() && stack_id != self.stack_id {
                return false;
            }
            self.nodes = nodes;
            self.nodes_fetched_at = fetched_at;
        }
        true
    }

    #[cfg(test)]
    fn apply_patch_for_test(&mut self, patch: DeltaPatch) -> bool {
        self.merge_fields(patch)
    }

    fn persist(&self) {
        if let Err(error) = save_state(self) {
            tracing::warn!(error = %error, "delta: persisting state failed");
        }
    }

    /// Compare only the fields that affect the host/client handoff, so a
    /// push-token write cannot cancel a valid host notification replay.
    pub(crate) fn matches_client_binding_state(&self, expected: &DeltaState) -> bool {
        self.base_url == expected.base_url
            && self.access_token == expected.access_token
            && self.refresh_token == expected.refresh_token
            && self.expires_at == expected.expires_at
            && self.user_id == expected.user_id
            && self.stack_id == expected.stack_id
            && self.node_id == expected.node_id
            && self.email == expected.email
    }

    pub(crate) fn is_signed_in(&self) -> bool {
        self.access_token.is_some() && self.stack_id.is_some()
    }

    pub fn status(&self) -> DeltaStatus {
        DeltaStatus {
            base_url: self.base_url.clone(),
            signed_in: self.access_token.is_some() && self.stack_id.is_some(),
            email: self.email.clone(),
            stack_id: self.stack_id,
            node_id: self.node_id,
            push_provider: self.push_token.as_ref().map(|token| token.provider.clone()),
            push_environment: self
                .push_token
                .as_ref()
                .and_then(|token| token.environment.clone()),
            push_registered: self
                .push_token
                .as_ref()
                .map(|token| token.registered)
                .unwrap_or(false),
            nodes: self.nodes.clone(),
            nodes_stale: self.nodes_are_stale(),
        }
    }

    fn nodes_are_stale(&self) -> bool {
        let Some(fetched_at) = self
            .nodes_fetched_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        else {
            return true;
        };
        let age = chrono::Utc::now().signed_duration_since(fetched_at.with_timezone(&chrono::Utc));
        age.to_std()
            .map(|age| age >= NODES_CACHE_TTL)
            .unwrap_or(true)
    }

    /// Stack/node identity to hand to a paired host so it can address push
    /// notifications at this client. `None` until signed in with a node id.
    pub fn current_client_info(&self) -> Option<ClientDeltaInfo> {
        self.access_token.as_ref()?;
        Some(ClientDeltaInfo {
            delta_url: self.base_url.clone(),
            stack_id: self.stack_id?,
            node_id: self.node_id?,
        })
    }

    pub fn host_node_for_pubkey(&self, pubkey: [u8; 32]) -> Option<DeltaHostNode> {
        self.host_nodes_by_pubkey
            .get(&encode_base64_no_pad(pubkey))
            .cloned()
    }

    pub fn remember_host_node(
        &mut self,
        pubkey: [u8; 32],
        host_node: DeltaHostNode,
        cx: &mut Context<Self>,
    ) {
        let key = encode_base64_no_pad(pubkey);
        let changed = self
            .host_nodes_by_pubkey
            .get(&key)
            .map(|entry| entry != &host_node)
            .unwrap_or(true);
        if changed {
            self.host_nodes_by_pubkey.insert(key, host_node);
            cx.emit(DeltaStateEvent::DeltaStateChanged);
            cx.notify();
        }
    }
}

impl EventEmitter<DeltaStateEvent> for DeltaState {}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

/// Clear all signed-in state, preserving the configured base URL. Persists to
/// disk and returns the cleared snapshot to apply back onto the entity.
pub fn sign_out(current: DeltaState) -> Result<DeltaState> {
    let state = DeltaState {
        base_url: current.base_url,
        ..DeltaState::default()
    };
    save_state(&state)?;
    Ok(state)
}

/// Permanently delete the signed-in Delta account and its server-side data.
/// Paired-host transport identities stay on-device, but Delta host bindings do not.
pub async fn delete_account(mut state: DeltaState) -> Result<DeltaState> {
    delete_bearer(&mut state, "/v1/me").await?;
    WorkspaceState::clear_delta_bindings().map_err(anyhow::Error::msg)?;
    sign_out(state)
}

#[derive(Debug, PartialEq, Eq)]
pub enum MobileNodeReconcileResult {
    Skipped,
    Unchanged,
    Updated,
    SignedOut,
}

pub async fn reconcile_mobile_node(
    mut state: DeltaState,
) -> Result<(MobileNodeReconcileResult, DeltaState)> {
    let (Some(stack_id), Some(node_id)) = (state.stack_id, state.node_id) else {
        return Ok((MobileNodeReconcileResult::Skipped, state));
    };
    if state.access_token.is_none() {
        return Ok((MobileNodeReconcileResult::Skipped, state));
    }

    let path = format!("/v1/stacks/{stack_id}/nodes/{node_id}");
    let Some(stored) = get_bearer::<NodeDetailResponse>(&mut state, &path).await? else {
        let state = sign_out(state)?;
        return Ok((MobileNodeReconcileResult::SignedOut, state));
    };

    let display_name = mobile_display_name();
    let desired_kind = mobile_node_kind();
    let desired_metadata = mobile_node_metadata(&display_name, desired_kind);
    if mobile_node_matches(&stored.node, desired_kind, &display_name, &desired_metadata) {
        save_state(&state)?;
        return Ok((MobileNodeReconcileResult::Unchanged, state));
    }

    let mobile =
        register_mobile_node(&mut state, load_mobile_signer()?.pubkey(), &display_name).await?;
    state.node_id = Some(mobile.node.id);
    save_state(&state)?;
    Ok((MobileNodeReconcileResult::Updated, state))
}

pub async fn sign_in_with_google(
    state: DeltaState,
    id_token: String,
    email: Option<String>,
) -> Result<DeltaState> {
    sign_in_with_oauth("google", state, id_token, email).await
}

pub async fn sign_in_with_apple(
    state: DeltaState,
    id_token: String,
    email: Option<String>,
) -> Result<DeltaState> {
    sign_in_with_oauth("apple", state, id_token, email).await
}

async fn sign_in_with_oauth(
    provider: &str,
    mut state: DeltaState,
    id_token: String,
    email: Option<String>,
) -> Result<DeltaState> {
    state.base_url = normalize_base_url(&state.base_url);

    let auth: AuthResponse = http()
        .post(format!("{}/v1/auth/oauth/{provider}", state.base_url))
        .json(&OAuthRequest { id_token })
        .send()
        .await
        .context("send OAuth request to Delta")?
        .error_for_status()
        .context("Delta rejected OAuth token")?
        .json()
        .await
        .context("decode Delta OAuth response")?;

    state.access_token = Some(auth.access_token);
    state.refresh_token = Some(auth.refresh_token);
    state.expires_at = Some(auth.expires_at);
    state.user_id = Some(auth.user.id);
    state.stack_id = Some(auth.stack.id);
    state.node_id = None;
    state.email = email.or(state.email);
    save_state(&state)?;

    let signer = load_mobile_signer()?;
    let mobile_name = mobile_display_name();
    let mobile = register_mobile_node(&mut state, signer.pubkey(), &mobile_name).await?;
    state.node_id = Some(mobile.node.id);
    save_state(&state)?;
    let alias_is_default = mobile
        .node
        .alias
        .as_deref()
        .map(|a| matches!(a, "ios" | "android" | "zedra-ios" | "zedra-android"))
        .unwrap_or(true);
    if !mobile.created && alias_is_default {
        // Skip update when the device name has no alias-safe characters; the
        // server-assigned default ("ios"/"android") stays in place.
        if let Some(alias) = normalize_alias_candidate(&mobile_name) {
            if let Err(err) = update_mobile_alias(&mut state, mobile.node.id, &alias).await {
                tracing::warn!("Delta mobile node alias update failed: {err:#}");
            }
        }
    }

    if state.push_token.is_some() {
        if let Err(err) = register_stored_push_token(&mut state).await {
            tracing::warn!("Delta push token registration after sign-in failed: {err:#}");
            save_state(&state)?;
        }
    }

    Ok(state)
}

/// How long the cached node list stays fresh before the Account screen refetches.
const NODES_CACHE_TTL: Duration = Duration::from_secs(300);

/// Fetch the stack's registered nodes. Returns the state as it stands after the
/// call so a token refresh triggered by `get_bearer` is not lost.
pub async fn fetch_nodes(mut state: DeltaState) -> Result<DeltaState> {
    let stack_id = state.stack_id.context("Delta stack is missing")?;
    let path = format!("/v1/stacks/{stack_id}/nodes");
    let list = get_bearer::<NodeListResponse>(&mut state, &path)
        .await?
        .context("Delta stack was not found")?;
    state.nodes = list
        .nodes
        .into_iter()
        .map(|node| StoredNode {
            id: node.id,
            alias: node.alias,
            kind: node.kind.as_str().to_string(),
            display_name: node.display_name,
            joined_at: node.joined_at,
        })
        .collect();
    state.nodes_fetched_at = Some(chrono::Utc::now().to_rfc3339());
    save_state(&state)?;
    Ok(state)
}

/// Native permission prompt, then registration of the returned token on Delta,
/// applied onto `delta_state`. Shared by every screen that offers to enable
/// notifications. `Ok(false)` means the request was abandoned before a token
/// arrived, which is silent — the user dismissed it or the screen went away.
pub async fn acquire_and_register_push_token(
    delta_state: Entity<DeltaState>,
    cx: &mut AsyncApp,
    on_registering: impl FnOnce(&mut AsyncApp),
) -> Result<bool> {
    let (tx, rx) = oneshot::channel();
    platform_bridge::request_delta_push_token(move |result| {
        let _ = tx.send(result);
    });
    let token = match rx.await {
        Ok(Ok(token)) => token,
        Ok(Err(message)) => bail!(message),
        Err(_) => return Ok(false),
    };
    on_registering(cx);
    let snapshot = delta_state.read_with(cx, |state, _| state.snapshot());
    let next = Tokio::spawn_result(
        cx,
        register_push_token(
            snapshot.clone(),
            token.provider,
            token.token,
            token.environment,
        ),
    )
    .await?;
    // Keep a newer state change from being overwritten by this stale result.
    let applied = delta_state.update(cx, |state, cx| {
        state.merge(DeltaPatch::between(&snapshot, &next), cx)
    });
    if !applied {
        tracing::info!("delta: push registration finished after state changed; skipped");
    }
    Ok(true)
}

pub async fn register_push_token(
    mut state: DeltaState,
    provider: String,
    token: String,
    environment: Option<String>,
) -> Result<DeltaState> {
    state.push_token = Some(StoredPushToken {
        provider,
        token,
        environment,
        registered: false,
    });

    if state.access_token.is_some() && state.stack_id.is_some() && state.node_id.is_some() {
        register_stored_push_token(&mut state).await?;
    }

    save_state(&state)?;
    Ok(state)
}

/// Result returned when the mobile app registers a host node with Delta.
pub struct HostNodeRegistrationResult {
    /// The host node record returned by Delta for this stack.
    pub node: DeltaHostNode,
    /// `true` if the host was newly registered; `false` if it was already known to Delta.
    pub created: bool,
}

pub async fn register_paired_host_node(
    mut state: DeltaState,
    public_key: [u8; 32],
    metadata: Value,
) -> Result<(Option<HostNodeRegistrationResult>, DeltaState)> {
    if state.access_token.is_none() {
        return Ok((None, state));
    }
    let Some(stack_id) = state.stack_id else {
        return Ok((None, state));
    };

    let req = NodeRegistrationRequest {
        public_key: encode_base64_no_pad(public_key),
        kind: NodeKind::Host,
        display_name: host_display_name(&metadata),
        metadata,
        receive_notifications: false,
    };
    let resp: NodeRegistrationResponse =
        post_bearer(&mut state, &format!("/v1/stacks/{stack_id}/nodes"), &req).await?;
    Ok((
        Some(HostNodeRegistrationResult {
            node: DeltaHostNode::from_response(&resp.node),
            created: resp.created,
        }),
        state,
    ))
}

async fn register_mobile_node(
    state: &mut DeltaState,
    public_key: [u8; 32],
    display_name: &str,
) -> Result<NodeRegistrationResponse> {
    let stack_id = state.stack_id.context("Delta stack id is missing")?;
    let kind = mobile_node_kind();
    let req = NodeRegistrationRequest {
        public_key: encode_base64_no_pad(public_key),
        kind,
        display_name: Some(display_name.to_string()),
        metadata: mobile_node_metadata(display_name, kind),
        receive_notifications: true,
    };
    post_bearer(state, &format!("/v1/stacks/{stack_id}/nodes"), &req).await
}

async fn update_mobile_alias(state: &mut DeltaState, node_id: Uuid, alias: &str) -> Result<()> {
    let stack_id = state.stack_id.context("Delta stack id is missing")?;
    patch_bearer::<_, NodeUpdateResponse>(
        state,
        &format!("/v1/stacks/{stack_id}/nodes/{node_id}"),
        &NodeUpdateRequest {
            alias: Some(alias.to_string()),
        },
    )
    .await?;
    Ok(())
}

async fn register_stored_push_token(state: &mut DeltaState) -> Result<()> {
    let stack_id = state.stack_id.context("Delta stack id is missing")?;
    let node_id = state.node_id.context("Delta mobile node id is missing")?;
    let Some(push_token) = state.push_token.as_ref() else {
        return Ok(());
    };
    let req = PushTokenRequest {
        provider: parse_push_provider(&push_token.provider)?,
        token: push_token.token.clone(),
        environment: push_token.environment.clone(),
    };
    post_bearer::<_, PushTokenResponse>(
        state,
        &format!("/v1/stacks/{stack_id}/nodes/{node_id}/push-tokens"),
        &req,
    )
    .await?;
    if let Some(push_token) = state.push_token.as_mut() {
        push_token.registered = true;
    }
    Ok(())
}

async fn post_bearer<B, T>(state: &mut DeltaState, path: &str, body: &B) -> Result<T>
where
    B: Serialize + ?Sized,
    T: serde::de::DeserializeOwned,
{
    bearer_json(reqwest::Method::POST, state, path, body).await
}

async fn patch_bearer<B, T>(state: &mut DeltaState, path: &str, body: &B) -> Result<T>
where
    B: Serialize + ?Sized,
    T: serde::de::DeserializeOwned,
{
    bearer_json(reqwest::Method::PATCH, state, path, body).await
}

async fn get_bearer<T>(state: &mut DeltaState, path: &str) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    let mut did_refresh = false;
    loop {
        let access_token = state
            .access_token
            .as_deref()
            .context("Delta auth token is missing")?
            .to_string();
        let response = http()
            .get(delta_url(state, path))
            .bearer_auth(access_token)
            .send()
            .await
            .with_context(|| format!("send Delta request {path}"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && !did_refresh
            && state.refresh_token.is_some()
        {
            did_refresh = true;
            refresh_access_token(state).await?;
            continue;
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        return decode_response(response, path).await.map(Some);
    }
}

async fn bearer_json<B, T>(
    method: reqwest::Method,
    state: &mut DeltaState,
    path: &str,
    body: &B,
) -> Result<T>
where
    B: Serialize + ?Sized,
    T: serde::de::DeserializeOwned,
{
    let mut did_refresh = false;
    loop {
        let access_token = state
            .access_token
            .as_deref()
            .context("Delta auth token is missing")?
            .to_string();
        let response = http()
            .request(method.clone(), delta_url(state, path))
            .bearer_auth(access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("send Delta request {path}"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && !did_refresh
            && state.refresh_token.is_some()
        {
            did_refresh = true;
            refresh_access_token(state).await?;
            continue;
        }

        return decode_response(response, path).await;
    }
}

async fn delete_bearer(state: &mut DeltaState, path: &str) -> Result<()> {
    let mut did_refresh = false;
    loop {
        let access_token = state
            .access_token
            .as_deref()
            .context("Delta auth token is missing")?
            .to_string();
        let response = http()
            .delete(delta_url(state, path))
            .bearer_auth(access_token)
            .send()
            .await
            .with_context(|| format!("send Delta request {path}"))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && !did_refresh
            && state.refresh_token.is_some()
        {
            did_refresh = true;
            refresh_access_token(state).await?;
            continue;
        }
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("Delta request failed: {path} returned HTTP {status}: {text}");
    }
}

async fn refresh_access_token(state: &mut DeltaState) -> Result<()> {
    let refresh_token = state
        .refresh_token
        .as_deref()
        .context("Delta refresh token is missing")?
        .to_string();
    let auth: AuthResponse = http()
        .post(delta_url(state, "/v1/auth/refresh"))
        .json(&RefreshRequest { refresh_token })
        .send()
        .await
        .context("send Delta refresh request")?
        .error_for_status()
        .context("Delta refresh request failed")?
        .json()
        .await
        .context("decode Delta refresh response")?;

    state.access_token = Some(auth.access_token);
    state.refresh_token = Some(auth.refresh_token);
    state.expires_at = Some(auth.expires_at);
    state.user_id = Some(auth.user.id);
    state.stack_id = Some(auth.stack.id);
    save_state(state)?;
    Ok(())
}

async fn decode_response<T>(response: reqwest::Response, path: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Delta request failed: {path} returned HTTP {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("decode Delta response: {path}"))
}

fn delta_url(state: &DeltaState, path: &str) -> String {
    format!("{}/{}", state.base_url, path.trim_start_matches('/'))
}

fn http() -> reqwest::Client {
    let platform = if cfg!(target_os = "ios") {
        "zedra-ios"
    } else if cfg!(target_os = "android") {
        "zedra-android"
    } else {
        "zedra"
    };
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("{}/{}", platform, env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn parse_push_provider(provider: &str) -> Result<PushProvider> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "apns" => Ok(PushProvider::Apns),
        "fcm" => Ok(PushProvider::Fcm),
        "mock" => Ok(PushProvider::Mock),
        other => bail!("unsupported push provider: {other}"),
    }
}

fn host_display_name(metadata: &Value) -> Option<String> {
    metadata
        .get("hostname")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
}

fn mobile_display_name() -> String {
    platform_bridge::device_name().unwrap_or_else(mobile_fallback_display_name)
}

fn mobile_node_kind() -> NodeKind {
    if cfg!(target_os = "android") {
        NodeKind::Android
    } else {
        NodeKind::Ios
    }
}

fn mobile_node_metadata(display_name: &str, kind: NodeKind) -> Value {
    json!({
        "device_name": display_name,
        "platform": match kind { NodeKind::Android => "android", _ => "ios" },
        "os": std::env::consts::OS,
        "os_version": platform_bridge::os_version(),
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "app_version": platform_bridge::app_version_with_build_number(),
    })
}

fn mobile_node_matches(
    stored: &NodeSummary,
    desired_kind: NodeKind,
    desired_display_name: &str,
    desired_metadata: &Value,
) -> bool {
    stored.kind == desired_kind
        && stored.display_name.as_deref() == Some(desired_display_name)
        && desired_metadata.as_object().is_some_and(|desired| {
            desired
                .iter()
                .all(|(key, value)| stored.metadata.get(key) == Some(value))
        })
}

fn mobile_fallback_display_name() -> String {
    if cfg!(target_os = "android") {
        "zedra-android".to_string()
    } else {
        "zedra-ios".to_string()
    }
}

fn normalize_alias_candidate(source: &str) -> Option<String> {
    let mut alias = String::new();
    let mut last_was_dash = false;
    for ch in source.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            alias.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !alias.is_empty() {
            alias.push('-');
            last_was_dash = true;
        }
    }
    while alias.ends_with('-') {
        alias.pop();
    }
    (!alias.is_empty()).then_some(alias)
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn encode_base64_no_pad(input: impl AsRef<[u8]>) -> String {
    STANDARD_NO_PAD.encode(input)
}

fn load_mobile_signer() -> Result<zedra_session::signer::FileClientSigner> {
    zedra_session::signer::FileClientSigner::load_or_generate(&client_key_path()?)
        .context("load Zedra mobile identity key")
}

fn load_state_from_disk() -> Result<DeltaState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(DeltaState::default());
    }
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let mut state: DeltaState =
        serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))?;
    state.base_url = normalize_base_url(&state.base_url);
    Ok(state)
}

fn save_state(state: &DeltaState) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(state)?;
    write_private_file(&path, &bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    Ok(store_dir()?.join(STATE_FILE))
}

fn client_key_path() -> Result<PathBuf> {
    Ok(store_dir()?.join(CLIENT_KEY_FILE))
}

fn store_dir() -> Result<PathBuf> {
    let data_dir = platform_bridge::bridge()
        .data_directory()
        .context("platform data directory is unavailable")?;
    Ok(PathBuf::from(data_dir).join(STORE_DIR))
}

fn write_private_file(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(data)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        DeltaPatch, DeltaState, NodeKind, NodeSummary, StoredNode, StoredPushToken,
        mobile_node_matches, normalize_alias_candidate,
    };

    fn signed_in(stack: Uuid) -> DeltaState {
        DeltaState {
            access_token: Some("access".into()),
            refresh_token: Some("refresh".into()),
            stack_id: Some(stack),
            user_id: Some(Uuid::from_u128(1)),
            ..DeltaState::default()
        }
    }

    fn node(id: Uuid) -> StoredNode {
        StoredNode {
            id,
            alias: Some("macbook".into()),
            kind: "host".into(),
            display_name: None,
            joined_at: None,
        }
    }

    #[test]
    fn patch_carries_only_the_groups_that_changed() {
        let stack = Uuid::from_u128(7);
        let before = signed_in(stack);
        let mut after = before.clone();
        after.nodes = vec![node(Uuid::from_u128(2))];

        let patch = DeltaPatch::between(&before, &after);
        assert!(patch.nodes.is_some());
        assert!(
            patch.session.is_none(),
            "untouched session must not be sent"
        );
        assert!(patch.push_token.is_none());
    }

    #[test]
    fn node_fetch_keeps_credentials_refreshed_mid_flight() {
        let stack = Uuid::from_u128(7);
        let before = signed_in(stack);
        // `get_bearer` refreshed the token while listing nodes.
        let mut after = before.clone();
        after.access_token = Some("rotated".into());
        after.refresh_token = Some("rotated-refresh".into());
        after.nodes = vec![node(Uuid::from_u128(2))];

        let mut live = before.clone();
        let patch = DeltaPatch::between(&before, &after);
        // Merge without a Context: exercise the field rules directly.
        live.apply_patch_for_test(patch);
        assert_eq!(live.access_token.as_deref(), Some("rotated"));
        assert_eq!(live.nodes.len(), 1);
    }

    #[test]
    fn push_registration_does_not_discard_a_concurrent_node_fetch() {
        let stack = Uuid::from_u128(7);
        let before = signed_in(stack);
        let mut fetched = before.clone();
        fetched.nodes = vec![node(Uuid::from_u128(2))];

        // A push registration landed first, touching only its own group.
        let mut live = before.clone();
        live.push_token = Some(StoredPushToken {
            provider: "apns".into(),
            token: "token".into(),
            environment: None,
            registered: true,
        });

        live.apply_patch_for_test(DeltaPatch::between(&before, &fetched));
        assert_eq!(live.nodes.len(), 1, "node list must still land");
        assert!(live.push_token.is_some(), "push token must survive");
    }

    #[test]
    fn signing_out_drops_stack_scoped_data() {
        let stack = Uuid::from_u128(7);
        let before = signed_in(stack);
        let mut live = before.clone();
        live.nodes = vec![node(Uuid::from_u128(2))];
        live.push_token = Some(StoredPushToken {
            provider: "apns".into(),
            token: "token".into(),
            environment: None,
            registered: true,
        });

        let after = DeltaState::default();
        live.apply_patch_for_test(DeltaPatch::between(&before, &after));
        assert!(live.access_token.is_none());
        assert!(
            live.nodes.is_empty(),
            "cached nodes must not outlive sign-out"
        );
        assert!(live.push_token.is_none());
    }

    #[test]
    fn a_node_list_for_another_stack_is_rejected() {
        let before = signed_in(Uuid::from_u128(7));
        let mut after = before.clone();
        after.stack_id = Some(Uuid::from_u128(9));
        after.nodes = vec![node(Uuid::from_u128(2))];

        // Only the node group survives the diff onto a live state on stack 7.
        let mut live = before.clone();
        let mut patch = DeltaPatch::between(&before, &after);
        patch.session = None;
        assert!(!live.apply_patch_for_test(patch));
        assert!(live.nodes.is_empty());
    }

    #[test]
    fn normalizes_alias_candidates_for_mobile_names() {
        assert_eq!(
            normalize_alias_candidate("Tan's iPhone 15 Pro").as_deref(),
            Some("tan-s-iphone-15-pro")
        );
        assert_eq!(normalize_alias_candidate("!!!"), None);
    }

    #[test]
    fn mobile_node_match_ignores_server_owned_metadata() {
        let node = NodeSummary {
            id: Uuid::nil(),
            alias: None,
            kind: NodeKind::Ios,
            display_name: Some("Phone".into()),
            metadata: json!({
                "app_version": "1.0(1)",
                "os_version": "26.0",
                "server_owned": true,
            }),
            joined_at: None,
        };

        assert!(mobile_node_matches(
            &node,
            NodeKind::Ios,
            "Phone",
            &json!({
                "app_version": "1.0(1)",
                "os_version": "26.0",
            }),
        ));
    }

    #[test]
    fn mobile_node_match_detects_mutable_metadata_changes() {
        let node = NodeSummary {
            id: Uuid::nil(),
            alias: None,
            kind: NodeKind::Android,
            display_name: Some("Phone".into()),
            metadata: json!({ "app_version": "1.0(1)" }),
            joined_at: None,
        };

        assert!(!mobile_node_matches(
            &node,
            NodeKind::Android,
            "Phone",
            &json!({ "app_version": "1.1(2)" }),
        ));
    }
}
