// main.rs

// --- NEW DEPENDENCIES ---
use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::Serialize;
use std::net::SocketAddr;
use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;
// --- END NEW DEPENDENCIES ---

use solana_sdk::pubkey::Pubkey;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokenir::constans::helper::{
    bounding_curve, fetch_solana_price, get_community_by_id, get_metadata, get_uri, metadata,
    parse_community_id,
};
use tokenir::database::{Database, DbToken};
use tokenir::filters::FilterSet;
use tokenir::{Client, Token, TokenPool}; // Make sure Token is public and imported
use tokenir::{DevPerformance, bundler::Bundler, constans::helper::pool_pda};
use tokio::sync::{Mutex, RwLock, Semaphore};

use tokenir::constans::requests::get_user_created_coins;
use tokenir::logs::{BuyEvent, CreateEvent, Event};

// --- NEW: WebSocket Shared State ---
// This struct holds the state of our WebSocket server, specifically
// a map of connected clients. Each client gets a unique ID and a
// sender channel to push messages to them.
#[derive(Default)]
struct AppState {
    clients: HashMap<Uuid, mpsc::UnboundedSender<Message>>,
}

// a type alias for our shared state, wrapped in Arc<RwLock<>> for
// concurrent reads (broadcast) without blocking.
type SharedState = Arc<RwLock<AppState>>;
// --- END NEW ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    let token_amount = Arc::new(AtomicI64::new(0));
    let sol_price = Arc::new(AtomicU64::new(180));
    let pool = Arc::new(Mutex::new(TokenPool::new()));
    let url = env::var("RPC_SOCKET")?;
    let twitter_key = Arc::new(env::var("TWITTER").unwrap());
    let solana = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        std::env::var("RPC_HTTP").unwrap(),
    ));
    let database = Arc::new(Database::new(std::env::var("SQL").unwrap()).await.unwrap());
    let _ = database.initialize_tables().await.unwrap();
    let shared_state = Arc::new(RwLock::new(AppState::default()));

    // background price/count updater
    tokio::spawn({
        let sol_price_clone = sol_price.clone();
        let database_clone = database.clone();
        let token_amount_clone = token_amount.clone();
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Ok(amount) = database_clone.get_total_coin_count().await {
                    token_amount_clone.store(amount, Ordering::Relaxed);
                }
                if let Ok(price) = fetch_solana_price().await {
                    let rounded_price = price.round() as u64;
                    sol_price_clone.store(rounded_price, Ordering::Relaxed);
                }
            }
        }
    });

    let (tx, mut rx) = tokio::sync::mpsc::channel::<(Duration, Event)>(1000);
    let semaphore = Arc::new(Semaphore::new(20));

    // wss listener
    tokio::spawn({
        let event_sender = tx.clone();
        async move {
            println!("[Producer] starting wss listener...");
            let client = Client::new(url);
            let _ = client
                .subscribe_to_pump(move |time_event| {
                    let tx_clone = event_sender.clone();
                    async move {
                        if let Err(e) = tx_clone.send(time_event).await {
                            eprintln!("[Producer] failed to send event to worker channel: {}", e);
                        }
                    }
                })
                .await;
            println!("[Producer] wss subscription ended.");
        }
    });

    // event consumer
    tokio::spawn({
        let solana_clone = Arc::clone(&solana);
        let pool_clone = Arc::clone(&pool);
        let db_clone = Arc::clone(&database);
        let twitter_key_clone = Arc::clone(&twitter_key);
        let sol_price_clone = Arc::clone(&sol_price);
        let semaphore_clone = Arc::clone(&semaphore);
        let state_clone = Arc::clone(&shared_state);

        async move {
            const BATCH_SIZE: usize = 5;
            while let Some(time_event) = rx.recv().await {
                let mut batch = Vec::with_capacity(BATCH_SIZE);
                batch.push(time_event);
                for _ in 1..BATCH_SIZE {
                    if let Ok(event) = rx.try_recv() {
                        batch.push(event);
                    } else {
                        break;
                    }
                }

                // pass clones (cheap Arcs) into processor
                let solana_task = solana_clone.clone();
                let pool_task = pool_clone.clone();
                let db_task = db_clone.clone();
                let twitter_key_task = twitter_key_clone.clone();
                let sol_price_task = sol_price_clone.clone();
                let semaphore_task = semaphore_clone.clone();
                let state_task = state_clone.clone();

                process_event_batch(
                    batch,
                    solana_task,
                    pool_task,
                    db_task,
                    twitter_key_task,
                    sol_price_task,
                    semaphore_task,
                    state_task,
                )
                .await;
            }
            println!("[Consumer] event channel closed. shutting down.");
        }
    });

    // run websocket server in foreground
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(shared_state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], 3001));
    println!("[websocket server] listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

// process_event_batch accepts SharedState (Arc<RwLock<AppState>>)
async fn process_event_batch(
    batch: Vec<(Duration, Event)>,
    solana: Arc<solana_client::nonblocking::rpc_client::RpcClient>,
    pool: Arc<Mutex<TokenPool>>,
    database: Arc<Database>,
    twitter_key: Arc<String>,
    sol_price: Arc<AtomicU64>,
    semaphore: Arc<Semaphore>,
    state: SharedState,
) {
    for event in batch {
        let solana_task = solana.clone();
        let pool_task = pool.clone();
        let db_task = database.clone();
        let twitter_key_task = twitter_key.clone();
        let sol_price_task = sol_price.clone();
        let state_task = state.clone();

        match event.1 {
            Event::Create(data) => {
                tokio::spawn(async move {
                    process_create_event(
                        data,
                        solana_task,
                        pool_task,
                        db_task,
                        &twitter_key_task,
                        sol_price_task.load(Ordering::Relaxed),
                        state_task,
                        event.0,
                    )
                    .await;
                });
            }
            Event::Buy(data) => {
                // spawn buy tasks so consumer loop isn't blocked by DB work
                let pool_task2 = pool_task.clone();
                let db_task2 = db_task.clone();
                let price = sol_price_task.load(Ordering::Relaxed);
                tokio::spawn(async move {
                    buy(data, pool_task2, db_task2, price).await;
                });
            }
            Event::Sell(_) => {}
        }
    }
}

async fn process_create_event(
    data: CreateEvent,
    solana: Arc<solana_client::nonblocking::rpc_client::RpcClient>,
    pool: Arc<Mutex<TokenPool>>,
    database: Arc<Database>,
    twitter_key: &str,
    price: u64,
    state: SharedState,
    time: Duration,
) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    // если событие старше 5 секунд — не пушим
    if now.saturating_sub(time) > Duration::from_secs(5) {
        println!("[skip] token is too old, not broadcasting");
        return;
    }
    println!("mint: {}", data.mint);
    let clone_data = data.clone();

    let metadata = match get_metadata(&clone_data.uri).await {
        Ok(data) => data,
        Err(err) => {
            let token = Token::fresh(
                clone_data.name,
                clone_data.symbol,
                clone_data.user,
                clone_data.bonding_curve,
                None,
                clone_data.mint,
            );
            broadcast_token(token.clone(), state.clone()).await;
            println!(
                "[broadcaster] new normal token found: {}. sending to subscribers.",
                token.mint
            );
            return;
        }
    };

    let Some(twitter) = &metadata.twitter else {
        let token = Token::fresh(
            clone_data.name,
            clone_data.symbol,
            clone_data.user,
            clone_data.bonding_curve,
            None,
            clone_data.mint,
        );
        broadcast_token(token.clone(), state.clone()).await;
        println!(
            "[broadcaster] new normal token found: {}. sending to subscribers.",
            token.mint
        );
        return;
    };

    let Some(id) = parse_community_id(&twitter) else {
        let token = Token::fresh(
            clone_data.name,
            clone_data.symbol,
            clone_data.user,
            clone_data.bonding_curve,
            None,
            clone_data.mint,
        );
        broadcast_token(token.clone(), state.clone()).await;
        println!(
            "[broadcaster] new normal token found: {}. sending to subscribers.",
            token.mint
        );
        return;
    };

    let community = match get_community_by_id(&twitter_key, &id).await {
        Ok(community) => community,
        Err(_) => {
            let token = Token::fresh(
                clone_data.name,
                clone_data.symbol,
                clone_data.user,
                clone_data.bonding_curve,
                None,
                clone_data.mint,
            );
            broadcast_token(token.clone(), state.clone()).await;
            println!(
                "[broadcaster] new normal token found: {}. sending to subscribers.",
                token.mint
            );

            return;
        }
    };

    let cloned_community = community.clone();
    let creator = community.creator;
    let id = creator.id.clone();
    let mint = data.mint.clone();

    let mut pool = pool.lock().await;
    pool.add(data, Some(cloned_community));

    let pda = pool_pda(&mint).0;
    if let Some(mut token) = pool.pool().get(&pda).cloned() {
        if let Some((average_mcap, last_tokens, count)) = average_dev_mcap(&database, &id).await {
            pool.filtered.push(mint.clone());

            token.dev_performance = Some(DevPerformance {
                average_ath: average_mcap,
                last_tokens,
                count,
            });

            // broadcast filtered token (again) — also done with minimal locking
        }

        println!(
            "[broadcaster] new filtered token found: {}. sending to subscribers.",
            token.mint
        );
        broadcast_token(token.clone(), state.clone()).await;

        let token_clone = token.clone();

        drop(token);
        drop(pool);

        let _ = database.add_dev(creator.id.clone()).await;
        let _ = database
            .add_token(&mint, &token_clone.dbtoken(mint), id)
            .await;
    }
}

/// main handler for websocket connections.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// manages an individual client's websocket connection.
async fn handle_socket(socket: WebSocket, state: SharedState) {
    let client_id = Uuid::new_v4();
    println!("[websocket] new client connected: {}", client_id);

    let (tx, mut rx) = mpsc::unbounded_channel();

    // add the client's sender to our shared state (write lock)
    {
        let mut w = state.write().await;
        w.clients.insert(client_id, tx);
    }

    let (mut sink, mut stream) = socket.split();

    // task to send messages to the client (reads from rx)
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // task to receive messages from client (we ignore content for now)
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(_msg)) = stream.next().await {
            // future: handle client messages if needed
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    // cleanup: remove client (write lock)
    {
        let mut w = state.write().await;
        w.clients.remove(&client_id);
    }
    println!("[websocket] client disconnected: {}", client_id);
}

/// serializes a token and broadcasts it to all connected clients.
/// optimized: take snapshot of senders under read-lock, drop lock, send.
/// if some sends fail - remove failed clients under write-lock.
async fn broadcast_token<T: Serialize + Clone>(data: T, state: SharedState) {
    let msg = match serde_json::to_string(&data) {
        Ok(json) => Message::Text(json),
        Err(e) => {
            eprintln!("[broadcaster] failed to serialize token: {}", e);
            return;
        }
    };

    // take a snapshot of (id, sender) under read lock
    let clients_snapshot: Vec<(Uuid, mpsc::UnboundedSender<Message>)> = {
        let r = state.read().await;
        r.clients.iter().map(|(id, tx)| (*id, tx.clone())).collect()
    };

    if clients_snapshot.is_empty() {
        return;
    }

    // send without holding the lock
    let mut failed = Vec::new();
    for (id, tx) in clients_snapshot {
        if tx.send(msg.clone()).is_err() {
            failed.push(id);
        }
    }

    // remove failed clients if any under write lock
    if !failed.is_empty() {
        let mut w = state.write().await;
        for id in failed {
            w.clients.remove(&id);
        }
    }
}

async fn buy(data: BuyEvent, pool: Arc<Mutex<TokenPool>>, database: Arc<Database>, price: u64) {
    let mint = data.mint.clone();
    let mut ath = 0;

    let mut pool = pool.lock().await;
    if let Some(token) = pool.pool().get(&mint) {
        ath = token.usd_ath();
    }

    let _ = pool.update(&data.mint.clone(), tokenir::Trade::Buy(data), price);

    if let Some(token) = pool.pool().get(&mint) {
        if token.usd_ath() > ath {
            let _ = database
                .update_token_ath(&token.mint, &token.clone().dbtoken(token.mint))
                .await;
        }
    }
}

pub async fn average_dev_mcap(db: &Database, dev: &str) -> Option<(u64, Vec<DbToken>, usize)> {
    match db.get_tokens_by_dev(dev).await {
        Ok(tokens) if !tokens.is_empty() => {
            let count = tokens.len();
            let sum: i64 = tokens.iter().map(|t| t.ath).sum();
            let avg = sum as u64 / tokens.len() as u64;
            let last_three = tokens.iter().rev().take(3).cloned().collect::<Vec<_>>();
            Some((avg, last_three, count))
        }
        _ => None,
    }
}
