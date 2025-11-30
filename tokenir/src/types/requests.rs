use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LogsNotification {
    jsonrpc: String,
    method: String,
    pub params: Params,
}

#[derive(Debug, Deserialize)]
pub struct Params {
    pub result: ResultField,
    subscription: u64,
}

#[derive(Debug, Deserialize)]
pub struct ResultField {
    context: Context,
    pub value: LogValue,
}

#[derive(Debug, Deserialize)]
pub struct Context {
    slot: u64,
}

#[derive(Debug, Deserialize)]
pub struct LogValue {
    signature: String,
    err: Option<serde_json::Value>,
    pub logs: Vec<String>,
}

#[derive(Deserialize)]
pub struct PriceResponse {
    pub solana: SolanaPrice,
}

#[derive(Deserialize)]
pub struct SolanaPrice {
    pub usd: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreatorHistory {
    pub tokens: Vec<TokenInfo>,
    pub counts: Counts,
}

#[derive(Debug, Deserialize)]
pub struct Counts {
    pub totalCount: u64,
    pub migratedCount: u64,
}

#[derive(Debug, Deserialize)]
pub struct TokenInfo {
    pub pairAddress: String,
    pub tokenAddress: String,
    pub tokenTicker: String,
    pub tokenName: String,
    pub tokenImage: String,
    pub protocol: String,
    pub supply: f64,
    pub extra: Option<serde_json::Value>,
    pub createdAt: String,
    pub migrated: bool,
    pub liquiditySol: f64,
    pub liquidityToken: f64,
    pub hourlyVolumeSol: f64,
    pub priceSol: f64,
}

impl TokenInfo {
    pub fn usd_mcap(&self) -> u64 {
        if self.liquidityToken == 0f64 {
            return 4900;
        }
        let mcap = self.liquiditySol as u64;
        (mcap.saturating_mul(177000000000) / self.liquidityToken as u64) as u64
    }
}

#[derive(Deserialize)]
pub struct Metadata {
    pub name: String,
    pub symbol: String,
    pub description: String,
    pub twitter: Option<String>,
    pub website: Option<String>,
}
