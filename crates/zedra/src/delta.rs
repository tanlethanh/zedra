use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;
use zedra_session::signer::ClientSigner;

use crate::platform_bridge;

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

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum NodeKind {
    Mobile,
    Host,
}

#[derive(Deserialize)]
struct NodeRegistrationResponse {
    node: NodeSummary,
}

#[derive(Deserialize)]
struct NodeSummary {
    id: Uuid,
    #[serde(default)]
    alias: Option<String>,
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
    pub mobile_node_id: Option<Uuid>,
    pub push_provider: Option<String>,
    pub push_environment: Option<String>,
    pub push_registered: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct DeltaState {
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
    mobile_node_id: Option<Uuid>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    push_token: Option<StoredPushToken>,
}

#[derive(Clone, Serialize, Deserialize)]
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
            mobile_node_id: None,
            email: None,
            push_token: None,
        }
    }
}

impl DeltaState {
    fn status(&self) -> DeltaStatus {
        DeltaStatus {
            base_url: self.base_url.clone(),
            signed_in: self.access_token.is_some() && self.stack_id.is_some(),
            email: self.email.clone(),
            stack_id: self.stack_id,
            mobile_node_id: self.mobile_node_id,
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
        }
    }
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

pub fn status() -> DeltaStatus {
    load_state().unwrap_or_default().status()
}

pub async fn sign_in_with_google(id_token: String, email: Option<String>) -> Result<DeltaStatus> {
    let mut state = load_state().unwrap_or_default();
    state.base_url = normalize_base_url(&state.base_url);

    let auth: AuthResponse = http()
        .post(format!("{}/v1/auth/oauth/google", state.base_url))
        .json(&OAuthRequest { id_token })
        .send()
        .await
        .context("send Google OAuth request to Delta")?
        .error_for_status()
        .context("Delta rejected Google OAuth token")?
        .json()
        .await
        .context("decode Delta OAuth response")?;

    state.access_token = Some(auth.access_token);
    state.refresh_token = Some(auth.refresh_token);
    state.expires_at = Some(auth.expires_at);
    state.user_id = Some(auth.user.id);
    state.stack_id = Some(auth.stack.id);
    state.email = email.or(state.email);
    save_state(&state)?;

    let signer = load_mobile_signer()?;
    let mobile_name = mobile_display_name();
    let mobile = register_mobile_node(&mut state, signer.pubkey(), &mobile_name).await?;
    state.mobile_node_id = Some(mobile.node.id);
    save_state(&state)?;
    if should_refresh_mobile_alias(mobile.node.alias.as_deref(), &mobile_name) {
        if let Err(err) = update_mobile_alias(&mut state, mobile.node.id, &mobile_name).await {
            tracing::warn!("Delta mobile node alias update failed: {err:#}");
        }
    }

    if state.push_token.is_some() {
        if let Err(err) = register_stored_push_token(&mut state).await {
            tracing::warn!("Delta push token registration after sign-in failed: {err:#}");
            save_state(&state)?;
        }
    }

    Ok(state.status())
}

pub async fn register_push_token(
    provider: String,
    token: String,
    environment: Option<String>,
) -> Result<DeltaStatus> {
    let mut state = load_state().unwrap_or_default();
    state.push_token = Some(StoredPushToken {
        provider,
        token,
        environment,
        registered: false,
    });

    if state.access_token.is_some() && state.stack_id.is_some() && state.mobile_node_id.is_some() {
        register_stored_push_token(&mut state).await?;
    }

    save_state(&state)?;
    Ok(state.status())
}

pub async fn register_paired_host_node(public_key: [u8; 32], metadata: Value) -> Result<bool> {
    let mut state = load_state().unwrap_or_default();
    if state.access_token.is_none() {
        return Ok(false);
    }
    let Some(stack_id) = state.stack_id else {
        return Ok(false);
    };

    let req = NodeRegistrationRequest {
        public_key: encode_base64_no_pad(public_key),
        kind: NodeKind::Host,
        display_name: host_display_name(&metadata),
        metadata,
        receive_notifications: false,
    };
    post_bearer::<_, NodeRegistrationResponse>(
        &mut state,
        &format!("/v1/stacks/{stack_id}/nodes"),
        &req,
    )
    .await?;
    Ok(true)
}

async fn register_mobile_node(
    state: &mut DeltaState,
    public_key: [u8; 32],
    display_name: &str,
) -> Result<NodeRegistrationResponse> {
    let stack_id = state.stack_id.context("Delta stack id is missing")?;
    let req = NodeRegistrationRequest {
        public_key: encode_base64_no_pad(public_key),
        kind: NodeKind::Mobile,
        display_name: Some(display_name.to_string()),
        metadata: json!({
            "device_name": display_name,
            "platform": "ios",
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
            "app_version": platform_bridge::app_version_with_build_number(),
        }),
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
    let node_id = state
        .mobile_node_id
        .context("Delta mobile node id is missing")?;
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
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("zedra-ios/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("valid Delta HTTP client")
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
    platform_bridge::device_name().unwrap_or_else(|| "zedra-ios".to_string())
}

fn should_refresh_mobile_alias(current_alias: Option<&str>, display_name: &str) -> bool {
    let Some(display_alias) = normalize_alias_candidate(display_name) else {
        return false;
    };
    match current_alias {
        None => true,
        Some(alias) if alias == display_alias => false,
        Some("mobile" | "zedra-ios") => display_alias != "mobile" && display_alias != "zedra-ios",
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

fn load_state() -> Result<DeltaState> {
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
    write_private_file(&path, &bytes).with_context(|| format!("write {}", path.display()))
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
    use super::{normalize_alias_candidate, should_refresh_mobile_alias};

    #[test]
    fn normalizes_alias_candidates_for_mobile_names() {
        assert_eq!(
            normalize_alias_candidate("Tan's iPhone 15 Pro").as_deref(),
            Some("tan-s-iphone-15-pro")
        );
        assert_eq!(normalize_alias_candidate("!!!"), None);
    }

    #[test]
    fn refreshes_only_default_mobile_aliases() {
        assert!(should_refresh_mobile_alias(Some("zedra-ios"), "Tan iPhone"));
        assert!(should_refresh_mobile_alias(Some("mobile"), "Tan iPhone"));
        assert!(!should_refresh_mobile_alias(
            Some("tan-iphone"),
            "Tan iPhone"
        ));
        assert!(!should_refresh_mobile_alias(
            Some("custom-phone"),
            "Tan iPhone"
        ));
        assert!(!should_refresh_mobile_alias(Some("zedra-ios"), "zedra-ios"));
    }
}
