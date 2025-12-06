use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}, Query}, // Added Query
    response::IntoResponse,
    routing::{get, post},
    Router,
    Json,
    http::StatusCode,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio::sync::{Mutex, Semaphore, broadcast};
use tower_http::cors::{Any, CorsLayer};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey};
use std::env;
use std::str::FromStr;

// Ensure you import the types needed for the DB actions
use tokenir::access::{AddUserPayload, User}; 
use tokenir::constans::helper::{
    bounding_curve, fetch_solana_price, get_community_by_id, get_metadata, get_uri, metadata,
    parse_community_id, pool_pda
};
use tokenir::database::{Database, DbToken};
use tokenir::filters::FilterSet;
use tokenir::{Client, Token, TokenPool};
use tokenir::{DevPerformance, bundler::Bundler};
use tokenir::constans::requests::get_user_created_coins;
use tokenir::logs::{BuyEvent, CreateEvent, Event};

struct AppState {
    tx: broadcast::Sender<String>,
    db: Arc<Database>,
}

type SharedState = Arc<AppState>;

#[derive(Deserialize)]
struct AddUserReq {
    admin_key: String,
    payload: AddUserPayload,
}

#[derive(Deserialize)]
struct RemoveUserReq {
    admin_key: String,
    user_id: i32,
}

#[derive(Deserialize)]
struct GetUsersReq {
    admin_key: String,
}

// --- NEW: Auth Struct for WebSocket ---
#[derive(Deserialize)]
struct WsAuth {
    key: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    // 1. Initialize Broadcast Channel
    let (tx, _rx) = broadcast::channel(100);
    
    // Database initialization
    let database = Arc::new(Database::new(std::env::var("SQL").unwrap()).await.unwrap());
    let _ = database.initialize_tables().await.unwrap();

    // 2. Initialize Shared State
    let shared_state = Arc::new(AppState { 
        tx,
        db: database.clone() 
    });

    let token_amount = Arc::new(AtomicI64::new(0));
    let sol_price = Arc::new(AtomicU64::new(180));
    let pool = Arc::new(Mutex::new(TokenPool::new()));

    let url = env::var("RPC_SOCKET")?;
    let twitter_key = Arc::new(env::var("TWITTER").unwrap());

    let solana = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        std::env::var("RPC_HTTP").unwrap(),
    ));

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

    let (tx_event, mut rx_event) = tokio::sync::mpsc::channel::<(Duration, Event)>(1000);
    let semaphore = Arc::new(Semaphore::new(20));

    // wss listener (Solana)
    tokio::spawn({
        let event_sender = tx_event.clone();
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
            while let Some(time_event) = rx_event.recv().await {
                let mut batch = Vec::with_capacity(BATCH_SIZE);
                batch.push(time_event);
                for _ in 1..BATCH_SIZE {
                    if let Ok(event) = rx_event.try_recv() {
                        batch.push(event);
                    } else {
                        break;
                    }
                }
                process_event_batch(
                    batch,
                    solana_clone.clone(),
                    pool_clone.clone(),
                    db_clone.clone(),
                    twitter_key_clone.clone(),
                    sol_price_clone.clone(),
                    semaphore_clone.clone(),
                    state_clone.clone(),
                )
                .await;
            }
            println!("[Consumer] event channel closed. shutting down.");
        }
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/admin/add_user", post(add_user_handler))
        .route("/admin/remove_user", post(remove_user_handler))
        .route("/admin/users", post(get_users_handler))
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

async fn add_user_handler(
    State(state): State<SharedState>,
    Json(req): Json<AddUserReq>,
) -> impl IntoResponse {
    match state.db.add_user(&req.admin_key, req.payload).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "success"}))),
        Err(sqlx::Error::RowNotFound) => (
            StatusCode::FORBIDDEN, 
            Json(serde_json::json!({"error": "Invalid admin key or unauthorized"}))
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR, 
            Json(serde_json::json!({"error": e.to_string()}))
        ),
    }
}

async fn remove_user_handler(
    State(state): State<SharedState>,
    Json(req): Json<RemoveUserReq>,
) -> impl IntoResponse {
    match state.db.remove_user(&req.admin_key, req.user_id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "removed"}))),
        Err(sqlx::Error::RowNotFound) => (
            StatusCode::FORBIDDEN, 
            Json(serde_json::json!({"error": "Invalid admin key or target not found/admin"}))
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR, 
            Json(serde_json::json!({"error": e.to_string()}))
        ),
    }
}

async fn get_users_handler(
    State(state): State<SharedState>,
    Json(req): Json<GetUsersReq>,
) -> impl IntoResponse {
    match state.db.fetch_all_users(&req.admin_key).await {
        Ok(users) => (StatusCode::OK, Json(serde_json::to_value(users).unwrap())),
        Err(sqlx::Error::RowNotFound) => (
            StatusCode::FORBIDDEN, 
            Json(serde_json::json!({"error": "Invalid admin key"}))
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR, 
            Json(serde_json::json!({"error": e.to_string()}))
        ),
    }
}

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
        }
        drop(pool);

        println!(
            "[broadcaster] new filtered token found: {}. sending to subscribers.",
            token.mint
        );
        broadcast_token(token.clone(), state.clone()).await;

        let token_clone = token.clone();
        drop(token);

        let _ = database.add_dev(creator.id.clone()).await;
        let _ = database
            .add_token(&mint, &token_clone.dbtoken(mint), id)
            .await;
    }
}

// --- MODIFIED: WebSocket Handler with Authentication ---
async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(auth): Query<WsAuth>, // Extract ?key=... from URL
    State(state): State<SharedState>,
) -> impl IntoResponse {
    // 1. Check Key Length
    if auth.key.len() != 32 {
        println!("[websocket] connection denied: key length != 32");
        return (StatusCode::BAD_REQUEST, "Invalid Key Length").into_response();
    }

    // 2. Validate Key against Database
    match state.db.validate_user_key(&auth.key).await {
        Ok(true) => {
            // Authorized
            ws.on_upgrade(|socket| handle_socket(socket, state))
        },
        Ok(false) => {
            // Key not found
            println!("[websocket] connection denied: key not found in db");
            (StatusCode::FORBIDDEN, "Unauthorized: Invalid Key").into_response()
        },
        Err(e) => {
            // DB Error
            eprintln!("[websocket] db error during auth: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    println!("[websocket] new client connected");
    let mut rx = state.tx.subscribe();
    let (mut sink, mut stream) = socket.split();

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if sink.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(amount)) => {
                    println!("[websocket] client lagged by {} msgs - skipping forward", amount);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(_)) = stream.next().await {
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
    println!("[websocket] client disconnected");
}

async fn broadcast_token<T: Serialize + Clone>(data: T, state: SharedState) {
    let msg = match serde_json::to_string(&data) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("[broadcaster] failed to serialize token: {}", e);
            return;
        }
    };
    let _ = state.tx.send(msg);
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