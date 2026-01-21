use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokenir_ui::Token;
use tokio_tungstenite::connect_async;

pub struct Client {
    url: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "connection_info")]
    ConnectionInfo { autobuy: bool, message: String },
    #[serde(rename = "NewToken")]
    NewToken { data: Token },
}

impl Client {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub async fn subscribe<F, Fut>(&self, mut __func__: F) -> Result<(), std::io::Error>
    where
        F: FnMut(Token, bool) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut autobuy = false; // Store autobuy status

        loop {
            let ws_stream = match connect_async(&self.url).await {
                Ok((stream, _)) => {
                    println!("[client] Connected to WebSocket");
                    stream
                }
                Err(e) => {
                    eprintln!("[client] Connection failed: {}, retrying in 5s...", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (_, mut __read__) = ws_stream.split();

            while let Some(msg) = __read__.next().await {
                let msg: tokio_tungstenite::tungstenite::Message = match msg {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("[client] Message error: {}", err);
                        continue;
                    }
                };

                let text = msg.to_string();

                // Try to parse as ServerMessage first
                match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(ServerMessage::ConnectionInfo {
                        autobuy: ab,
                        message,
                    }) => {
                        autobuy = ab;
                        println!("[client] {} (autobuy: {})", message, autobuy);
                    }
                    Ok(ServerMessage::NewToken { data }) => {
                        __func__(data, autobuy).await;
                    }
                    Err(_) => {
                        // Fallback: try parsing as Token directly (for backward compatibility)
                        match serde_json::from_str::<Token>(&text) {
                            Ok(token) => __func__(token, autobuy).await,
                            Err(err) => {
                                eprintln!("[client] Failed to parse message: {}", err);
                            }
                        }
                    }
                }
            }

            eprintln!("[client] Connection closed, reconnecting in 5s...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}
