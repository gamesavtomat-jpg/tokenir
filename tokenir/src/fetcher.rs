use std::{future::Future, time::SystemTime};
use std::string::ParseError;
use std::time::Duration;

use borsh::BorshDeserialize;
use futures::{SinkExt, StreamExt};
use serde_json::from_str;
use tokio::time::sleep; // Added for delays
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error, Message},
};

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

// Helper function for timestamped logs
fn ts(step: &str) {
    println!("[{}] {}", Local::now().format("%H:%M:%S"), step);
}

use tokio::select;

impl Client {
    pub fn new(url: String) -> Self {
        Self { url }
    }


pub async fn subscribe_to_pump<F, Fut>(&self, func: F) -> Result<(), Error>
where
    F: FnMut((Duration, Event)) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    // Spawn both WebSocket connections concurrently
    let pump_handle = {
        let func = func.clone();
        let url = self.url.clone();
        tokio::spawn(async move {
            Client::subscribe_to_websocket(url, constans::requests::SUBSCRIBE_REQUEST_PUMP, func).await
        })
    };

    let amm_handle = {
        let url = self.url.clone();
        tokio::spawn(async move {
            Client::subscribe_to_websocket(url, constans::requests::SUBSCRIBE_REQUEST_AMM, func).await
        })
    };

    // Wait for both tasks (they run indefinitely with reconnection logic)
    select! {
        _ = pump_handle => {},
        _ = amm_handle => {},
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
    // Infinite loop to handle reconnections
    loop {
        ts(&format!(
            "Attempting to connect to WebSocket ({})...",
            subscription_request
        ));

        // Try to connect
        let ws_stream = match connect_async(&url).await {
            Ok((stream, _)) => {
                ts(&format!(
                    "WebSocket connection established ({}).",
                    subscription_request
                ));
                stream
            }
            Err(e) => {
                eprintln!(
                    "[{}] Failed to connect ({}): {}. Retrying in 5 seconds...",
                    Local::now().format("%H:%M:%S"),
                    subscription_request,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let (mut write, mut read) = ws_stream.split();

        // Re-subscribe after each successful connection
        if let Err(e) = write
            .send(Message::Text(subscription_request.into()))
            .await
        {
            eprintln!(
                "[{}] Failed to send subscription request ({}): {}. Reconnecting...",
                Local::now().format("%H:%M:%S"),
                subscription_request,
                e
            );
            sleep(Duration::from_secs(1)).await;
            continue;
        }

        ts(&format!(
            "Subscription request sent ({}). Listening for events...",
            subscription_request
        ));

        // Message processing loop
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => match from_str::<LogsNotification>(&text) {
                    Ok(parsed) => {
                        let logs = &parsed.params.result.value.logs;
                        for log in logs {
                            let Some(data) = log.strip_prefix("Program data: ") else {
                                continue;
                            };
                            let Ok(event) = parse(data) else {
                                continue;
                            };

                            let start = std::time::SystemTime::now();
                            let since_the_epoch = start
                                .duration_since(std::time::UNIX_EPOCH)
                                .expect("time should go forward");
                            func((since_the_epoch, event)).await;
                        }
                    }
                    Err(err) => {
                        println!("JSON parsing error ({}): {}", subscription_request, err)
                    }
                },
                Ok(_) => {
                    // Ignore other message types (Ping, Pong, Binary, etc.)
                }
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

        ts(&format!(
            "Connection lost ({}). Retrying in 5 seconds...",
            subscription_request
        ));
        sleep(Duration::from_secs(5)).await;
    }
}
}


enum ParseEventError {
    DecodeError(base64::DecodeError),
    DeserializationError(std::io::Error),
    EventDoesNotExist,
}

impl From<base64::DecodeError> for ParseEventError {
    fn from(value: base64::DecodeError) -> Self {
        Self::DecodeError(value)
    }
}

impl From<std::io::Error> for ParseEventError {
    fn from(value: std::io::Error) -> Self {
        Self::DeserializationError(value)
    }
}

const CREATE_DISCRIMINATOR: [u8; 8] = [27, 114, 169, 77, 222, 235, 99, 118];
const TRADE_DISCRIMINATOR: [u8; 8] = [0xbd, 0xdb, 0x7f, 0xd3, 0x4e, 0xe6, 0x61, 0xee];

const BUY_AMM_DISCRIMINATOR: [u8; 8] = [62, 47, 55, 10, 165, 3, 220, 42];
const SELL_AMM_DISCRIMINATOR: [u8; 8] = [103, 244, 82, 31, 44, 245, 119, 119];

fn parse(data: &str) -> Result<Event, ParseEventError> {
    let data = base64::decode(data)?;
    if let Some(discriminator) = data.get(0..8) {
        let mut buffer = &data[8..];

        if discriminator == CREATE_DISCRIMINATOR {
            let create = CreateEventV2::deserialize(&mut buffer);

            if let Ok(create) = create {
                println!("[create v2]");
                let event = Event::Create(create.into());
                return Ok(event);
            } else {
                println!("[create legacy]");
                let create = CreateEvent::deserialize(&mut buffer)?;
                let event = Event::Create(create);
                return Ok(event);
            }
        } else if discriminator == BUY_AMM_DISCRIMINATOR {
            let Ok(buy) = BuyEventAMM::deserialize(&mut buffer) else {
                return Err(ParseEventError::EventDoesNotExist);
            };
            let event = Event::Buy(buy.into());
            //println!("{:#?}", &event);
            return Ok(event);
        } else if discriminator == TRADE_DISCRIMINATOR {
            let event = TradeEvent::deserialize(&mut buffer)?;

            let impact = calc_price_impact(
                event.virtual_sol_reserves,
                event.virtual_token_reserves,
                event.sol_amount,
                event.token_amount,
                event.is_buy,
                1_000_000_000,
            );

            let pool = pool_pda(&event.mint).0;

            if event.is_buy {
                return Ok(Event::Buy(BuyEvent {
                    mint: pool,
                    sol_amount: event.sol_amount,
                    token_amount: event.token_amount,
                    user: event.user,
                    timestamp: event.timestamp,
                    virtual_sol_reserves_before: event.virtual_sol_reserves,
                    virtual_sol_reserves_after: impact.mcap_after,
                    virtual_token_reserves: event.virtual_token_reserves,
                }));
            }

            return Ok(Event::Sell(SellEvent {
                mint: pool,
                sol_amount: event.sol_amount,
                token_amount: event.token_amount,
                user: event.user,
                timestamp: event.timestamp,
                virtual_sol_reserves_before: event.virtual_sol_reserves,
                virtual_sol_reserves_after: impact.mcap_after,
                virtual_token_reserves: event.virtual_token_reserves,
            }));
        }
    }
    Err(ParseEventError::EventDoesNotExist)
}
