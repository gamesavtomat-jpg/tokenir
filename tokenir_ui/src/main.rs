#![windows_subsystem = "windows"]

use std::{
    env, io::Read, sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
        RwLock, // Using std::sync::RwLock for sharing between threads
    }
};

use solana_sdk::{pubkey, signature::Keypair};
use tokenir_ui::migration::get_user_created_coins;
use tokio::sync::Mutex;

use crate::{
    autobuy::{AutoBuyConfig, BuyAutomata, Params},
    blacklist::Blacklist,
    fetcher::Client,
    filter::FilterSet,
    pool::Pool, ui::KeyConfig,
};

mod autobuy;
mod blacklist;
mod fetcher;
mod filter;
mod pool;
mod pump_interaction;
mod ui;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    // 1. Initialize objects that don't depend on the key
    let solana_client = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        env::var("SOLANA_RPC").unwrap(),
    ));

    let blacklist = Arc::new(Mutex::new(Blacklist::load()));

    let automata = Arc::new(Mutex::new(BuyAutomata::with_config(
        solana_client.clone(),
        AutoBuyConfig::load(),
    )));

    let pool = Arc::new(Mutex::new(Pool::new()));
    let price = Arc::new(AtomicU64::new(180));
    let total = Arc::new(AtomicI64::new(0));
    
    // Global state for browser opening permission
    let is_logged_in = Arc::new(RwLock::new(false));

    // 2. Clone Arcs to be moved into the connection task
    let task_pool = pool.clone();
    let task_total = total.clone();
    let task_automata = automata.clone();
    let task_blacklist = blacklist.clone();
    let task_solana = solana_client.clone(); // Kept if needed later
    
    // Clone login state for the background task
    let task_login_state = is_logged_in.clone();

    // 4. Spawn the background task that WAITS for the key before connecting
    let (tx, mut rx) = tokio::sync::watch::channel(String::new());

tokio::spawn(async move {
    let mut current_handle: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        // ждём нового ключа
        rx.changed().await.unwrap();
        let access_key = rx.borrow().clone();

        if access_key.is_empty() {
            continue;
        }

        println!("Key received/updated, starting connection...");

        // отменяем предыдущий клиент, если есть
        if let Some(handle) = current_handle.take() {
            handle.abort();
            println!("Previous connection aborted");
        }

        let base_url = std::env::var("SERVER").expect("SERVER env var must be set");
        let url = format!("{}?key={}", base_url, access_key);

        let client = Client::new(url);

        // создаём новую таску для subscription
        let handle = tokio::spawn({
            let task_total = task_total.clone();
            let task_pool = task_pool.clone();
            let task_blacklist = task_blacklist.clone();
            let task_automata = task_automata.clone();
            let task_login_state = task_login_state.clone();

            async move {
                let _ = client
                    .subscribe(|mut token| {
                        let total = task_total.clone();
                        let pool = task_pool.clone();
                        let blacklist = task_blacklist.clone();
                        let automata = task_automata.clone();
                        let login_state = task_login_state.clone();

                        async move {
                            if !*login_state.read().unwrap() {
                                return;
                            }

                            let migration = get_user_created_coins(&token.dev).await.ok();
                            token.migrated = migration;

                            let mut token_clone = token.clone();

                            if let Some(performance) = &token.dev_performance {
                                let lock = pool.lock().await;

                                if lock.filters.matches(&token, Some(performance.average_ath)) {
                                    let blacklist = blacklist.lock().await;
                                    drop(lock);

                                    if let Some(twitter) = &token.twitter {
                                        if !blacklist.present(&blacklist::Bannable::Twitter(
                                            twitter.creator.id.clone(),
                                        )) {
                                            drop(blacklist);
                                            let average_ath = performance.average_ath;
                                            let curve = token.curve.clone();

                                            tokio::spawn(async move {
                                                let _ = token.load_history().await;
                                                let mut lock = pool.lock().await;
                                                lock.add(token);
                                            });

                                            if automata
                                                .lock()
                                                .await
                                                .config
                                                .params
                                                .filters
                                                .matches(&token_clone, Some(average_ath))
                                            {
                                                let automata = automata.lock().await;

                                                if automata.active_twitter {
                                                    let _ = automata.buy(&token_clone).await;
                                                    println!("bought!");
                                                }
                                            }

                                            total.fetch_add(1, Ordering::Relaxed);

                                            if *login_state.read().unwrap() {
                                                let _ = open::that(format!(
                                                    "https://axiom.trade/meme/{}",
                                                    curve.to_string()
                                                ));
                                            }
                                        }
                                    }

                                    return;
                                }
                            } else if let Some(_migrated) = &token_clone.migrated {
                                let lock = pool.lock().await;

                                if lock.filters.matches(&token_clone, None) {
                                    let blacklist = blacklist.lock().await;
                                    drop(lock);

                                    if !blacklist.present(&blacklist::Bannable::Wallet(token.dev)) {
                                        let curve = token_clone.curve.clone();
                                        let mut lock = pool.lock().await;

                                        if automata
                                            .lock()
                                            .await
                                            .config
                                            .params
                                            .filters
                                            .matches(&token_clone, None)
                                        {
                                            let automata = automata.lock().await;

                                            if automata.active_migrate {
                                                let _ = automata.buy(&token_clone).await;
                                                println!("bought migrated!");
                                            }
                                        }

                                        if !lock.feed_check.contains(&token_clone.mint) {
                                            lock.add(token_clone);
                                        }

                                        drop(lock);
                                        total.fetch_add(1, Ordering::Relaxed);

                                        let url = format!("https://axiom.trade/meme/{}", curve);
                                        let thread_login_state = login_state.clone();
                                        std::thread::spawn(move || {
                                            if *thread_login_state.read().unwrap() {
                                                let _ = open::that(url);
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    })
                    .await;
            }
        });

        current_handle = Some(handle);
    }
});



    // 5. Setup UI
    const ICON: &[u8; 9946] = include_bytes!("../logo.png");

    let mut options = eframe::NativeOptions::default();
    options.viewport.icon = Some(Arc::new(eframe::icon_data::from_png_bytes(ICON)
        .expect("The icon data must be valid")));
    
    // 6. Launch App, passing the sender 'tx' and the login state
    let close_automata = automata.clone();
    
    let app = ui::Launcher::new(
        pool.clone(),
        blacklist.clone(),
        price,
        total,
        automata.clone(),
        Some(AutoBuyConfig::load()),
        tx,
        is_logged_in, // Pass the lock here
    );

    eframe::run_native("MemeX", options, Box::new(|_| Ok(Box::new(app))));

    close_automata.lock().await.config.to_file();
}