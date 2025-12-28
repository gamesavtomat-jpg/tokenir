use std::{future::Future, time::Duration};
use borsh::BorshDeserialize;
use futures::{SinkExt, StreamExt};
use serde_json::from_str;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message},
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use crate::{
    constans::{
        self,
        helper::{calc_price_impact, pool_pda},
    },
    logs::{
        BuyEvent, BuyEventAMM, CreateEvent, CreateEventV2, Event, SellEvent, SellEventAMM,
        TradeEvent,
    },
    requests::LogsNotification,
};

pub struct Client {
    url: String,
}

use chrono::Local;

// Inline for zero-cost abstraction
#[inline(always)]
fn ts(step: &str) {
    println!("[{}] {}", Local::now().format("%H:%M:%S"), step);
}

impl Client {
    #[inline]
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub async fn subscribe_to_pump<F, Fut>(&self, func: F, amm: bool) -> Result<(), Error>
    where
        F: FnMut((Duration, Event)) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let pump_handle = {
            let func = func.clone();
            let url = self.url.clone();
            tokio::spawn(async move {
                Client::subscribe_to_websocket(
                    url,
                    constans::requests::SUBSCRIBE_REQUEST_PUMP,
                    func,
                )
                .await
            })
        };

        let amm_handle = if amm {
            let url = self.url.clone();
            let func = func.clone();
            Some(tokio::spawn(async move {
                Client::subscribe_to_websocket(
                    url,
                    constans::requests::SUBSCRIBE_REQUEST_AMM,
                    func,
                )
                .await
            }))
        } else {
            None
        };

        tokio::select! {
            _ = pump_handle => {},
            _ = async {
                if let Some(h) = amm_handle {
                    let _ = h.await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {},
        }
        Ok(())
    }

    async fn subscribe_to_websocket<F, Fut>(
        url: String,
        subscription_request: &'static str,
        mut func: F,
    ) -> Result<(), Error>
    where
        F: FnMut((Duration, Event)) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        // Pre-allocate buffer for base64 decoding (reuse across iterations)
        let mut decode_buf = Vec::with_capacity(512);
        
        loop {
            ts(&format!("Connecting to WebSocket ({})...", subscription_request));

            let ws_stream = match connect_async(&url).await {
                Ok((stream, _)) => {
                    ts(&format!("Connected ({}).", subscription_request));
                    stream
                }
                Err(e) => {
                    eprintln!(
                        "[{}] Connection failed ({}): {}. Retrying in 5s...",
                        Local::now().format("%H:%M:%S"),
                        subscription_request,
                        e
                    );
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            if let Err(e) = write.send(Message::Text(subscription_request.into())).await {
                eprintln!(
                    "[{}] Subscription failed ({}): {}. Reconnecting...",
                    Local::now().format("%H:%M:%S"),
                    subscription_request,
                    e
                );
                sleep(Duration::from_secs(1)).await;
                continue;
            }

            ts(&format!("Subscribed ({}). Listening...", subscription_request));

            // Message processing loop
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        // Fast-path: parse JSON
                        if let Ok(parsed) = from_str::<LogsNotification>(&text) {
                            let logs = &parsed.params.result.value.logs;
                            
                            // Process logs with minimal allocations
                            for log in logs {
                                // Avoid allocation for prefix check
                                if !log.starts_with("Program data: ") {
                                    continue;
                                }
                                
                                let data = &log[14..]; // Skip "Program data: "
                                
                                // Parse event (optimized)
                                if let Ok(event) = parse_optimized(data, &mut decode_buf) {
                                    // Get timestamp once
                                    let since_epoch = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or(Duration::ZERO);
                                    
                                    func((since_epoch, event)).await;
                                }
                            }
                        }
                    }
                    Ok(_) => {} // Ignore other message types
                    Err(e) => {
                        eprintln!(
                            "[{}] WebSocket error ({}): {}. Reconnecting...",
                            Local::now().format("%H:%M:%S"),
                            subscription_request,
                            e
                        );
                        break;
                    }
                }
            }

            ts(&format!("Connection lost ({}). Retrying in 5s...", subscription_request));
            sleep(Duration::from_secs(5)).await;
        }
    }
}

// Discriminators as constants
const CREATE_DISCRIMINATOR: [u8; 8] = [27, 114, 169, 77, 222, 235, 99, 118];
const TRADE_DISCRIMINATOR: [u8; 8] = [0xbd, 0xdb, 0x7f, 0xd3, 0x4e, 0xe6, 0x61, 0xee];
const BUY_AMM_DISCRIMINATOR: [u8; 8] = [62, 47, 55, 10, 165, 3, 220, 42];
const SELL_AMM_DISCRIMINATOR: [u8; 8] = [103, 244, 82, 31, 44, 245, 119, 119];

// Optimized parse function with buffer reuse
#[inline]
fn parse_optimized(data: &str, decode_buf: &mut Vec<u8>) -> Result<Event, ()> {
    // Decode base64 into reusable buffer
    decode_buf.clear();
    BASE64_STANDARD.decode_vec(data, decode_buf).map_err(|_| ())?;

    // Fast bounds check
    if decode_buf.len() < 8 {
        return Err(());
    }

    // Get discriminator without allocation
    let discriminator = &decode_buf[0..8];
    let mut buffer = &decode_buf[8..];

    // Match discriminator (branch prediction friendly)
    if discriminator == TRADE_DISCRIMINATOR {
        // Most common case first for better branch prediction
        let event = TradeEvent::deserialize(&mut buffer).map_err(|_| ())?;
        
        let impact = calc_price_impact(
            event.virtual_sol_reserves,
            event.virtual_token_reserves,
            event.sol_amount,
            event.token_amount,
            event.is_buy,
            1_000_000_000,
        );
        
        let pool = pool_pda(&event.mint).0;
        
        // Use if/else instead of match for better codegen
        if event.is_buy {
            Ok(Event::Buy(BuyEvent {
                mint: pool,
                sol_amount: event.sol_amount,
                token_amount: event.token_amount,
                user: event.user,
                timestamp: event.timestamp,
                virtual_sol_reserves_before: event.virtual_sol_reserves,
                virtual_sol_reserves_after: impact.mcap_after,
                virtual_token_reserves: event.virtual_token_reserves,
            }))
        } else {
            Ok(Event::Sell(SellEvent {
                mint: pool,
                sol_amount: event.sol_amount,
                token_amount: event.token_amount,
                user: event.user,
                timestamp: event.timestamp,
                virtual_sol_reserves_before: event.virtual_sol_reserves,
                virtual_sol_reserves_after: impact.mcap_after,
                virtual_token_reserves: event.virtual_token_reserves,
            }))
        }
    } else if discriminator == CREATE_DISCRIMINATOR {
        // Try V2 first, fallback to V1
        if let Ok(create) = CreateEventV2::deserialize(&mut buffer) {
            Ok(Event::Create(create.into()))
        } else {
            buffer = &decode_buf[8..]; // Reset buffer
            let create = CreateEvent::deserialize(&mut buffer).map_err(|_| ())?;
            Ok(Event::Create(create))
        }
    } else if discriminator == BUY_AMM_DISCRIMINATOR {
        let buy = BuyEventAMM::deserialize(&mut buffer).map_err(|_| ())?;
        Ok(Event::Buy(buy.into()))
    } else if discriminator == SELL_AMM_DISCRIMINATOR {
        let sell = SellEventAMM::deserialize(&mut buffer).map_err(|_| ())?;
        Ok(Event::Sell(sell.into()))
    } else {
        Err(())
    }
}