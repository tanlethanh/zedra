use anyhow::{Context, Result};
use data_encoding::BASE64_NOPAD;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
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
    pub kind: NodeKind,
    pub display_name: Option<String>,
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
    let client = DeltaClient::new(delta_url);
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
                display_name: Some(default_host_display_name()),
                metadata: serde_json::json!({
                    "source": "zedra_cli",
                    "hostname": hostname::get().ok().and_then(|name| name.into_string().ok()),
                }),
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
    Ok(config)
}

pub async fn start_browser_auth(delta_url: &str) -> Result<CliAuthSession> {
    let config_dir = identity::host_config_dir()?;
    std::fs::create_dir_all(&config_dir)?;
    let signing_key = load_signing_key()?;
    let client = DeltaClient::new(delta_url);
    let started = client
        .cli_auth_start(&CliAuthStartRequest {
            public_key: encode_base64_no_pad(signing_key.verifying_key().to_bytes()),
            display_name: Some(default_host_display_name()),
            metadata: serde_json::json!({
                "source": "zedra_cli",
                "hostname": hostname::get().ok().and_then(|name| name.into_string().ok()),
            }),
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
    let client = DeltaClient::new(&session.delta_url);
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
                let config = DeltaConfig {
                    delta_url: session.delta_url.clone(),
                    stack_id,
                    node_id,
                    access_token,
                    refresh_token,
                    token_expires_at: expires_at,
                };
                save_config(&config)?;
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

pub async fn list_nodes() -> Result<Vec<NodeSummary>> {
    let config = load_config()?;
    let signing_key = load_signing_key()?;
    let client = DeltaClient::new(&config.delta_url);
    let response = client
        .list_nodes_signed(config.stack_id, config.node_id, &signing_key)
        .await?;
    Ok(response.nodes)
}

pub async fn send_notification(
    target_node_id: Uuid,
    title: String,
    body: Option<String>,
    category: Option<String>,
    deeplink: Option<String>,
) -> Result<NotificationSendResponse> {
    let config = load_config()?;
    let signing_key = load_signing_key()?;
    let client = DeltaClient::new(&config.delta_url);
    client
        .send_notification_signed(
            config.stack_id,
            config.node_id,
            &signing_key,
            &NotificationSendRequest {
                target_node_id: Some(target_node_id),
                title,
                body,
                category,
                priority: NotificationPriority::Normal,
                ttl_seconds: None,
                collapse_key: None,
                deeplink,
                data: Value::Object(Default::default()),
            },
        )
        .await
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
    match hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
    {
        Some(name) if !name.is_empty() => format!("Zedra host ({name})"),
        _ => "Zedra host".to_string(),
    }
}

struct DeltaClient {
    base_url: String,
    http: reqwest::Client,
}

impl DeltaClient {
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

    async fn list_nodes_signed(
        &self,
        stack_id: Uuid,
        node_id: Uuid,
        signing_key: &SigningKey,
    ) -> Result<NodeListResponse> {
        self.signed_json::<serde_json::Value, _>(
            "GET",
            &format!("/v1/stacks/{stack_id}/nodes"),
            node_id,
            signing_key,
            None,
        )
        .await
    }

    async fn send_notification_signed(
        &self,
        stack_id: Uuid,
        node_id: Uuid,
        signing_key: &SigningKey,
        req: &NotificationSendRequest,
    ) -> Result<NotificationSendResponse> {
        self.signed_json(
            "POST",
            &format!("/v1/stacks/{stack_id}/notifications"),
            node_id,
            signing_key,
            Some(req),
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
}
