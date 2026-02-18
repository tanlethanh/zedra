use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;

use crate::types::{
    CreateRoomResponse, JoinRoomResponse, RecvResponse, SendRequest, SendResponse, SignalRequest,
    SignalResponse,
};

/// HTTP client for the relay server API.
///
/// Provides typed methods for all relay endpoints:
/// - Room management: create, join, heartbeat, delete
/// - Messaging: send (base64 payloads), recv (with sequence tracking)
/// - Signaling: set/get out-of-band signal data
pub struct RelayClient {
    http: Client,
    relay_url: String,
    room_code: String,
    secret: String,
    role: String,
}

impl RelayClient {
    /// Create a client for an existing room.
    pub fn new(relay_url: String, room_code: String, secret: String, role: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");
        Self {
            http,
            relay_url,
            room_code,
            secret,
            role,
        }
    }

    /// POST /rooms - Create a new room. No auth required.
    /// Returns the room code and secret.
    pub async fn create_room(relay_url: &str) -> Result<CreateRoomResponse> {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let resp = http
            .post(format!("{}/rooms", relay_url))
            .send()
            .await
            .context("create_room request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("create_room failed ({}): {}", status, body);
        }
        resp.json().await.context("create_room: invalid response")
    }

    /// POST /rooms/:code/join - Join an existing room.
    pub async fn join_room(&self) -> Result<JoinRoomResponse> {
        let resp = self
            .http
            .post(format!("{}/rooms/{}/join", self.relay_url, self.room_code))
            .bearer_auth(&self.secret)
            .send()
            .await
            .context("join_room request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("join_room failed ({}): {}", status, body);
        }
        resp.json().await.context("join_room: invalid response")
    }

    /// POST /rooms/:code/send - Send base64-encoded messages.
    pub async fn send_messages(&self, messages: &[String]) -> Result<SendResponse> {
        let body = SendRequest {
            role: self.role.clone(),
            messages: messages.to_vec(),
        };
        let resp = self
            .http
            .post(format!("{}/rooms/{}/send", self.relay_url, self.room_code))
            .bearer_auth(&self.secret)
            .json(&body)
            .send()
            .await
            .context("send_messages request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("send_messages failed ({}): {}", status, body);
        }
        resp.json()
            .await
            .context("send_messages: invalid response")
    }

    /// GET /rooms/:code/recv?role=X&after=N - Receive messages after a sequence number.
    pub async fn recv_messages(&self, after: u64) -> Result<RecvResponse> {
        let resp = self
            .http
            .get(format!("{}/rooms/{}/recv", self.relay_url, self.room_code))
            .bearer_auth(&self.secret)
            .query(&[("role", &self.role), ("after", &after.to_string())])
            .send()
            .await
            .context("recv_messages request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("recv_messages failed ({}): {}", status, body);
        }
        resp.json()
            .await
            .context("recv_messages: invalid response")
    }

    /// POST /rooms/:code/signal - Set signaling data for this role.
    pub async fn set_signal(&self, data: serde_json::Value) -> Result<()> {
        let body = SignalRequest {
            role: self.role.clone(),
            data,
        };
        let resp = self
            .http
            .post(format!(
                "{}/rooms/{}/signal",
                self.relay_url, self.room_code
            ))
            .bearer_auth(&self.secret)
            .json(&body)
            .send()
            .await
            .context("set_signal request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("set_signal failed ({}): {}", status, body);
        }
        Ok(())
    }

    /// GET /rooms/:code/signal?role=X - Get peer's signaling data.
    pub async fn get_signal(&self) -> Result<SignalResponse> {
        let resp = self
            .http
            .get(format!(
                "{}/rooms/{}/signal",
                self.relay_url, self.room_code
            ))
            .bearer_auth(&self.secret)
            .query(&[("role", &self.role)])
            .send()
            .await
            .context("get_signal request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("get_signal failed ({}): {}", status, body);
        }
        resp.json().await.context("get_signal: invalid response")
    }

    /// POST /rooms/:code/heartbeat - Keep room alive.
    pub async fn heartbeat(&self) -> Result<()> {
        let resp = self
            .http
            .post(format!(
                "{}/rooms/{}/heartbeat",
                self.relay_url, self.room_code
            ))
            .bearer_auth(&self.secret)
            .send()
            .await
            .context("heartbeat request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("heartbeat failed ({}): {}", status, body);
        }
        Ok(())
    }

    /// DELETE /rooms/:code - Delete the room.
    pub async fn delete_room(&self) -> Result<()> {
        let resp = self
            .http
            .delete(format!("{}/rooms/{}", self.relay_url, self.room_code))
            .bearer_auth(&self.secret)
            .send()
            .await
            .context("delete_room request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("delete_room failed ({}): {}", status, body);
        }
        Ok(())
    }

    pub fn room_code(&self) -> &str {
        &self.room_code
    }

    pub fn role(&self) -> &str {
        &self.role
    }
}
