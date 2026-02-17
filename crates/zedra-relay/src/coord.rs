// Coordination Server Client
//
// HTTP client for the host registry API. Used by:
// - zedra-host: register + heartbeat loop
// - zedra-transport: host lookup + signaling for reconnection

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Address entry in a host registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAddress {
    #[serde(rename = "type")]
    pub addr_type: String,
    pub addr: String,
}

/// Session entry in a host registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSession {
    pub id: String,
    pub name: String,
    pub workdir: String,
}

/// POST /hosts/register request body.
#[derive(Debug, Clone, Serialize)]
pub struct RegisterRequest {
    pub device_id: String,
    pub public_key: String,
    pub hostname: String,
    pub addresses: Vec<HostAddress>,
    pub sessions: Vec<HostSession>,
    pub capabilities: Vec<String>,
    pub version: String,
}

/// POST /hosts/register response.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterResponse {
    pub ttl: u64,
    pub relay_endpoint: String,
}

/// POST /hosts/:device_id/heartbeat request body.
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<HostAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<HostSession>>,
}

/// POST /hosts/:device_id/heartbeat response.
#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatResponse {
    pub ttl: u64,
}

/// GET /hosts/:device_id response.
#[derive(Debug, Clone, Deserialize)]
pub struct HostLookupResponse {
    pub online: bool,
    pub last_seen: String,
    pub hostname: String,
    pub addresses: Vec<HostAddress>,
    pub sessions: Vec<HostSession>,
    pub capabilities: Vec<String>,
    pub relay_endpoint: String,
}

/// Connection candidate for signaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionCandidate {
    #[serde(rename = "type")]
    pub candidate_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub priority: u32,
}

/// POST /signal/:device_id request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalCandidates {
    pub from_device_id: String,
    pub candidates: Vec<ConnectionCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// GET /signal/:device_id response.
#[derive(Debug, Clone, Deserialize)]
pub struct SignalResponse {
    pub signals: Vec<SignalCandidates>,
}

/// Client for the coordination server API.
pub struct CoordClient {
    http: reqwest::Client,
    base_url: String,
}

impl CoordClient {
    /// Create a new coordination client.
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Register a host with the coordination server.
    pub async fn register(&self, req: &RegisterRequest) -> Result<RegisterResponse> {
        let url = format!("{}/hosts/register", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("register failed ({}): {}", status, body);
        }

        Ok(resp.json().await?)
    }

    /// Send heartbeat to keep registration alive.
    pub async fn heartbeat(
        &self,
        device_id: &str,
        req: &HeartbeatRequest,
    ) -> Result<HeartbeatResponse> {
        let url = format!("{}/hosts/{}/heartbeat", self.base_url, device_id);
        let resp = self
            .http
            .post(&url)
            .json(req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("heartbeat failed ({}): {}", status, body);
        }

        Ok(resp.json().await?)
    }

    /// Look up a host by device ID.
    pub async fn lookup(&self, device_id: &str) -> Result<HostLookupResponse> {
        let url = format!("{}/hosts/{}", self.base_url, device_id);
        let resp = self.http.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("lookup failed ({}): {}", status, body);
        }

        Ok(resp.json().await?)
    }

    /// Send connection candidates to a target device.
    pub async fn signal(
        &self,
        target_device_id: &str,
        candidates: &SignalCandidates,
    ) -> Result<()> {
        let url = format!("{}/signal/{}", self.base_url, target_device_id);
        let resp = self
            .http
            .post(&url)
            .json(candidates)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("signal failed ({}): {}", status, body);
        }

        Ok(())
    }

    /// Drain pending signals for a device.
    pub async fn drain_signals(&self, device_id: &str) -> Result<Vec<SignalCandidates>> {
        let url = format!("{}/signal/{}", self.base_url, device_id);
        let resp = self.http.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("drain_signals failed ({}): {}", status, body);
        }

        let result: SignalResponse = resp.json().await?;
        Ok(result.signals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_serialization() {
        let req = RegisterRequest {
            device_id: "TEST-DEVICE".to_string(),
            public_key: "base64url_key".to_string(),
            hostname: "my-laptop".to_string(),
            addresses: vec![HostAddress {
                addr_type: "lan".to_string(),
                addr: "192.168.1.100:2123".to_string(),
            }],
            sessions: vec![HostSession {
                id: "uuid-1".to_string(),
                name: "zedra".to_string(),
                workdir: "/home/user/zedra".to_string(),
            }],
            capabilities: vec!["terminal".to_string(), "fs".to_string()],
            version: "0.2.0".to_string(),
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("TEST-DEVICE"));
        assert!(json.contains("192.168.1.100:2123"));
        assert!(json.contains("terminal"));
    }

    #[test]
    fn lookup_response_deserialization() {
        let json = r#"{
            "online": true,
            "last_seen": "2026-02-16T10:30:00Z",
            "hostname": "my-laptop",
            "addresses": [{"type": "lan", "addr": "192.168.1.100:2123"}],
            "sessions": [{"id": "uuid-1", "name": "zedra", "workdir": "/home/user"}],
            "capabilities": ["terminal"],
            "relay_endpoint": "wss://relay.zedra.dev"
        }"#;

        let resp: HostLookupResponse = serde_json::from_str(json).unwrap();
        assert!(resp.online);
        assert_eq!(resp.hostname, "my-laptop");
        assert_eq!(resp.addresses.len(), 1);
        assert_eq!(resp.addresses[0].addr_type, "lan");
    }

    #[test]
    fn signal_candidates_roundtrip() {
        let signal = SignalCandidates {
            from_device_id: "CLIENT-123".to_string(),
            candidates: vec![ConnectionCandidate {
                candidate_type: "direct-lan".to_string(),
                addr: Some("192.168.1.50:45678".to_string()),
                url: None,
                priority: 0,
            }],
            session_id: Some("uuid-1".to_string()),
        };

        let json = serde_json::to_string(&signal).unwrap();
        let parsed: SignalCandidates = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.from_device_id, "CLIENT-123");
        assert_eq!(parsed.candidates[0].priority, 0);
    }
}
