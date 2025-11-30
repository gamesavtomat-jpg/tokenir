#![windows_subsystem = "windows"]

use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use solana_sdk::{pubkey, signature::Keypair};
use tokenir_ui::migration::get_user_created_coins;
use tokio::sync::Mutex;

use crate::{
    autobuy::{AutoBuyConfig, BuyAutomata, Params},
    blacklist::Blacklist,
    fetcher::Client,
    filter::FilterSet,
    pool::Pool,
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
    let solana_client = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        env::var("SOLANA_RPC").unwrap(),
    ));

    let blacklist = Arc::new(Mutex::new(Blacklist::load()));

    let automata = Arc::new(Mutex::new(BuyAutomata::with_config(
        solana_client.clone(),
        AutoBuyConfig::load(),
    )));

    let url = env::var("SERVER").unwrap();
    let pool = Arc::new(Mutex::new(Pool::new()));
    let client = Client::new(url);

    let price = Arc::new(AtomicU64::new(180));
    let total = Arc::new(AtomicI64::new(0));

    let ui_pool = pool.clone();
    let ui_price = price.clone();
    let ui_total = total.clone();
    let ui_automata = automata.clone();
    let close_automata = automata.clone();
    let blacklist_clone = blacklist.clone();

    tokio::spawn(async move {
        let _ = client
            .subscribe(|mut token| {
                let total = total.clone();
                let pool = pool.clone();
                let pool_buy = pool.clone();
                let blacklist = blacklist.clone();
                let automata = automata.clone();
                let solana_client = solana_client.clone();
                let solana_client_buy = solana_client.clone();
                //println!("yes");
                async move {
                    let migration = get_user_created_coins(&token.dev).await.ok();
                    token.migrated = migration;

                    let mut token_clone = token.clone();

                    if let Some(performance) = &token.dev_performance {
                        let lock = pool.lock().await;
                        
                        println!("with twitter!");
                        if lock.filters.matches(&token, Some(performance.average_ath)) {
                            let blacklist = blacklist.lock().await;
                            drop(lock);

                            if let Some(twitter) = &token.twitter {
                                if !blacklist.present(&blacklist::Bannable::Twitter(
                                    twitter.creator.id.clone()
                                )) {
                                    drop(blacklist);
                                    let average_ath = performance.average_ath;
                                    let curve = token.curve.clone();

                                    tokio::spawn(async move {
                                        println!("why would i add it lol");
                                        let _ = token.load_history().await;

                                        let mut lock = pool.lock().await;
                                        lock.add(token);
                                        drop(lock);
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
                                            if let Ok(_) = automata.buy(&token_clone).await {
                                                println!("bought!");
                                            };
                                        }
                                    }

                                    total.fetch_add(1, Ordering::Relaxed);

                                    let _ = open::that(format!(
                                        "https://axiom.trade/meme/{}",
                                        curve.to_string()
                                    ));
                                }
                            }

                            return;
                        }
                    } else {
                        if let Some(migrated) = &token_clone.migrated {
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
                                            if let Ok(_) = automata.buy(&token_clone).await {
                                                println!("bought migrated!");
                                            };
                                        }
                                    }

                                    if !lock.feed_check.contains(&token_clone.mint) {
                                        lock.add(token_clone);
                                    }

                                    drop(lock);

                                    total.fetch_add(1, Ordering::Relaxed);

                                    std::thread::spawn({
                                        let url = format!("https://axiom.trade/meme/{}", curve);
                                        move || {
                                            if let Err(e) = open::that(url) {
                                                eprintln!("failed to open url: {e}");
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            })
            .await;
    });

    const ICON: &[u8; 9946] = include_bytes!("../logo.png");

    let mut options = eframe::NativeOptions::default();
    options.viewport.icon =  Some(Arc::new(eframe::icon_data::from_png_bytes(ICON)
        .expect("The icon data must be valid")));
    
    let app = ui::MyApp::new(
        ui_pool.clone(),
        blacklist_clone.clone(),
        ui_price,
        ui_total,
        ui_automata.clone(),
        Some(AutoBuyConfig::load()),
    );

    eframe::run_native("MemeX", options, Box::new(|_| Ok(Box::new(app))));

    close_automata.lock().await.config.to_file();
}
