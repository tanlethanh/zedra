use serde::{Deserialize, Serialize};

/// Response from POST /rooms
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRoomResponse {
    pub code: String,
    pub secret: String,
}

/// Response from POST /rooms/:code/join
#[derive(Debug, Serialize, Deserialize)]
pub struct JoinRoomResponse {
    pub joined: bool,
    #[serde(rename = "mobileId")]
    pub mobile_id: String,
}

/// Request body for POST /rooms/:code/send
#[derive(Debug, Serialize)]
pub struct SendRequest {
    pub role: String,
    pub messages: Vec<String>,
}

/// Response from POST /rooms/:code/send
#[derive(Debug, Serialize, Deserialize)]
pub struct SendResponse {
    pub sent: usize,
    pub seq: u64,
}

/// Response from GET /rooms/:code/recv
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecvResponse {
    pub messages: Vec<RelayMessage>,
    pub last_seq: u64,
}

/// A single message from the relay
#[derive(Debug, Serialize, Deserialize)]
pub struct RelayMessage {
    pub seq: u64,
    pub data: String,
}

/// Request body for POST /rooms/:code/signal
#[derive(Debug, Serialize)]
pub struct SignalRequest {
    pub role: String,
    pub data: serde_json::Value,
}

/// Response from GET /rooms/:code/signal
#[derive(Debug, Serialize, Deserialize)]
pub struct SignalResponse {
    pub data: Option<serde_json::Value>,
}
