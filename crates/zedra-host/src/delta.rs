use anyhow::{Context, Result};
use data_encoding::BASE64_NOPAD;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use crate::identity;

const CONFIG_FILE: &str = "delta.json";

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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Mobile,
    Host,
    Agent,
    External,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mobile => "mobile",
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveActivityUpdateResponse {
    pub accepted: bool,
    pub recipients: u32,
    pub provider_success: u32,
    pub provider_failure: u32,
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

#[derive(Debug, Serialize)]
struct NodeUpdateRequest {
    alias: Option<String>,
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
    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("Delta auth config not found at {}", path.display()))?;
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
    let identity = identity::HostIdentity::load_or_generate()?;
    Ok(SigningKey::from_bytes(
        &identity.iroh_secret_key().to_bytes(),
    ))
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
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "zedra_version": env!("CARGO_PKG_VERSION"),
    })
}

fn default_hostname() -> Option<String> {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

// ---------------------------------------------------------------------------
// Public DeltaClient — holds config + signing key, exposes API methods
// ---------------------------------------------------------------------------

/// A reusable Delta API client. Load once with `DeltaClient::load()` and
/// share across calls to avoid repeated config and key reads.
pub struct DeltaClient {
    config: DeltaConfig,
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

    pub async fn update_node_alias(&self, target: String, alias: String) -> Result<NodeSummary> {
        let target_id = self.resolve_target(&target).await?;
        let response: NodeUpdateResponse = self
            .signed(
                "PATCH",
                &format!("/v1/stacks/{}/nodes/{target_id}", self.config.stack_id),
                Some(&NodeUpdateRequest { alias: Some(alias) }),
            )
            .await?;
        Ok(response.node)
    }

    pub async fn delete_node(&self, target: String) -> Result<NodeDeleteResponse> {
        let target_id = self.resolve_target(&target).await?;
        if target_id == self.config.node_id {
            anyhow::bail!("refusing to delete the authenticated host node");
        }
        self.signed(
            "DELETE",
            &format!("/v1/stacks/{}/nodes/{target_id}", self.config.stack_id),
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

    /// Broadcast a push notification to all nodes in this stack.
    pub async fn send_notification_to_stack(
        &self,
        title: String,
        body: Option<String>,
        category: Option<String>,
        deeplink: Option<String>,
    ) -> Result<NotificationSendResponse> {
        self.signed(
            "POST",
            &format!("/v1/stacks/{}/notifications", self.config.stack_id),
            Some(&NotificationSendRequest {
                target_node_id: None,
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
                    anyhow::bail!("unknown Delta node alias `{target}`; run `zedra stack nodes`");
                }
                anyhow::bail!(
                    "unknown Delta node alias `{target}`; available aliases: {}",
                    aliases.join(", ")
                );
            }
            _ => {
                anyhow::bail!("Delta node alias `{target}` is ambiguous; use the node id instead")
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
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
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
}
