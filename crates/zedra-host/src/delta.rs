use anyhow::{Context, Result};
use data_encoding::BASE64_NOPAD;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::identity;

const CONFIG_FILE: &str = "delta.json";
const SIGNING_KEY_FILE: &str = "delta.key";
const DELTA_SIGN_IN_HINT: &str =
    "Not authenticated with Zedra Delta. Run `zedra auth login` to sign in this host.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaConfig {
    pub delta_url: String,
    pub stack_id: Uuid,
    pub node_id: Uuid,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeSummary {
    pub id: Uuid,
    #[serde(default)]
    pub alias: Option<String>,
    pub kind: NodeKind,
    pub display_name: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    pub public_key_fingerprint: String,
    pub push_enabled: bool,
    #[serde(default)]
    pub joined_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Ios,
    Android,
    Host,
    Agent,
    #[serde(other)]
    External,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
            Self::Host => "host",
            Self::Agent => "agent",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationSendResponse {
    pub accepted: bool,
    pub recipients: u32,
    pub provider_success: u32,
    pub provider_failure: u32,
    #[serde(default)]
    pub errors: Vec<ProviderError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderError {
    pub node_id: Uuid,
    pub provider: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveActivityUpdateResponse {
    pub accepted: bool,
    pub recipients: u32,
    pub provider_success: u32,
    pub provider_failure: u32,
    #[serde(default)]
    pub errors: Vec<ProviderError>,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    expires_at: String,
    stack: StackSummary,
}

#[derive(Debug, Deserialize)]
struct StackSummary {
    id: Uuid,
}

#[derive(Debug, Serialize)]
struct DevAuthRequest {
    subject: String,
    email: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Clone)]
pub struct CliAuthSession {
    pub auth_url: String,
    pub user_code: String,
    pub expires_at: String,
    poll_token: String,
    delta_url: String,
}

#[derive(Debug, Serialize)]
struct CliAuthStartRequest {
    public_key: String,
    display_name: Option<String>,
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct CliAuthStartResponse {
    auth_url: String,
    user_code: String,
    poll_token: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct CliAuthPollRequest {
    poll_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CliAuthPollResponse {
    Pending,
    Approved {
        stack_id: Uuid,
        node_id: Uuid,
        access_token: String,
        refresh_token: String,
        expires_at: String,
    },
    Expired,
    Denied,
}

#[derive(Debug, Serialize)]
struct NodeRegistrationRequest {
    public_key: String,
    kind: NodeKind,
    display_name: Option<String>,
    metadata: Value,
    receive_notifications: bool,
}

#[derive(Debug, Deserialize)]
struct NodeRegistrationResponse {
    node: NodeSummary,
}

#[derive(Debug, Deserialize)]
struct NodeListResponse {
    nodes: Vec<NodeSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeKeySummary {
    pub node_id: Uuid,
    #[serde(default)]
    pub alias: Option<String>,
    pub kind: NodeKind,
    /// Ed25519 public key, base64 without padding.
    pub public_key: String,
    pub public_key_fingerprint: String,
}

#[derive(Debug, Deserialize)]
struct NodeKeyListResponse {
    keys: Vec<NodeKeySummary>,
}

/// One active grant where the queried node is the subject — an ability it holds.
#[derive(Debug, Clone, Deserialize)]
pub struct NodeAbilitySummary {
    pub ability: String,
    pub object_kind: String,
    pub object_id: Uuid,
    #[serde(default)]
    pub object_alias: Option<String>,
    pub created_at: String,
    /// Operational status declared by the ability spec; absent for
    /// pure-permission abilities.
    #[serde(default)]
    pub status: Option<AbilityStatus>,
}

/// Non-sensitive readiness info for an ability (never tokens or secrets).
#[derive(Debug, Clone, Deserialize)]
pub struct AbilityStatus {
    pub ready: bool,
    #[serde(default)]
    pub detail: serde_json::Value,
}

/// One active grant where the queried node is the object — what others may do to it.
#[derive(Debug, Clone, Deserialize)]
pub struct NodeInboundGrantSummary {
    pub ability: String,
    pub subject_kind: String,
    pub subject_id: Uuid,
    #[serde(default)]
    pub subject_alias: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct NodeDetailResponse {
    pub node: NodeSummary,
    /// Ed25519 public key, base64 without padding.
    pub public_key: String,
    pub abilities: Vec<NodeAbilitySummary>,
    pub grants: Vec<NodeInboundGrantSummary>,
}

/// Stack-level ability state, merged per ability by the server.
#[derive(Debug, Deserialize)]
pub struct AbilityStateRollupResponse {
    pub states: std::collections::HashMap<String, Value>,
}

/// One grant edge from the grant management API.
#[derive(Debug, Clone, Deserialize)]
pub struct GrantSummary {
    pub id: Uuid,
    pub subject_kind: String,
    pub subject_id: Uuid,
    #[serde(default)]
    pub subject_alias: Option<String>,
    pub ability: String,
    pub object_kind: String,
    pub object_id: Uuid,
    #[serde(default)]
    pub object_alias: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub granted_by_kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GrantListResponse {
    grants: Vec<GrantSummary>,
}

#[derive(Debug, Deserialize)]
struct GrantCreateResponse {
    grant: GrantSummary,
}

#[derive(Debug, Serialize)]
struct GrantCreateRequest {
    subject_kind: &'static str,
    subject_id: Uuid,
    ability: String,
}

#[derive(Debug, Serialize)]
struct NodeUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NodeUpdateResponse {
    node: NodeSummary,
}

#[derive(Debug, Deserialize)]
pub struct NodeDeleteResponse {
    pub node_id: Uuid,
    pub deleted: bool,
}

#[derive(Debug, Serialize)]
struct NotificationSendRequest {
    target_node_id: Option<Uuid>,
    title: String,
    body: Option<String>,
    category: Option<String>,
    priority: NotificationPriority,
    ttl_seconds: Option<u32>,
    collapse_key: Option<String>,
    deeplink: Option<String>,
    data: Value,
}

#[derive(Debug, Serialize)]
struct LiveActivityUpdateRequest {
    target_node_id: Option<Uuid>,
    activity_id: String,
    event: LiveActivityEvent,
    alert_title: Option<String>,
    alert_body: Option<String>,
    content_state: Value,
    stale_at: Option<String>,
    dismissal_at: Option<String>,
    priority: NotificationPriority,
    ttl_seconds: Option<u32>,
    collapse_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum LiveActivityEvent {
    Update,
    End,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum NotificationPriority {
    Normal,
}

pub fn config_path() -> Result<PathBuf> {
    Ok(identity::host_config_dir()?.join(CONFIG_FILE))
}

pub fn load_config() -> Result<DeltaConfig> {
    let path = config_path()?;
    load_config_at(&path)
}

fn load_config_at(path: &Path) -> Result<DeltaConfig> {
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(DELTA_SIGN_IN_HINT);
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to read Delta auth config from {}", path.display())
            });
        }
    };
    serde_json::from_str(&json).context("failed to parse Delta auth config")
}

pub fn remove_config() -> Result<bool> {
    let path = config_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMetadataReconcileResult {
    Skipped,
    Missing,
    Unchanged,
    Updated,
}

pub async fn reconcile_signed_in_host_metadata() -> Result<HostMetadataReconcileResult> {
    if !config_path()?.exists() {
        return Ok(HostMetadataReconcileResult::Skipped);
    }

    let mut config = load_config()?;
    let client = DeltaHttp::new(&config.delta_url);
    let path = format!("/v1/stacks/{}/nodes/{}", config.stack_id, config.node_id);
    let stored = match client
        .get_bearer_optional::<NodeDetailResponse>(&path, &config.access_token)
        .await?
    {
        BearerGetResult::Found(detail) => detail,
        BearerGetResult::Missing => return Ok(HostMetadataReconcileResult::Missing),
        BearerGetResult::Unauthorized => {
            refresh_host_access_token(&client, &mut config).await?;
            match client
                .get_bearer_optional::<NodeDetailResponse>(&path, &config.access_token)
                .await?
            {
                BearerGetResult::Found(detail) => detail,
                BearerGetResult::Missing => return Ok(HostMetadataReconcileResult::Missing),
                BearerGetResult::Unauthorized => {
                    anyhow::bail!("Delta rejected refreshed host access token")
                }
            }
        }
    };

    let display_name = default_host_display_name();
    let metadata = default_host_metadata();
    let public_key = public_key()?;
    let encoded_public_key = encode_base64_no_pad(public_key);
    if stored.public_key == encoded_public_key
        && host_metadata_matches(&stored.node, &display_name, &metadata)
    {
        return Ok(HostMetadataReconcileResult::Unchanged);
    }

    let registered = client
        .register_node(
            &config.access_token,
            config.stack_id,
            &NodeRegistrationRequest {
                public_key: encoded_public_key,
                kind: NodeKind::Host,
                display_name: Some(display_name),
                metadata,
                receive_notifications: false,
            },
        )
        .await?;
    if config.node_id != registered.node.id {
        config.node_id = registered.node.id;
        save_config(&config)?;
    }
    Ok(HostMetadataReconcileResult::Updated)
}

pub async fn dev_auth(delta_url: &str, subject: &str) -> Result<DeltaConfig> {
    let config_dir = identity::host_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let signing_key = load_signing_key()?;
    let client = DeltaHttp::new(delta_url);
    let display_name = default_host_display_name();
    let auth = client
        .dev_auth(&DevAuthRequest {
            subject: subject.to_string(),
            email: None,
            display_name: Some(subject.to_string()),
        })
        .await?;
    let node = client
        .register_node(
            &auth.access_token,
            auth.stack.id,
            &NodeRegistrationRequest {
                public_key: encode_base64_no_pad(signing_key.verifying_key().to_bytes()),
                kind: NodeKind::Host,
                display_name: Some(display_name.clone()),
                metadata: default_host_metadata(),
                receive_notifications: false,
            },
        )
        .await?;
    let config = DeltaConfig {
        delta_url: client.origin().to_string(),
        stack_id: auth.stack.id,
        node_id: node.node.id,
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        token_expires_at: auth.expires_at,
    };
    save_config(&config)?;
    if let Err(err) =
        refresh_host_alias_if_default(&client, &config, &signing_key, &display_name).await
    {
        tracing::warn!("Delta host alias refresh failed: {err:#}");
    }
    Ok(config)
}

pub async fn start_browser_auth(delta_url: &str) -> Result<CliAuthSession> {
    let config_dir = identity::host_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let signing_key = load_signing_key()?;
    let client = DeltaHttp::new(delta_url);
    let display_name = default_host_display_name();
    let started = client
        .cli_auth_start(&CliAuthStartRequest {
            public_key: encode_base64_no_pad(signing_key.verifying_key().to_bytes()),
            display_name: Some(display_name),
            metadata: default_host_metadata(),
        })
        .await?;

    Ok(CliAuthSession {
        auth_url: started.auth_url,
        user_code: started.user_code,
        expires_at: started.expires_at,
        poll_token: started.poll_token,
        delta_url: client.origin().to_string(),
    })
}

pub async fn complete_browser_auth(session: &CliAuthSession) -> Result<DeltaConfig> {
    let client = DeltaHttp::new(&session.delta_url);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10 * 60);
    loop {
        match client
            .cli_auth_poll(&CliAuthPollRequest {
                poll_token: session.poll_token.clone(),
            })
            .await?
        {
            CliAuthPollResponse::Pending => {
                if std::time::Instant::now() >= deadline {
                    anyhow::bail!("CLI auth timed out; start `zedra auth login` again");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            CliAuthPollResponse::Approved {
                stack_id,
                node_id,
                access_token,
                refresh_token,
                expires_at,
            } => {
                let signing_key = load_signing_key()?;
                let display_name = default_host_display_name();
                let config = DeltaConfig {
                    delta_url: session.delta_url.clone(),
                    stack_id,
                    node_id,
                    access_token,
                    refresh_token,
                    token_expires_at: expires_at,
                };
                save_config(&config)?;
                if let Err(err) =
                    refresh_host_alias_if_default(&client, &config, &signing_key, &display_name)
                        .await
                {
                    tracing::warn!("Delta host alias refresh failed: {err:#}");
                }
                return Ok(config);
            }
            CliAuthPollResponse::Expired => {
                anyhow::bail!("CLI auth request expired; start `zedra auth login` again");
            }
            CliAuthPollResponse::Denied => {
                anyhow::bail!("CLI auth request was denied");
            }
        }
    }
}

async fn refresh_host_alias_if_default(
    client: &DeltaHttp,
    config: &DeltaConfig,
    signing_key: &SigningKey,
    display_name: &str,
) -> Result<()> {
    let response: NodeListResponse = client
        .signed_json(
            "GET",
            &format!("/v1/stacks/{}/nodes", config.stack_id),
            config.node_id,
            signing_key,
            None::<&serde_json::Value>,
        )
        .await?;
    let Some(self_node) = response.nodes.iter().find(|node| node.id == config.node_id) else {
        return Ok(());
    };
    if !should_refresh_host_alias(self_node.alias.as_deref(), display_name) {
        return Ok(());
    }
    let _: NodeUpdateResponse = client
        .signed_json(
            "PATCH",
            &format!("/v1/stacks/{}/nodes/{}", config.stack_id, config.node_id),
            config.node_id,
            signing_key,
            Some(&NodeUpdateRequest {
                alias: Some(display_name.to_string()),
                display_name: None,
            }),
        )
        .await?;
    Ok(())
}

fn should_refresh_host_alias(current_alias: Option<&str>, display_name: &str) -> bool {
    let Some(display_alias) = normalize_alias_candidate(display_name) else {
        return false;
    };
    match current_alias {
        None => true,
        Some(alias) if alias == display_alias => false,
        Some("host" | "zedra-host") => display_alias != "host" && display_alias != "zedra-host",
        Some(alias) if alias.starts_with("zedra-host-") => true,
        Some(_) => false,
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

fn save_config(config: &DeltaConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(config)?;
    identity::write_secret_file(&path, &json)?;
    Ok(())
}

fn load_signing_key() -> Result<SigningKey> {
    let path = identity::host_config_dir()?.join(SIGNING_KEY_FILE);
    load_signing_key_at(&path)
}

fn load_signing_key_at(path: &Path) -> Result<SigningKey> {
    let signing_key = if path.exists() {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read Delta signing key from {}", path.display()))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "invalid Delta signing key at {}: expected 32 bytes, got {}",
                path.display(),
                bytes.len()
            )
        })?;
        SigningKey::from_bytes(&bytes)
    } else {
        let signing_key = SigningKey::generate(&mut rand::thread_rng());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        identity::write_secret_file(&path, &signing_key.to_bytes())?;
        tracing::info!("Generated Delta signing key at {}", path.display());
        signing_key
    };
    Ok(signing_key)
}

pub fn public_key() -> Result<[u8; 32]> {
    Ok(load_signing_key()?.verifying_key().to_bytes())
}

fn default_host_display_name() -> String {
    default_hostname().unwrap_or_else(|| "zedra-host".to_string())
}

fn default_host_metadata() -> Value {
    let hostname = default_hostname();
    serde_json::json!({
        "source": "zedra_cli",
        "hostname": hostname,
        "os": std::env::consts::OS,
        "os_version": crate::rpc_daemon::os_version_string(),
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "host_version": env!("CARGO_PKG_VERSION"),
    })
}

fn host_metadata_matches(stored: &NodeSummary, display_name: &str, metadata: &Value) -> bool {
    stored.kind == NodeKind::Host
        && stored.display_name.as_deref() == Some(display_name)
        && metadata.as_object().is_some_and(|desired| {
            desired
                .iter()
                .all(|(key, value)| stored.metadata.get(key) == Some(value))
        })
}

async fn refresh_host_access_token(client: &DeltaHttp, config: &mut DeltaConfig) -> Result<()> {
    let auth = client
        .refresh_auth(&RefreshRequest {
            refresh_token: config.refresh_token.clone(),
        })
        .await?;
    config.access_token = auth.access_token;
    config.refresh_token = auth.refresh_token;
    config.token_expires_at = auth.expires_at;
    config.stack_id = auth.stack.id;
    save_config(config)
}

fn default_hostname() -> Option<String> {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

// ---------------------------------------------------------------------------
// ClientDeltaInfo — reported by a mobile client, held in server memory only
// ---------------------------------------------------------------------------

/// Reported by the mobile client when it connects. Lets the host send push
/// notifications without being signed in. Held in `DaemonState.delta` for the
/// life of the daemon and never written to disk; a reconnecting client
/// re-sends it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientDeltaInfo {
    pub delta_url: String,
    pub stack_id: Uuid,
    /// The last connected signed-in mobile client's node ID.
    pub client_node_id: Uuid,
    pub host_node_id: Uuid,
}

// ---------------------------------------------------------------------------
// Public DeltaClient — holds config + signing key, exposes API methods
// ---------------------------------------------------------------------------

/// A reusable Delta API client. Load once with `DeltaClient::load()` and
/// share across calls to avoid repeated config and key reads.
pub struct DeltaClient {
    config: DeltaConfig,
    client_node_id: Option<Uuid>,
    signing_key: SigningKey,
    http: DeltaHttp,
}

impl DeltaClient {
    /// Load config and signing key from disk.
    pub fn load() -> Result<Self> {
        let config = load_config()?;
        let signing_key = load_signing_key()?;
        let http = DeltaHttp::new(&config.delta_url);
        Ok(Self {
            config,
            client_node_id: None,
            signing_key,
            http,
        })
    }

    /// Try to load from disk. Returns `None` if Delta is not configured.
    pub fn try_load() -> Option<Arc<Self>> {
        match Self::load() {
            Ok(client) => Some(Arc::new(client)),
            Err(err) => {
                tracing::debug!("Delta not configured: {err:#}");
                None
            }
        }
    }

    /// Build an anonymous client from a `ClientDeltaInfo` saved by the daemon.
    /// Uses the host's Delta signing key with the host_node_id from the info.
    /// No bearer token — authentication is via ed25519 signed request headers.
    pub fn from_client_info(info: &ClientDeltaInfo) -> Result<Arc<Self>> {
        let signing_key = load_signing_key()?;
        let config = DeltaConfig {
            delta_url: info.delta_url.clone(),
            stack_id: info.stack_id,
            node_id: info.host_node_id,
            access_token: String::new(),
            refresh_token: String::new(),
            token_expires_at: String::new(),
        };
        let http = DeltaHttp::new(&config.delta_url);
        Ok(Arc::new(Self {
            config,
            client_node_id: Some(info.client_node_id),
            signing_key,
            http,
        }))
    }

    pub fn stack_id(&self) -> Uuid {
        self.config.stack_id
    }

    pub fn host_node_id(&self) -> Uuid {
        self.config.node_id
    }

    pub fn client_node_id(&self) -> Option<Uuid> {
        self.client_node_id
    }

    /// Signed request helper — pre-binds node_id and signing_key from this client.
    async fn signed<B, T>(&self, method: &str, path: &str, body: Option<&B>) -> Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        self.http
            .signed_json(method, path, self.config.node_id, &self.signing_key, body)
            .await
    }

    pub async fn list_nodes(&self) -> Result<Vec<NodeSummary>> {
        let response: NodeListResponse = self
            .signed(
                "GET",
                &format!("/v1/stacks/{}/nodes", self.config.stack_id),
                None::<&serde_json::Value>,
            )
            .await?;
        Ok(response.nodes)
    }

    pub async fn list_node_keys(&self) -> Result<Vec<NodeKeySummary>> {
        let response: NodeKeyListResponse = self
            .signed(
                "GET",
                &format!("/v1/stacks/{}/nodes/keys", self.config.stack_id),
                None::<&serde_json::Value>,
            )
            .await?;
        Ok(response.keys)
    }

    /// Full node details (metadata, key, abilities, inbound grants).
    /// `None` targets the authenticated host node.
    pub async fn node_detail(&self, target: Option<String>) -> Result<NodeDetailResponse> {
        let node_id = match target {
            Some(target) => self.resolve_target(&target).await?,
            None => self.config.node_id,
        };
        self.signed(
            "GET",
            &format!("/v1/stacks/{}/nodes/{node_id}", self.config.stack_id),
            None::<&serde_json::Value>,
        )
        .await
    }

    pub async fn update_node(
        &self,
        target: String,
        alias: Option<String>,
        display_name: Option<String>,
    ) -> Result<NodeSummary> {
        let target_id = self.resolve_target(&target).await?;
        let response: NodeUpdateResponse = self
            .signed(
                "PATCH",
                &format!("/v1/stacks/{}/nodes/{target_id}", self.config.stack_id),
                Some(&NodeUpdateRequest {
                    alias,
                    display_name,
                }),
            )
            .await?;
        Ok(response.node)
    }

    /// Stack-level ability state, merged across nodes by each ability spec.
    pub async fn stack_ability_states(&self) -> Result<AbilityStateRollupResponse> {
        self.signed(
            "GET",
            &format!("/v1/stacks/{}/ability-states", self.config.stack_id),
            None::<&serde_json::Value>,
        )
        .await
    }

    /// Grant an ability to a node. The server derives the object from the
    /// ability's declared edge shape.
    pub async fn grant_ability(&self, target: String, ability: String) -> Result<GrantSummary> {
        let subject_id = self.resolve_target(&target).await?;
        let response: GrantCreateResponse = self
            .signed(
                "POST",
                &format!("/v1/stacks/{}/grants", self.config.stack_id),
                Some(&GrantCreateRequest {
                    subject_kind: "node",
                    subject_id,
                    ability,
                }),
            )
            .await?;
        Ok(response.grant)
    }

    /// Revoke every active grant of `ability` held by the target node.
    /// Returns the revoked grants.
    pub async fn revoke_ability(
        &self,
        target: String,
        ability: String,
    ) -> Result<Vec<GrantSummary>> {
        let subject_id = self.resolve_target(&target).await?;
        let response: GrantListResponse = self
            .signed(
                "GET",
                &format!("/v1/stacks/{}/grants", self.config.stack_id),
                None::<&serde_json::Value>,
            )
            .await?;
        let matched: Vec<GrantSummary> = response
            .grants
            .into_iter()
            .filter(|grant| {
                grant.subject_kind == "node"
                    && grant.subject_id == subject_id
                    && grant.ability == ability
            })
            .collect();
        if matched.is_empty() {
            anyhow::bail!(
                "node holds no `{ability}` grant; run `zedra stack show {target}` to list its abilities"
            );
        }
        for grant in &matched {
            let _: serde_json::Value = self
                .signed(
                    "DELETE",
                    &format!("/v1/stacks/{}/grants/{}", self.config.stack_id, grant.id),
                    None::<&serde_json::Value>,
                )
                .await?;
        }
        Ok(matched)
    }

    pub async fn delete_node(&self, target: String, force: bool) -> Result<NodeDeleteResponse> {
        let target_id = self.resolve_target(&target).await?;
        if target_id == self.config.node_id {
            anyhow::bail!("refusing to delete the authenticated host node");
        }
        let force_query = if force { "?force=true" } else { "" };
        self.signed(
            "DELETE",
            &format!(
                "/v1/stacks/{}/nodes/{target_id}{force_query}",
                self.config.stack_id
            ),
            None::<&serde_json::Value>,
        )
        .await
    }

    /// Send a push notification to a specific node (by alias or UUID).
    pub async fn send_notification(
        &self,
        target: String,
        title: String,
        body: Option<String>,
        category: Option<String>,
        deeplink: Option<String>,
    ) -> Result<NotificationSendResponse> {
        let target_node_id = Some(self.resolve_target(&target).await?);
        self.signed(
            "POST",
            &format!("/v1/stacks/{}/notifications", self.config.stack_id),
            Some(&NotificationSendRequest {
                target_node_id,
                title,
                body,
                category,
                priority: NotificationPriority::Normal,
                ttl_seconds: None,
                collapse_key: None,
                deeplink,
                data: Value::Object(Default::default()),
            }),
        )
        .await
    }

    /// Send a push notification to the last connected signed-in mobile client.
    pub async fn send_notification_to_client(
        &self,
        title: String,
        body: Option<String>,
        category: Option<String>,
        deeplink: Option<String>,
    ) -> Result<NotificationSendResponse> {
        let client_node_id = self
            .client_node_id
            .context("no previous signed-in mobile client is known for this workspace")?;
        self.signed(
            "POST",
            &format!("/v1/stacks/{}/notifications", self.config.stack_id),
            Some(&NotificationSendRequest {
                target_node_id: Some(client_node_id),
                title,
                body,
                category,
                priority: NotificationPriority::Normal,
                ttl_seconds: None,
                collapse_key: None,
                deeplink,
                data: Value::Object(Default::default()),
            }),
        )
        .await
    }

    /// Send a Live Activity update to a specific node.
    pub async fn update_live_activity(
        &self,
        target: String,
        activity_id: String,
        alert_title: Option<String>,
        alert_body: Option<String>,
        content_state: Value,
        end: bool,
    ) -> Result<LiveActivityUpdateResponse> {
        let target_node_id = Some(self.resolve_target(&target).await?);
        self.send_live_activity(
            target_node_id,
            activity_id,
            alert_title,
            alert_body,
            content_state,
            end,
        )
        .await
    }

    /// Send a Live Activity update to all nodes in this stack.
    pub async fn update_live_activity_for_stack(
        &self,
        activity_id: String,
        alert_title: Option<String>,
        alert_body: Option<String>,
        content_state: Value,
        end: bool,
    ) -> Result<LiveActivityUpdateResponse> {
        self.send_live_activity(
            None,
            activity_id,
            alert_title,
            alert_body,
            content_state,
            end,
        )
        .await
    }

    async fn send_live_activity(
        &self,
        target_node_id: Option<Uuid>,
        activity_id: String,
        alert_title: Option<String>,
        alert_body: Option<String>,
        content_state: Value,
        end: bool,
    ) -> Result<LiveActivityUpdateResponse> {
        tracing::info!(
            target = "LA",
            activity_id = %activity_id,
            node = ?target_node_id,
            end,
            "sending live activity update to delta"
        );
        let response: LiveActivityUpdateResponse = self
            .signed(
                "POST",
                &format!("/v1/stacks/{}/live-activities", self.config.stack_id),
                Some(&LiveActivityUpdateRequest {
                    target_node_id,
                    activity_id: activity_id.clone(),
                    event: if end {
                        LiveActivityEvent::End
                    } else {
                        LiveActivityEvent::Update
                    },
                    alert_title,
                    alert_body,
                    content_state,
                    stale_at: None,
                    dismissal_at: None,
                    priority: NotificationPriority::Normal,
                    ttl_seconds: Some(300),
                    collapse_key: Some(activity_id.clone()),
                }),
            )
            .await?;
        tracing::info!(
            target = "LA",
            activity_id = %activity_id,
            accepted = response.accepted,
            recipients = response.recipients,
            provider_ok = response.provider_success,
            provider_fail = response.provider_failure,
            errors = ?response.errors,
            "delta accepted live activity update"
        );
        Ok(response)
    }

    async fn resolve_target(&self, target: &str) -> Result<Uuid> {
        if let Ok(id) = Uuid::parse_str(target) {
            return Ok(id);
        }
        let response: NodeListResponse = self
            .signed(
                "GET",
                &format!("/v1/stacks/{}/nodes", self.config.stack_id),
                None::<&serde_json::Value>,
            )
            .await?;
        let normalized = normalize_alias_candidate(target);
        let matches = response
            .nodes
            .iter()
            .filter(|node| {
                node.alias.as_deref() == Some(target)
                    || normalized
                        .as_deref()
                        .is_some_and(|alias| node.alias.as_deref() == Some(alias))
                    || node
                        .display_name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(target))
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [node] => Ok(node.id),
            [] => {
                let aliases = response
                    .nodes
                    .iter()
                    .filter_map(|node| node.alias.as_deref())
                    .collect::<Vec<_>>();
                if aliases.is_empty() {
                    anyhow::bail!("unknown Delta node `{target}`; run `zedra stack list`");
                }
                anyhow::bail!(
                    "unknown Delta node `{target}`; available aliases: {}",
                    aliases.join(", ")
                );
            }
            _ => {
                anyhow::bail!("Delta node `{target}` is ambiguous; use the node id instead")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Private HTTP transport
// ---------------------------------------------------------------------------

struct DeltaHttp {
    base_url: String,
    http: reqwest::Client,
}

impl DeltaHttp {
    fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    fn origin(&self) -> &str {
        &self.base_url
    }

    async fn dev_auth(&self, req: &DevAuthRequest) -> Result<AuthResponse> {
        self.post_json("/v1/auth/dev", None, req).await
    }

    async fn cli_auth_start(&self, req: &CliAuthStartRequest) -> Result<CliAuthStartResponse> {
        self.post_json("/v1/cli/auth/start", None, req).await
    }

    async fn cli_auth_poll(&self, req: &CliAuthPollRequest) -> Result<CliAuthPollResponse> {
        self.post_json("/v1/cli/auth/poll", None, req).await
    }

    async fn refresh_auth(&self, req: &RefreshRequest) -> Result<AuthResponse> {
        self.post_json("/v1/auth/refresh", None, req).await
    }

    async fn get_bearer_optional<T>(
        &self,
        path: &str,
        access_token: &str,
    ) -> Result<BearerGetResult<T>>
    where
        T: DeserializeOwned,
    {
        let response = self
            .http
            .get(self.url(path))
            .header("accept", "application/json")
            .header("x-request-id", request_id())
            .bearer_auth(access_token)
            .send()
            .await?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(BearerGetResult::Missing),
            reqwest::StatusCode::UNAUTHORIZED => Ok(BearerGetResult::Unauthorized),
            _ => decode_response("GET", path, response)
                .await
                .map(BearerGetResult::Found),
        }
    }

    async fn register_node(
        &self,
        access_token: &str,
        stack_id: Uuid,
        req: &NodeRegistrationRequest,
    ) -> Result<NodeRegistrationResponse> {
        self.post_json(
            &format!("/v1/stacks/{stack_id}/nodes"),
            Some(access_token),
            req,
        )
        .await
    }

    async fn post_json<B, T>(&self, path: &str, access_token: Option<&str>, body: &B) -> Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let body = serde_json::to_vec(body)?;
        let mut request = self
            .http
            .post(self.url(path))
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("x-request-id", request_id())
            .body(body);
        if let Some(token) = access_token {
            request = request.bearer_auth(token);
        }
        decode_response("POST", path, request.send().await?).await
    }

    async fn signed_json<B, T>(
        &self,
        method: &str,
        path: &str,
        node_id: Uuid,
        signing_key: &SigningKey,
        body: Option<&B>,
    ) -> Result<T>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let body = match body {
            Some(body) => serde_json::to_vec(body)?,
            None => Vec::new(),
        };
        let timestamp = unix_timestamp()?;
        let canonical = canonical_node_signature_payload(method, path, timestamp, &body);
        let signature: Signature = signing_key.sign(canonical.as_bytes());
        let method = reqwest::Method::from_bytes(method.as_bytes())?;
        let request = self
            .http
            .request(method.clone(), self.url(path))
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("x-request-id", request_id())
            .header("x-delta-node-id", node_id.to_string())
            .header("x-delta-timestamp", timestamp.to_string())
            .header(
                "x-delta-signature",
                encode_base64_no_pad(signature.to_bytes()),
            )
            .body(body);
        decode_response(method.as_str(), path, request.send().await?).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

enum BearerGetResult<T> {
    Found(T),
    Missing,
    Unauthorized,
}

async fn decode_response<T>(method: &str, path: &str, response: reqwest::Response) -> Result<T>
where
    T: DeserializeOwned,
{
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{method} {path} returned HTTP {status}: {text}");
    }
    serde_json::from_str(&text).context("decode Delta JSON response")
}

fn canonical_node_signature_payload(
    method: &str,
    path_and_query: &str,
    timestamp: i64,
    body: &[u8],
) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path_and_query,
        timestamp,
        sha256_hex(body)
    )
}

fn sha256_hex(input: &[u8]) -> String {
    hex::encode(Sha256::digest(input))
}

fn encode_base64_no_pad(input: impl AsRef<[u8]>) -> String {
    BASE64_NOPAD.encode(input.as_ref())
}

fn unix_timestamp() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs() as i64)
}

fn request_id() -> String {
    format!("zedra-cli-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_signing_key_is_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SIGNING_KEY_FILE);

        let first = load_signing_key_at(&path).unwrap();
        let second = load_signing_key_at(&path).unwrap();

        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(std::fs::read(path).unwrap(), first.to_bytes());
    }

    #[test]
    fn missing_delta_config_reports_sign_in_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);

        let err = load_config_at(&path).unwrap_err();

        assert_eq!(err.to_string(), DELTA_SIGN_IN_HINT);
    }

    fn sample_info() -> ClientDeltaInfo {
        ClientDeltaInfo {
            delta_url: "https://delta.example.com".into(),
            stack_id: uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            client_node_id: uuid::Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap(),
            host_node_id: uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
        }
    }

    #[test]
    fn client_delta_info_json_roundtrip() {
        let info = sample_info();
        let json = serde_json::to_string(&info).unwrap();
        let decoded: ClientDeltaInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.delta_url, info.delta_url);
        assert_eq!(decoded.stack_id, info.stack_id);
        assert_eq!(decoded.client_node_id, info.client_node_id);
        assert_eq!(decoded.host_node_id, info.host_node_id);
    }

    #[test]
    fn canonical_payload_matches_delta_protocol() {
        let payload = canonical_node_signature_payload(
            "post",
            "/v1/stacks/abc/notifications?x=1",
            123,
            br#"{"title":"hello"}"#,
        );
        assert_eq!(
            payload,
            "POST\n/v1/stacks/abc/notifications?x=1\n123\ncf6c63ce25116b04e3b776a2957606e18d8ac798dde21e3ec30882ac2dfbe0cb"
        );
    }

    #[test]
    fn normalizes_alias_candidates_for_cli_lookup() {
        assert_eq!(
            normalize_alias_candidate("Tan iPhone!").as_deref(),
            Some("tan-iphone")
        );
        assert_eq!(normalize_alias_candidate("!!!"), None);
    }

    #[test]
    fn refreshes_only_default_host_aliases() {
        assert!(should_refresh_host_alias(
            Some("zedra-host-tanmacpro"),
            "tanmacpro"
        ));
        assert!(should_refresh_host_alias(Some("host"), "tanmacpro"));
        assert!(should_refresh_host_alias(None, "tanmacpro"));
        assert!(!should_refresh_host_alias(Some("tanmacpro"), "tanmacpro"));
        assert!(!should_refresh_host_alias(Some("tanmac"), "tanmacpro"));
    }

    #[test]
    fn host_metadata_match_ignores_server_owned_metadata() {
        let node = NodeSummary {
            id: Uuid::nil(),
            alias: None,
            kind: NodeKind::Host,
            display_name: Some("tanmacpro".into()),
            metadata: serde_json::json!({
                "host_version": "0.2.6",
                "os_version": "macOS 26.0",
                "server_owned": true,
            }),
            public_key_fingerprint: String::new(),
            push_enabled: false,
            joined_at: None,
        };

        assert!(host_metadata_matches(
            &node,
            "tanmacpro",
            &serde_json::json!({
                "host_version": "0.2.6",
                "os_version": "macOS 26.0",
            }),
        ));
    }

    #[test]
    fn host_metadata_match_detects_version_changes() {
        let node = NodeSummary {
            id: Uuid::nil(),
            alias: None,
            kind: NodeKind::Host,
            display_name: Some("tanmacpro".into()),
            metadata: serde_json::json!({ "host_version": "0.2.5" }),
            public_key_fingerprint: String::new(),
            push_enabled: false,
            joined_at: None,
        };

        assert!(!host_metadata_matches(
            &node,
            "tanmacpro",
            &serde_json::json!({ "host_version": "0.2.6" }),
        ));
    }
}
