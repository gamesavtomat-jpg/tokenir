use crate::{
    constans::{
        self,
        helper::{calc_price_impact, pool_pda},
    },
    logs::{
        BuyEvent, BuyEventAMM, CreateEvent, CreateEventV2, Event, PumpCreateEvent, SellEvent,
        SellEventAMM, TradeEvent,
    },
    requests::LogsNotification,
};

use jito_protos::shredstream::{
    shredstream_proxy_client::ShredstreamProxyClient, SubscribeEntriesRequest,
};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use borsh::{BorshDeserialize, BorshSerialize};
use futures::{SinkExt, StreamExt};
use serde_json::from_str;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use std::{future::Future, time::Duration};
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message},
};

pub struct Client {
    url: String,
}

const PUMP_PROGRAM: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
const CREATE_V2_DISCRIMINATOR: &[u8] = &[214, 144, 76, 236, 95, 139, 49, 180];
const BUY_DISCRIMINATOR: &[u8] = &[102, 6, 61, 18, 1, 218, 235, 234];

// Internal structs for Borsh (mapping raw instruction data)
#[derive(BorshSerialize, BorshDeserialize)]
struct CreateV2Args {
    name: String,
    symbol: String,
    uri: String,
    creator: [u8; 32],
    mayhem: bool,
}

#[derive(BorshSerialize, BorshDeserialize)]
struct BuyArgs {
    amount: u64,
    max_sol: u64,
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
                Client::subscribe_to_websocket(url, constans::requests::SUBSCRIBE_REQUEST_AMM, func)
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
        use futures_util::{SinkExt, StreamExt};
        use std::sync::Arc;
        use tokio::sync::Mutex;
        use tokio::time::{sleep, Duration};
        use tokio_tungstenite::tungstenite::protocol::Message;

        let mut decode_buf = Vec::with_capacity(512);

        loop {
            ts(&format!(
                "connecting to websocket ({})...",
                subscription_request
            ));

            let ws_stream = match connect_async(&url).await {
                Ok((stream, _)) => {
                    ts(&format!("connected ({}).", subscription_request));
                    stream
                }
                Err(e) => {
                    eprintln!(
                        "[{}] connection failed ({}): {}. retrying in 5s...",
                        Local::now().format("%H:%M:%S"),
                        subscription_request,
                        e
                    );
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (write, mut read) = ws_stream.split();
            let write = Arc::new(Mutex::new(write));

            // subscribe
            {
                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Text(subscription_request.into())).await {
                    eprintln!(
                        "[{}] subscription failed ({}): {}",
                        Local::now().format("%H:%M:%S"),
                        subscription_request,
                        e
                    );
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }

            ts(&format!(
                "subscribed ({}). listening...",
                subscription_request
            ));

            // ===== heartbeat task =====
            let write_hb = write.clone();
            let heartbeat = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(15));
                loop {
                    interval.tick().await;
                    let mut w = write_hb.lock().await;
                    if w.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
            });

            // ===== read loop =====
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Ping(payload)) => {
                        let mut w = write.lock().await;
                        let _ = w.send(Message::Pong(payload)).await;
                    }

                    Ok(Message::Pong(_)) => {
                        // alive
                    }

                    Ok(Message::Text(text)) => {
                        if let Ok(parsed) = from_str::<LogsNotification>(&text) {
                            for log in &parsed.params.result.value.logs {
                                if !log.starts_with("Program data: ") {
                                    continue;
                                }

                                let data = &log[14..];
                                if let Ok(event) = parse_optimized(data, &mut decode_buf) {
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default();

                                    func((ts, event)).await;
                                }
                            }
                        }
                    }

                    Ok(Message::Close(frame)) => {
                        eprintln!(
                            "[{}] ws closed ({}): {:?}",
                            Local::now().format("%H:%M:%S"),
                            subscription_request,
                            frame
                        );
                        break;
                    }

                    Err(e) => {
                        eprintln!(
                            "[{}] ws error ({}): {}",
                            Local::now().format("%H:%M:%S"),
                            subscription_request,
                            e
                        );
                        break;
                    }

                    _ => {}
                }
            }

            heartbeat.abort();

            ts(&format!(
                "connection lost ({}). retrying in 5s...",
                subscription_request
            ));
            sleep(Duration::from_secs(5)).await;
        }
    }

    pub async fn subscribe_new_tokens<F, Fut>(&self, mut func: F) -> Result<(), Error>
    where
        F: FnMut((Event, Duration)) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        loop {
            ts("Connecting to PumpPortal Data API...");

            let ws_result = connect_async(&self.url).await;
            let (ws_stream, _) = match ws_result {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "[{}] Connection failed: {}. Retrying in 5s...",
                        Local::now().format("%H:%M:%S"),
                        e
                    );
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            let subscribe_msg = r#"{"method":"subscribeNewToken"}"#;
            if let Err(e) = write.send(Message::Text(subscribe_msg.into())).await {
                eprintln!("Subscription send failed: {}", e);
                continue;
            }

            ts("Subscribed to New Tokens. Listening for pump-only create events...");

            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(raw_event) = from_str::<PumpCreateEvent>(&text) {
                            // FILTER: Only "create" type AND only "pump" pool
                            if raw_event.tx_type == "create" && raw_event.pool == "pump" {
                                // Safe parsing of Pubkeys from dynamic strings
                                let mint = raw_event.mint.parse::<Pubkey>().unwrap_or_default();
                                let bonding_curve = raw_event
                                    .bonding_curve_key
                                    .parse::<Pubkey>()
                                    .unwrap_or_default();
                                let user = raw_event
                                    .trader_public_key
                                    .parse::<Pubkey>()
                                    .unwrap_or_default();

                                let event = CreateEvent {
                                    name: raw_event.name,
                                    symbol: raw_event.symbol,
                                    uri: raw_event.uri,
                                    mint,
                                    bonding_curve,
                                    user,
                                    timestamp: current_timestamp_secs() as i64,
                                    token_2022: true,
                                };

                                let since_epoch = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or(Duration::ZERO);

                                func((Event::Create(event), since_epoch)).await;
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        eprintln!("Websocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            ts("Connection lost. Reconnecting in 2s...");
            sleep(Duration::from_secs(2)).await;
        }
    }

    pub async fn subscribe_jito<F, Fut>(
        &self,
        jito_url: String,
        mut func: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut((Event, Duration)) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        ts(&format!(
            "Connecting to Jito Shredstream at {}...",
            jito_url
        ));

        // Wrapped in a loop for basic reconnection logic
        loop {
            let mut client = match ShredstreamProxyClient::connect(jito_url.clone()).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Jito connection failed: {}. Retrying in 5s...", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let mut stream = match client.subscribe_entries(SubscribeEntriesRequest {}).await {
                Ok(s) => s.into_inner(),
                Err(e) => {
                    eprintln!("Jito subscription failed: {}. Retrying...", e);
                    continue;
                }
            };

            ts("Jito Stream Connected. Monitoring transactions...");

            while let Some(slot_entry_res) = stream.message().await.ok().flatten() {
                let entries: Vec<solana_entry::entry::Entry> =
                    match bincode::deserialize(&slot_entry_res.entries) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };

                for tx in entries.iter().flat_map(|e| &e.transactions) {
                    let lookup = tx.message.static_account_keys();

                    for instruction in tx.message.instructions() {
                        // --- FIX: SAFE BOUNDS CHECKING ---
                        // Instead of calling instruction.program_id(&lookup) which panics,
                        // we manually check if the index exists in our static lookup.
                        let program_idx = instruction.program_id_index as usize;
                        let program_id = match lookup.get(program_idx) {
                            Some(id) => id,
                            None => continue, // Skip if index is out of bounds (e.g. in a Lookup Table)
                        };

                        if program_id != &PUMP_PROGRAM {
                            continue;
                        }

                        let since_epoch = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or(Duration::ZERO);

                        // 2. Handle CREATE_V2
                        if instruction.data.starts_with(CREATE_V2_DISCRIMINATOR) {
                            if let Ok(args) = CreateV2Args::try_from_slice(&instruction.data[8..]) {
                                // --- FIX: SAFE ACCOUNT RESOLUTION ---
                                // Using the updated get_account_ptr helper below
                                let mint = get_account_ptr(0, &instruction.accounts, lookup);
                                let bonding_curve =
                                    get_account_ptr(2, &instruction.accounts, lookup);
                                let token_acc = get_account_ptr(7, &instruction.accounts, lookup);

                                if let (Some(mint), Some(bonding_curve), Some(token_acc)) =
                                    (mint, bonding_curve, token_acc)
                                {
                                    let event = CreateEvent {
                                        name: args.name,
                                        symbol: args.symbol,
                                        uri: args.uri,
                                        mint: *mint,
                                        bonding_curve: *bonding_curve,
                                        user: Pubkey::new_from_array(args.creator),
                                        timestamp: since_epoch.as_secs() as i64,
                                        // Token2022 check
                                        token_2022: *token_acc
                                            == pubkey!(
                                                "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
                                            ),
                                    };
                                    func((Event::Create(event), since_epoch)).await;
                                }
                            }
                        }
                    }
                }
            }
            ts("Jito connection lost. Reconnecting...");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Helper to resolve accounts from instruction account indexes and transaction lookup table
/// UPDATED: Helper to resolve accounts with safety checks
fn get_account_ptr<'a>(
    index: u8,
    instruction_accounts: &[u8],
    lookup: &'a [Pubkey],
) -> Option<&'a Pubkey> {
    // 1. Get the account index from the instruction's account list
    let account_index = *instruction_accounts.get(index as usize)? as usize;

    // 2. Safely return the Pubkey from the transaction's static account list
    lookup.get(account_index)
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
    BASE64_STANDARD
        .decode_vec(data, decode_buf)
        .map_err(|_| ())?;

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
        if let Ok(create) = CreateEventV2::deserialize(&mut buffer) {
            // inside the message processing loop, replace:
            let since_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO);
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

fn current_timestamp_secs() -> f64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs() as f64 + now.subsec_nanos() as f64 * 1e-9
}
