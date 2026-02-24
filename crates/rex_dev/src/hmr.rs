use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
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
    },
    #[serde(rename = "full-reload")]
    FullReload,
    #[serde(rename = "error")]
    Error {
        message: String,
        file: Option<String>,
    },
}

/// Broadcast channel for HMR messages
#[derive(Clone)]
pub struct HmrBroadcast {
    tx: broadcast::Sender<HmrMessage>,
}

impl HmrBroadcast {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HmrMessage> {
        self.tx.subscribe()
    }

    pub fn send_update(&self, path: &str) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let _ = self.tx.send(HmrMessage::Update {
            path: path.to_string(),
            timestamp,
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
}

/// WebSocket handler for HMR
pub async fn hmr_websocket_handler(
    ws: WebSocketUpgrade,
    State(hmr): State<HmrBroadcast>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_hmr_socket(socket, hmr))
}

pub async fn handle_hmr_socket(mut socket: WebSocket, hmr: HmrBroadcast) {
    info!("HMR client connected");

    // Send connected message
    let connected = serde_json::to_string(&HmrMessage::Connected).unwrap();
    if socket.send(Message::Text(connected.into())).await.is_err() {
        return;
    }

    let mut rx = hmr.subscribe();

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(hmr_msg) => {
                        let json = serde_json::to_string(&hmr_msg).unwrap();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        debug!("HMR client lagged by {n} messages, sending full reload");
                        let json = serde_json::to_string(&HmrMessage::FullReload).unwrap();
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
