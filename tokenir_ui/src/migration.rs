use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use rmp_serde::{decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest, tungstenite::protocol::Message,
};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatorHistory {
    #[serde(flatten)]
    pub counts: Migrated,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Migrated {
    #[serde(alias = "tokensCreated")]
    pub total_count: u64,
    #[serde(alias = "tokensBonded")]
    pub migrated_count: u64,
}

pub struct PadreClient {
    tx: mpsc::Sender<Message>,
    pending_requests: Arc<DashMap<u32, oneshot::Sender<CreatorHistory>>>,
    next_seq: Arc<AtomicU32>,
}

impl PadreClient {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, mut rx) = mpsc::channel::<Message>(100);

        // Fix: Explicit type annotation
        let pending_requests: Arc<DashMap<u32, oneshot::Sender<CreatorHistory>>> =
            Arc::new(DashMap::new());

        let next_seq = Arc::new(AtomicU32::new(1000));
        let pending_clone = pending_requests.clone();
        let loop_tx = tx.clone();

        tokio::spawn(async move {
            let cookie = env::var("PADRE_COOKIE").unwrap_or_default();
            let url = "wss://backend.padre.gg/_heavy_multiplex?desc=%2Ftrade%2Fsolana%2F3f2e2jJ7H5anAQkc1t7qYfapZnd4WbdUavJgBwtUfC3J";

            loop {
                let mut request = match url.into_client_request() {
                    Ok(r) => r,
                    Err(_) => {
                        sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };

                let headers = request.headers_mut();
                headers.insert("User-Agent", "Mozilla/5.0".parse().unwrap());
                headers.insert("Origin", "https://trade.padre.gg".parse().unwrap());
                headers.insert("Cookie", cookie.parse().unwrap());

                if let Ok((ws_stream, _)) = connect_async(request).await {
                    let (mut ws_writer, mut ws_reader) = ws_stream.split();
                    loop {
                        tokio::select! {
                            Some(msg) = rx.recv() => {
                                if ws_writer.send(msg).await.is_err() { break; }
                            }
                            msg_res = ws_reader.next() => {
                                match msg_res {
                                    Some(Ok(Message::Binary(bin))) => {
                                        if bin.len() <= 2 {
                                            let _ = loop_tx.send(Message::Binary(bin)).await;
                                            continue;
                                        }
                                        if let Ok(raw_array) = decode::from_slice::<Vec<Value>>(&bin) {
                                            if raw_array.len() >= 4 {
                                                if let Some(seq) = raw_array[1].as_u64() {
                                                    let seq_u32 = seq as u32;
                                                    if let Some((_, sender)) = pending_clone.remove(&seq_u32) {
                                                        if let Ok(history) = serde_json::from_value::<CreatorHistory>(raw_array[3].clone()) {
                                                            let _ = sender.send(history);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => break,
                                }
                            }
                        }
                    }
                }
                sleep(Duration::from_secs(1)).await; // Wait before reconnecting
            }
        });

        Ok(Self {
            tx,
            pending_requests,
            next_seq,
        })
    }

    pub async fn get_dev_history(&self, dev_address: &str) -> Option<CreatorHistory> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let route = format!("/dev-tokens/chain/SOLANA/dev/{}/get-dev-stats", dev_address);
        let request_id = Uuid::new_v4().to_string();

        let (otx, orx) = oneshot::channel();
        self.pending_requests.insert(seq, otx);

        let payload = (8, seq, route, request_id);
        let mut buf = Vec::new();

        if encode::write(&mut buf, &payload).is_ok() {
            // This will buffer messages even if the connection is currently down
            if self.tx.send(Message::Binary(buf)).await.is_err() {
                return None;
            }
        }

        // Increased timeout slightly to allow for reconnection time
        tokio::time::timeout(Duration::from_secs(5), orx)
            .await
            .ok()?
            .ok()
    }
}
