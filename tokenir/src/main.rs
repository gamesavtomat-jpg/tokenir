use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query as AxQuery, State as AxState,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use std::time::{Duration, SystemTime};
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};

// Library imports
use tokenir::database::{Database, DbToken};
use tokenir::logs::{CreateEvent, Event};
use tokenir::{access::AddUserPayload, usd_mcap};
use tokenir::{
    constans::helper::{fetch_solana_price, get_community_by_id, get_metadata, parse_community_id},
    requests::Metadata,
};
use tokenir::{Client, DevPerformance, Token};

// --- Optimized Data Types ---

#[derive(Serialize, Clone)]
#[serde(tag = "type", content = "data")]
enum SocketMessage {
    NewToken(Token),
}

struct AppState {
    tx: broadcast::Sender<Arc<String>>,
    db: Arc<Database>,
    active_connections: Arc<Mutex<HashMap<String, u64>>>,
    next_session_id: AtomicU64,
    token_cache: Arc<Mutex<TokenCache>>,
    community_cache: Arc<Mutex<CommunityCache>>,
    // Add this:
    shutdown_tx: mpsc::Sender<()>,
}

#[derive(Default)]
struct TokenCache {
    images: HashMap<String, ()>,
    ipfs: HashMap<String, ()>,
    descriptions: HashMap<String, ()>,
    names: HashMap<String, ()>,
    tickers: HashMap<String, ()>,
    name_ticker_pairs: HashMap<(String, String), ()>,
    desc_name_pairs: HashMap<(String, String), ()>,
    desc_ticker_pairs: HashMap<(String, String), ()>,
}

#[derive(Default)]
struct CommunityCache {
    community_ids: HashMap<String, ()>,
}

type SharedState = Arc<AppState>;

#[derive(Deserialize)]
struct WsAuth {
    key: String,
}

#[derive(Serialize)]
struct ConnectionStatus {
    key: String,
    session_id: u64,
}

#[derive(Deserialize)]
struct RestartReq {
    admin_key: String,
}

// --- Main Server ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    // OPTIMIZATION: Massive channel buffer for burst handling
    let (broadcast_tx, _rx) = broadcast::channel(10000);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    let database =
        Arc::new(Database::new(std::env::var("SQL").expect("SQL env var missing")).await?);

    let _ = database.initialize_tables().await?;

    let shared_state = Arc::new(AppState {
        tx: broadcast_tx.clone(),
        db: database.clone(),
        active_connections: Arc::new(Mutex::new(HashMap::new())),
        next_session_id: AtomicU64::new(0),
        token_cache: Arc::new(Mutex::new(TokenCache::default())),
        community_cache: Arc::new(Mutex::new(CommunityCache::default())),
        shutdown_tx,
    });

    let sol_price = Arc::new(AtomicU64::new(180));
    let twitter_key = Arc::new(env::var("TWITTER").expect("TWITTER env var missing"));
    let rpc_url = env::var("RPC_SOCKET").expect("RPC_SOCKET env var missing");
    let ipfs_local_node =
        Arc::new(env::var("IPFS").unwrap_or_else(|_| "http://127.0.0.1:5001".to_string()));

    // --- Background Task: SOL Price Polling ---
    tokio::spawn({
        let sp_clone = sol_price.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Ok(price) = fetch_solana_price().await {
                    sp_clone.store(price.round() as u64, Ordering::Relaxed);
                }
            }
        }
    });

    // --------------------------------------------------------
    // CONNECTION 1: SERVING (Ultra-Fast Broadcast)
    // --------------------------------------------------------
    let url_serving = rpc_url.clone();
    let b_tx_serving = broadcast_tx.clone();
    let tw_serving = twitter_key.clone();
    let db_serving = database.clone();
    let cache_serving = shared_state.token_cache.clone();
    let comm_cache_serving = shared_state.community_cache.clone();
    let ipfs_local_node_clone = ipfs_local_node.clone();

    tokio::spawn(async move {
        let client = Client::new("wss://pumpportal.fun/api/data".to_string());
        println!("[subscriber] Serving connection started...");

        let jito_link = env::var("SHREDS").expect("No SHREDS link found in .env");

        let _ = client
            .subscribe_jito(jito_link, move |(event, time)| {
                let b_tx = b_tx_serving.clone();
                let tw_key = tw_serving.clone();
                let db = db_serving.clone();
                let cache = cache_serving.clone();
                let comm_cache = comm_cache_serving.clone();
                let ipfs_local_node = ipfs_local_node_clone.clone();
                async move {
                    if let Event::Create(data) = event {
                        // OPTIMIZATION: Inline fast-path processing
                        tokio::spawn(async move {
                            if let Some(token) = process_fast_create(
                                data,
                                &tw_key,
                                time,
                                db,
                                cache,
                                comm_cache,
                                &ipfs_local_node,
                            )
                            .await
                            {
                                // OPTIMIZATION: Pre-serialize and wrap in Arc for zero-copy broadcast
                                if let Ok(json_string) = serde_json::to_string(&token) {
                                    let shared_msg = Arc::new(json_string);
                                    // Fire and forget - non-blocking send
                                    let _ = b_tx.send(shared_msg);
                                }
                            }
                        });
                    }
                }
            })
            .await;
    });

    // --------------------------------------------------------
    // CONNECTION 2: SAVING & ANALYZING (Database / Deep Logic)
    // --------------------------------------------------------
    let url_analysis = rpc_url.clone();
    let db_analysis = database.clone();
    let tw_analysis = twitter_key.clone();
    let sp_analysis = sol_price.clone();
    let cache_analysis = shared_state.token_cache.clone();
    let comm_cache_analysis = shared_state.community_cache.clone();

    tokio::spawn(async move {
        let client = Client::new(url_analysis);
        println!("[subscriber] Analysis connection started...");

        let _ = client
            .subscribe_to_pump(
                move |(_time, event)| {
                    let db = db_analysis.clone();
                    let tw_key = tw_analysis.clone();
                    let sp = sp_analysis.clone();
                    let cache = cache_analysis.clone();
                    let comm_cache = comm_cache_analysis.clone();
                    let ipfs_local_node_clone_clone = ipfs_local_node.clone();

                    async move {
                        match event {
                            Event::Create(data) => {
                                tokio::spawn(async move {
                                    let _ = process_slow_create(
                                        data,
                                        db,
                                        &tw_key,
                                        cache,
                                        comm_cache,
                                        &ipfs_local_node_clone_clone,
                                    )
                                    .await;
                                });
                            }
                            Event::Buy(data) => {
                                let current_sol_price = sp.load(Ordering::Relaxed);
                                let mcap = usd_mcap(
                                    data.virtual_sol_reserves_before,
                                    data.virtual_token_reserves,
                                    current_sol_price,
                                ) as i64;

                                let _ = db.update_token_ath(&data.mint, mcap).await;
                            }
                            _ => {}
                        }
                    }
                },
                true,
            )
            .await;
    });

    // --- Axum Server with Optimizations ---
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/admin/add_user", post(add_user_handler))
        .route("/admin/remove_user", post(remove_user_handler))
        .route("/admin/users", post(get_users_handler))
        .route("/admin/connections", get(get_connections_handler))
        .route("/admin/restart", post(restart_handler)) // New Route
        .with_state(shared_state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], 3001));
    println!("[server] running on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Wrap the serve with graceful shutdown
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            shutdown_rx.recv().await;
            println!("[server] shutdown signal received, closing server...");
        })
        .await?;

    println!("[server] server stopped. exiting with code 0 for systemd restart.");
    std::process::exit(0);
}

// --- WS HANDLERS ---

async fn ws_handler(
    ws: WebSocketUpgrade,
    AxQuery(auth): AxQuery<WsAuth>,
    AxState(state): AxState<SharedState>,
) -> impl IntoResponse {
    println!("[ws] connection attempt with key: {}", auth.key);

    match state.db.validate_user_key(&auth.key).await {
        Ok(true) => {
            // Generate session ID
            let session_id = state.next_session_id.fetch_add(1, Ordering::SeqCst);

            // CRITICAL FIX: Check if key already has an active connection
            let mut connections = state.active_connections.lock().await;

            if let Some(old_session_id) = connections.get(&auth.key).cloned() {
                let total = connections.len();
                drop(connections);
                println!(
                    "[ws] REJECTED: key {} already has active session {} (tried to start session {}) | total connections: {}",
                    auth.key, old_session_id, session_id, total
                );
                return (
                    StatusCode::CONFLICT, 
                    "Another session is already active for this key. Please close the existing connection first."
                ).into_response();
            }

            // Insert BEFORE upgrading websocket
            connections.insert(auth.key.clone(), session_id);
            let total = connections.len();
            drop(connections);

            println!(
                "[ws] authorized: {} | session_id: {} | total unique keys: {}",
                auth.key, session_id, total
            );

            ws.on_upgrade(move |socket| handle_socket(socket, state, auth.key, session_id))
        }
        _ => {
            println!("[ws] forbidden: {}", auth.key);
            (StatusCode::FORBIDDEN, "Unauthorized").into_response()
        }
    }
}

async fn handle_socket(socket: WebSocket, state: SharedState, key: String, session_id: u64) {
    println!(
        "[ws] socket handler started for key: {} | session_id: {}",
        key, session_id
    );

    let mut rx_broadcast = state.tx.subscribe();
    let (mut sink, mut stream) = socket.split();
    let key_clone = key.clone();
    let key_clone2 = key.clone();

    // Send autobuy status notification immediately after connection
    let autobuy_status = state
        .db
        .get_user_autobuy_status(&key)
        .await
        .unwrap_or(false);
    let status_msg = serde_json::json!({
        "type": "connection_info",
        "autobuy": autobuy_status,
        "session_id": session_id,
        "message": if autobuy_status {
            "Autobuy is enabled for your account"
        } else {
            "Autobuy is disabled for your account"
        }
    });

    if let Ok(json_str) = serde_json::to_string(&status_msg) {
        let _ = sink.send(Message::Text(json_str)).await;
    }

    // OPTIMIZATION: Buffered writes for better throughput
    let send_task = tokio::spawn(async move {
        loop {
            match rx_broadcast.recv().await {
                Ok(arc_msg) => {
                    if sink.send(Message::Text((*arc_msg).clone())).await.is_err() {
                        println!(
                            "[ws] send failed for key: {} session: {}",
                            key_clone, session_id
                        );
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[ws] broadcast lagged for {} by {} messages", key_clone, n);
                }
                Err(_) => {
                    println!(
                        "[ws] broadcast channel closed for key: {} session: {}",
                        key_clone, session_id
                    );
                    break;
                }
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(result) = stream.next().await {
            if result.is_err() {
                println!(
                    "[ws] recv error for key: {} session: {}",
                    key_clone2, session_id
                );
                break;
            }
        }
        println!(
            "[ws] recv stream ended for key: {} session: {}",
            key_clone2, session_id
        );
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {
            println!("[ws] send task completed for key: {} session: {}", key, session_id);
        }
        _ = recv_task => {
            println!("[ws] recv task completed for key: {} session: {}", key, session_id);
        }
    }

    // FIXED: Cleanup only if this session is still the active one
    let mut connections = state.active_connections.lock().await;
    if let Some(&current_session_id) = connections.get(&key) {
        if current_session_id == session_id {
            connections.remove(&key);
            println!(
                "[ws] cleaned up session {} for key: {} | remaining connections: {}",
                session_id,
                key,
                connections.len()
            );
        } else {
            println!(
                "[ws] NOT cleaning up session {} for key: {} (current session is {})",
                session_id, key, current_session_id
            );
        }
    } else {
        println!(
            "[ws] session {} for key: {} already removed | remaining connections: {}",
            session_id,
            key,
            connections.len()
        );
    }
}

// --- LOGIC HELPERS ---

impl CommunityCache {
    fn has_community(&self, community_id: &str) -> bool {
        self.community_ids.contains_key(community_id)
    }

    fn insert_community(&mut self, community_id: &str) {
        self.community_ids.insert(community_id.to_string(), ());
    }
}

impl TokenCache {
    fn check_duplicate(
        &self,
        image: Option<&str>,
        ipfs: Option<&str>,
        description: Option<&str>,
        name: Option<&str>,
        ticker: Option<&str>,
    ) -> bool {
        // Match SQL logic: EXISTS if ANY condition is true

        // Check: image exists
        if let Some(img) = image {
            if self.images.contains_key(img) {
                return true;
            }
        }

        // Check: ipfs exists
        if let Some(ipfs_val) = ipfs {
            if self.ipfs.contains_key(ipfs_val) {
                return true;
            }
        }

        // Check: description + name pair exists
        if let (Some(desc), Some(n)) = (description, name) {
            if self
                .desc_name_pairs
                .contains_key(&(desc.to_string(), n.to_string()))
            {
                return true;
            }
        }

        // Check: description + ticker pair exists
        if let (Some(desc), Some(t)) = (description, ticker) {
            if self
                .desc_ticker_pairs
                .contains_key(&(desc.to_string(), t.to_string()))
            {
                return true;
            }
        }

        // Check: name + ticker pair exists
        if let (Some(n), Some(t)) = (name, ticker) {
            if self
                .name_ticker_pairs
                .contains_key(&(n.to_string(), t.to_string()))
            {
                return true;
            }
        }

        // Check: name exists (standalone)
        if let Some(n) = name {
            if self.names.contains_key(n) {
                return true;
            }
        }

        false
    }

    fn insert_token(
        &mut self,
        image: Option<&str>,
        ipfs: Option<&str>,
        description: Option<&str>,
        name: Option<&str>,
        ticker: Option<&str>,
    ) {
        if let Some(img) = image {
            self.images.insert(img.to_string(), ());
        }
        if let Some(ipfs_val) = ipfs {
            self.ipfs.insert(ipfs_val.to_string(), ());
        }
        if let Some(desc) = description {
            self.descriptions.insert(desc.to_string(), ());
            if let Some(n) = name {
                self.desc_name_pairs
                    .insert((desc.to_string(), n.to_string()), ());
            }
            if let Some(t) = ticker {
                self.desc_ticker_pairs
                    .insert((desc.to_string(), t.to_string()), ());
            }
        }
        if let Some(n) = name {
            self.names.insert(n.to_string(), ());
            if let Some(t) = ticker {
                self.name_ticker_pairs
                    .insert((n.to_string(), t.to_string()), ());
            }
        }
        if let Some(t) = ticker {
            self.tickers.insert(t.to_string(), ());
        }
    }
}

async fn process_fast_create(
    data: CreateEvent,
    twitter_key: &str,
    time: Duration,
    database: Arc<Database>,
    cache: Arc<Mutex<TokenCache>>,
    comm_cache: Arc<Mutex<CommunityCache>>,
    ipfs_local_node: &str,
) -> Option<Token> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();

    // OPTIMIZATION: Run metadata fetch and DB check concurrently
    let t0 = Instant::now();

    let metadata_fut = async {
        let t = Instant::now();
        let res = get_metadata(ipfs_local_node, &data.uri).await;
        (res, t.elapsed())
    };

    let db_check_fut = database.token_any_exists(
        Some(&data.name),
        Some(&data.symbol),
        Some(&data.uri),
        None,
        None,
    );

    let ((metadata_res, meta_time), token_exists) = tokio::join!(metadata_fut, db_check_fut);

    let metadata = match metadata_res {
        Ok(m) => m,
        Err(err) => {
            // Fast cache check even without metadata
            let cache_guard = cache.lock().await;
            if cache_guard.check_duplicate(
                None,
                Some(&data.uri),
                None,
                Some(&data.name),
                Some(&data.symbol),
            ) {
                return None;
            }
            drop(cache_guard);

            let token = Token::fresh(
                data.name.clone(),
                data.symbol.clone(),
                data.user,
                data.bonding_curve,
                None,
                data.mint.clone(),
                data.token_2022,
                Some(data.uri.clone()),
                None,
            );
            pretty_token_log(&token, None, meta_time, None, t0.elapsed());
            return Some(token);
        }
    };

    // OPTIMIZATION: Check cache first (fast path)
    {
        let cache_guard = cache.lock().await;
        if cache_guard.check_duplicate(
            metadata.image.as_deref(),
            Some(&data.uri),
            metadata.description.as_deref(),
            Some(&data.name),
            Some(&data.symbol),
        ) {
            return None;
        }
    }

    if token_exists.unwrap_or(false) {
        return None;
    }

    // OPTIMIZATION: Early timeout check to prevent slow tokens from broadcasting
    if now.saturating_sub(time) > Duration::from_secs(5) {
        println!(
            "[break early {} took more than 5 seconds to load]",
            &data.mint
        );
        return None;
    }

    let (community, twitter_time) = if let Some(tw) = &metadata.twitter {
        if let Some(id) = parse_community_id(tw) {
            let t = Instant::now();
            let res = get_community_by_id(twitter_key, &id).await.ok();
            (res, Some(t.elapsed()))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let dev_perf = if let Some(ref c) = community {
        median_dev_mcap_excluding(&database, c.creator.id.clone(), &data.mint.to_string()).await
    } else {
        None
    };

    let mut token = Token::fresh(
        data.name,
        data.symbol,
        data.user,
        data.bonding_curve,
        community.clone(),
        data.mint.clone(),
        data.token_2022,
        Some(data.uri.clone()),
        Some(metadata.clone()),
    );

    if let Some((avg, last, cnt)) = dev_perf {
        token.dev_performance = Some(DevPerformance {
            average_ath: avg,
            last_tokens: last,
            count: cnt,
        });
    }

    // Add to cache after successful creation
    {
        let mut cache_guard = cache.lock().await;
        cache_guard.insert_token(
            metadata.image.as_deref(),
            Some(&data.uri),
            metadata.description.as_deref(),
            Some(&token.name),
            Some(&token.ticker),
        );
    }

    let total_time = t0.elapsed();

    pretty_token_log(&token, Some(&metadata), meta_time, twitter_time, total_time);

    Some(token)
}

async fn process_slow_create(
    data: CreateEvent,
    database: Arc<Database>,
    twitter_key: &str,
    cache: Arc<Mutex<TokenCache>>,
    comm_cache: Arc<Mutex<CommunityCache>>,
    ipfs_local_node: &str,
) -> Option<()> {
    let metadata = get_metadata(ipfs_local_node, &data.uri).await.ok()?;

    // OPTIMIZATION: Check cache first before DB
    {
        let cache_guard = cache.lock().await;
        if cache_guard.check_duplicate(
            metadata.image.as_deref(),
            Some(&data.uri),
            metadata.description.as_deref(),
            Some(&data.name),
            Some(&data.symbol),
        ) {
            return None;
        }
    }

    let token_exists = database
        .token_any_exists(
            Some(&data.name),
            Some(&data.symbol),
            Some(&data.uri),
            metadata.image.as_deref(),
            metadata.description.as_deref(),
        )
        .await
        .unwrap();

    if token_exists {
        return None;
    }

    let mut token = Token::fresh(
        data.name,
        data.symbol,
        data.user,
        data.bonding_curve,
        None,
        data.mint.clone(),
        data.token_2022,
        Some(data.uri.clone()),
        Some(metadata.clone()),
    );

    if let Some(tw) = metadata.twitter {
        if let Some(id) = parse_community_id(&tw) {
            // OPTIMIZATION: Check community cache first
            {
                let comm_cache_guard = comm_cache.lock().await;
                if comm_cache_guard.has_community(&id) {
                    return None;
                }
            }

            // Check database if not in cache
            if database.token_community_exists(&id).await.unwrap_or(false) {
                // Add to cache for future checks
                let mut comm_cache_guard = comm_cache.lock().await;
                comm_cache_guard.insert_community(&id);
                return None;
            }

            if let Ok(comm) = get_community_by_id(twitter_key, &id).await {
                token.twitter = Some(comm.clone());

                // Add to community cache immediately after successful fetch
                {
                    let mut comm_cache_guard = comm_cache.lock().await;
                    comm_cache_guard.insert_community(&id);
                }

                let _ = database
                    .add_token(
                        &token.mint,
                        &token.dbtoken(token.mint.clone()),
                        comm.creator.id.clone(),
                    )
                    .await;

                // Add to token cache after successful DB insert
                let mut cache_guard = cache.lock().await;
                cache_guard.insert_token(
                    metadata.image.as_deref(),
                    Some(&data.uri),
                    metadata.description.as_deref(),
                    Some(&token.name),
                    Some(&token.ticker),
                );
            }
        }
    }

    Some(())
}

pub async fn median_dev_mcap_excluding(
    db: &Database,
    dev_address: String,
    exclude_mint: &str,
) -> Option<(u64, Vec<DbToken>, usize)> {
    let (median, count) = db
        .get_dev_median_ath_excluding(&dev_address, exclude_mint)
        .await
        .ok()??;
    let last_three = db
        .get_last_tokens_by_dev_excluding(&dev_address, exclude_mint, 3)
        .await
        .ok()?;
    Some((median as u64, last_three, count))
}

// --- ADMIN HANDLERS ---

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

async fn add_user_handler(
    AxState(state): AxState<SharedState>,
    Json(req): Json<AddUserReq>,
) -> impl IntoResponse {
    match state.db.add_user(&req.admin_key, req.payload).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "success"})),
        ),
        _ => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "unauthorized"})),
        ),
    }
}

async fn remove_user_handler(
    AxState(state): AxState<SharedState>,
    Json(req): Json<RemoveUserReq>,
) -> impl IntoResponse {
    let target_key = state.db.get_key_by_id(req.user_id).await.ok();

    if state
        .db
        .remove_user(&req.admin_key, req.user_id)
        .await
        .is_ok()
    {
        if let Some(key) = target_key {
            println!("[admin] revoking access for key: {}", key);

            // Just remove from map - the connection will die naturally when it tries to send
            let mut connections = state.active_connections.lock().await;
            if connections.remove(&key).is_some() {
                println!("[admin] removed key {} from active connections", key);
            }
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "success"})),
        )
            .into_response();
    }

    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

async fn get_users_handler(
    AxState(state): AxState<SharedState>,
    Json(req): Json<GetUsersReq>,
) -> impl IntoResponse {
    match state.db.fetch_all_users(&req.admin_key).await {
        Ok(users) => (StatusCode::OK, Json(serde_json::to_value(users).unwrap())),
        _ => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ),
    }
    .into_response()
}

async fn get_connections_handler(AxState(state): AxState<SharedState>) -> impl IntoResponse {
    let connections = state.active_connections.lock().await;
    let conn_list: Vec<ConnectionStatus> = connections
        .iter()
        .map(|(key, session_id)| ConnectionStatus {
            key: key.clone(),
            session_id: *session_id,
        })
        .collect();

    Json(serde_json::json!({
        "count": conn_list.len(),
        "connections": conn_list
    }))
}

fn pretty_token_log(
    token: &Token,
    metadata: Option<&Metadata>,
    meta_time: Duration,
    twitter_time: Option<Duration>,
    total_time: Duration,
) {
    println!(
        "\n [data: {}]new token
├─ name:      {} ({})
├─ mint:      {}
├─ twitter:   {}
├─ twitter_data: {:?}
├─ dev_perf:  {}
├─ token2022: {}
├─ timing:
│  ├─ metadata: {:>4} ms
│  ├─ twitter:  {:>4} ms
│  └─ total:    {:>4} ms",
        metadata.is_some(),
        token.name,
        token.ticker,
        token.mint,
        token.twitter.is_some(),
        metadata
            .and_then(|m| m.twitter.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("None"),
        token.dev_performance.is_some(),
        token.token_2022,
        meta_time.as_millis(),
        twitter_time.map(|t| t.as_millis()).unwrap_or(0),
        total_time.as_millis(),
    );
}

async fn restart_handler(
    AxState(state): AxState<SharedState>,
    Json(req): Json<RestartReq>,
) -> impl IntoResponse {
    match state.db.fetch_all_users(&req.admin_key).await {
        Ok(_) => {
            println!(
                "[admin] restart triggered by admin key ending in ...{}",
                &req.admin_key[req.admin_key.len().saturating_sub(4)..]
            );

            // Send the signal to shutdown
            let _ = state.shutdown_tx.send(()).await;

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "restarting",
                    "message": "Server is shutting down for systemd restart"
                })),
            )
                .into_response() // <--- Added .into_response() here
        }
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response(), // <--- Both arms now return Response<Body>
    }
}
