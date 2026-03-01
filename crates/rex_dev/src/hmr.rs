use axum::extract::ws::{Message, WebSocket};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum HmrMessage {
    #[serde(rename = "connected")]
    Connected,
    #[serde(rename = "update")]
    Update {
        path: String,
        timestamp: u64,
        /// Serialized manifest ({ build_id, pages }) for the client to hot-swap chunks
        manifest: serde_json::Value,
    },
    #[serde(rename = "full-reload")]
    FullReload,
    #[serde(rename = "error")]
    Error {
        message: String,
        file: Option<String>,
    },
    #[serde(rename = "tsc-error")]
    TscError { errors: Vec<TscDiagnostic> },
    #[serde(rename = "tsc-clear")]
    TscClear {},
}

#[derive(Debug, Clone, Serialize)]
pub struct TscDiagnostic {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub code: String,
    pub message: String,
}

/// Broadcast channel for HMR messages
#[derive(Clone)]
pub struct HmrBroadcast {
    tx: broadcast::Sender<HmrMessage>,
}

impl Default for HmrBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

impl HmrBroadcast {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    pub fn send_update(&self, path: &str, manifest: serde_json::Value) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_millis() as u64;

        let _ = self.tx.send(HmrMessage::Update {
            path: path.to_string(),
            timestamp,
            manifest,
        });
    }

    pub fn send_full_reload(&self) {
        let _ = self.tx.send(HmrMessage::FullReload);
    }

    pub fn send_error(&self, message: &str, file: Option<&str>) {
        let _ = self.tx.send(HmrMessage::Error {
            message: message.to_string(),
            file: file.map(|s| s.to_string()),
        });
    }

    pub fn send_tsc_errors(&self, errors: Vec<TscDiagnostic>) {
        let _ = self.tx.send(HmrMessage::TscError { errors });
    }

    pub fn send_tsc_clear(&self) {
        let _ = self.tx.send(HmrMessage::TscClear {});
    }
}

pub async fn handle_hmr_socket(mut socket: WebSocket, hmr: HmrBroadcast) {
    info!("HMR client connected");

    // Send connected message
    let connected =
        serde_json::to_string(&HmrMessage::Connected).expect("HmrMessage serialization");
    if socket.send(Message::Text(connected.into())).await.is_err() {
        return;
    }

    let mut rx = hmr.tx.subscribe();

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(hmr_msg) => {
                        let json = serde_json::to_string(&hmr_msg).expect("HmrMessage serialization");
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!("HMR client lagged by {n} messages, sending full reload");
                        let json = serde_json::to_string(&HmrMessage::FullReload).expect("HmrMessage serialization");
                        let _ = socket.send(Message::Text(json.into())).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(_)) => {} // Ignore client messages
                    _ => break, // Client disconnected
                }
            }
        }
    }

    info!("HMR client disconnected");
}
