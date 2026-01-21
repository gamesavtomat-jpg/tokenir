#![windows_subsystem = "windows"]

use std::{
    env,
    sync::{
        Arc, RwLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use rmp_serde::{decode, encode};
use serde::{Deserialize, Serialize};
use tokenir_ui::{Token, migration::PadreClient};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::client::IntoClientRequest,
    tungstenite::protocol::Message,
};
use uuid::Uuid;

use crate::{
    autobuy::{AutoBuyConfig, BuyAutomata},
    blacklist::Blacklist,
    fetcher::Client,
    pool::Pool,
    ui::TradeTerminal,
    whitelist::{Allowable, Whitelist},
};

mod autobuy;
mod blacklist;
mod fetcher;
mod filter;
mod pool;
mod pump_interaction;
mod ui;
mod whitelist;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let solana_client = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        env::var("SOLANA_RPC").expect("SOLANA_RPC missing"),
    ));

    let blacklist = Arc::new(Mutex::new(Blacklist::load()));
    let whitelist = Arc::new(Mutex::new(Whitelist::load()));
    let automata = Arc::new(Mutex::new(BuyAutomata::with_config(
        solana_client.clone(),
        AutoBuyConfig::load(),
    )));

    let pool = Arc::new(Mutex::new(Pool::new()));
    let price = Arc::new(AtomicU64::new(180));
    let total = Arc::new(AtomicI64::new(0));

    let is_logged_in = Arc::new(RwLock::new(false));
    let trade_terminal = Arc::new(RwLock::new(
        ui::TradeTerminal::load_from_file("./terminal.json").unwrap_or(ui::TradeTerminal::Axiom),
    ));

    let (tx, mut rx) = tokio::sync::watch::channel(String::new());

    tokio::spawn({
        let pool = pool.clone();
        let total = total.clone();
        let blacklist = blacklist.clone();
        let automata = automata.clone();
        let login_state = is_logged_in.clone();
        let trade_terminal = trade_terminal.clone();

        async move {
            let mut current: Option<tokio::task::JoinHandle<()>> = None;
            loop {
                rx.changed().await.unwrap();
                let key = rx.borrow().clone();
                if key.is_empty() {
                    continue;
                }

                if let Some(h) = current.take() {
                    h.abort();
                }

                let base = env::var("SERVER").expect("SERVER missing");
                let client = Client::new(format!("{}?key={}", base, key));

                current = Some(tokio::spawn(run_subscription(
                    client,
                    pool.clone(),
                    total.clone(),
                    blacklist.clone(),
                    whitelist.clone(),
                    automata.clone(),
                    login_state.clone(),
                    trade_terminal.clone(),
                )));
            }
        }
    });

    // GUI Launch
    const ICON: &[u8] = include_bytes!("../logo.png");
    let mut options = eframe::NativeOptions::default();
    options.viewport.icon = Some(Arc::new(eframe::icon_data::from_png_bytes(ICON).unwrap()));

    let app = ui::Launcher::new(
        pool.clone(),
        blacklist.clone(),
        price,
        total,
        automata.clone(),
        Some(AutoBuyConfig::load()),
        tx,
        is_logged_in.clone(),
        trade_terminal.clone(),
    );

    eframe::run_native("MemeX", options, Box::new(|_| Ok(Box::new(app))));
    automata.lock().await.config.to_file();
}

async fn run_subscription(
    client: Client,
    pool: Arc<Mutex<Pool>>,
    total: Arc<AtomicI64>,
    blacklist: Arc<Mutex<Blacklist>>,
    whitelist: Arc<Mutex<Whitelist>>,
    automata: Arc<Mutex<BuyAutomata>>,
    login_state: Arc<RwLock<bool>>,
    trade_terminal: Arc<RwLock<TradeTerminal>>,
) {
    // Initialize Padre Client for this subscription session
    let padre = Arc::new(
        PadreClient::new()
            .await
            .expect("Failed to connect to Padre"),
    );

    let _ = client
        .subscribe(|mut token, autobuy| {
            let pool = pool.clone();
            let total = total.clone();
            let blacklist = blacklist.clone();
            let whitelist = whitelist.clone();
            let automata = automata.clone();
            let login_state = login_state.clone();
            let trade_terminal = trade_terminal.clone();
            let padre = padre.clone();

            async move {
                {
                    automata.lock().await.enabled = autobuy;
                }

                if !*login_state.read().unwrap() {
                    return;
                }

                // --- IMPLEMENTATION: WEBSOCKET MIGRATION FETCH ---
                // Request dev history from Padre and wait for binary response
                if let Some(history) = padre.get_dev_history(&token.dev.to_string()).await {
                    token.migrated = Some(history);
                }

                let token_clone = token.clone();

                // Whitelist logic
                let whitelist_bought = {
                    let whitelist_data = Whitelist::load();
                    let lock = automata.lock().await;
                    let whitelist_buy = lock.active_whitelist;
                    drop(lock);

                    if whitelist_buy && is_whitelisted(&whitelist_data, &token).await {
                        let curve = token.curve.clone();
                        let login_state = login_state.clone();
                        let trade_terminal = trade_terminal.clone();

                        std::thread::spawn(move || {
                            if *login_state.read().unwrap() {
                                let terminal = *trade_terminal.read().unwrap();
                                let _ = open::that(terminal.url(&curve));
                            }
                        });

                        tokio::spawn({
                            let pool = pool.clone();
                            let mut token = token.clone();
                            async move {
                                let _ = token.load_history().await;
                                pool.lock().await.add(token);
                            }
                        });

                        let automata = automata.lock().await;
                        let _ = automata.buy(&token_clone).await;
                        true
                    } else {
                        false
                    }
                };

                if whitelist_bought {
                    return;
                }

                // Normal Flow
                if let Some(perf) = &token.dev_performance {
                    handle_perf(
                        token.clone(),
                        perf.average_ath,
                        pool,
                        total,
                        blacklist,
                        automata,
                        login_state,
                        trade_terminal,
                    )
                    .await;
                } else {
                    handle_migrated(
                        token,
                        pool,
                        total,
                        blacklist,
                        automata,
                        login_state,
                        trade_terminal,
                    )
                    .await;
                }
            }
        })
        .await;
}

async fn handle_perf(
    token: Token,
    avg_ath: u64,
    pool: Arc<Mutex<Pool>>,
    total: Arc<AtomicI64>,
    blacklist: Arc<Mutex<Blacklist>>,
    automata: Arc<Mutex<BuyAutomata>>,
    login_state: Arc<RwLock<bool>>,
    trade_terminal: Arc<RwLock<TradeTerminal>>,
) {
    let token_clone = token.clone();
    let lock = pool.lock().await;
    if !lock.filters.matches(&token, Some(avg_ath)) {
        return;
    }
    drop(lock);

    if let Some(twitter) = &token.twitter {
        if blacklist
            .lock()
            .await
            .present(&blacklist::Bannable::Twitter(twitter.creator.id.clone()))
        {
            return;
        }
    }

    let curve = token.curve.clone();

    tokio::spawn({
        let pool = pool.clone();
        let mut token = token.clone();
        async move {
            let _ = token.load_history().await;
            pool.lock().await.add(token);
        }
    });

    if automata
        .lock()
        .await
        .config
        .params
        .filters
        .matches(&token_clone, Some(avg_ath))
    {
        let automata = automata.lock().await;
        if automata.active_twitter {
            let _ = automata.buy(&token_clone).await;
        }
    }

    total.fetch_add(1, Ordering::Relaxed);

    std::thread::spawn(move || {
        if *login_state.read().unwrap() {
            let terminal = *trade_terminal.read().unwrap();
            let _ = open::that(terminal.url(&curve));
        }
    });
}

async fn handle_migrated(
    token: Token,
    pool: Arc<Mutex<Pool>>,
    total: Arc<AtomicI64>,
    blacklist: Arc<Mutex<Blacklist>>,
    automata: Arc<Mutex<BuyAutomata>>,
    login_state: Arc<RwLock<bool>>,
    trade_terminal: Arc<RwLock<TradeTerminal>>,
) {
    let mut lock = pool.lock().await;
    lock.add(token);
    // if !lock.filters.matches(&token, None) {
    //     return;
    // }
    // drop(lock);

    // if blacklist
    //     .lock()
    //     .await
    //     .present(&blacklist::Bannable::Wallet(token.dev))
    // {
    //     return;
    // }

    // if automata
    //     .lock()
    //     .await
    //     .config
    //     .params
    //     .filters
    //     .matches(&token, None)
    // {
    //     let automata = automata.lock().await;
    //     if automata.active_migrate {
    //         let _ = automata.buy(&token).await;
    //     }
    // }

    // let curve = token.curve.clone();
    // let mut lock = pool.lock().await;

    // if !lock.feed_check.contains(&token.mint) {
    //     lock.add(token);
    // }

    // total.fetch_add(1, Ordering::Relaxed);

    // let login_state = login_state.clone();
    // std::thread::spawn(move || {
    //     if *login_state.read().unwrap() {
    //         let terminal = *trade_terminal.read().unwrap();
    //         let _ = open::that(terminal.url(&curve));
    //     }
    // });
}

/// проверка whitelist для токена
async fn is_whitelisted(whitelist: &Whitelist, token: &Token) -> bool {
    let wl = whitelist;

    // wallet
    if wl.present(&Allowable::Wallet(token.dev)) {
        return true;
    }

    // twitter screen_name
    if let Some(twitter) = &token.twitter {
        if let Some(screen_name) = &twitter.creator.screen_name {
            return wl.present(&Allowable::Twitter(screen_name.to_string()));
        }
    }

    false
}
