use reqwest::{ClientBuilder, Url, cookie::Jar};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::{env, fmt, sync::Arc};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatorHistory {
    pub counts: Migrated,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Migrated {
    pub totalCount: u64,
    pub migratedCount: u64,
}

#[derive(Debug)]
pub enum HistoryError {
    RequestError(reqwest::Error),
    JsonError(serde_json::Error),
    Other(String),
    EmptyResponse,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::RequestError(e) => write!(f, "HTTP request failed: {}", e),
            HistoryError::JsonError(e) => write!(f, "JSON parse failed: {}", e),
            HistoryError::Other(msg) => write!(f, "Other error: {}", msg),
            HistoryError::EmptyResponse => write!(f, "empty responce"),
        }
    }
}

impl std::error::Error for HistoryError {}
impl From<reqwest::Error> for HistoryError {
    fn from(err: reqwest::Error) -> HistoryError {
        HistoryError::RequestError(err)
    }
}
impl From<serde_json::Error> for HistoryError {
    fn from(err: serde_json::Error) -> HistoryError {
        HistoryError::JsonError(err)
    }
}

async fn refresh_access_token(jar: &Jar) -> Result<(), HistoryError> {
    let jar = Arc::new(Jar::default());

    let url = "https://api3.axiom.trade/refresh-access-token";
    let client = reqwest::Client::builder()
        .cookie_provider(jar.clone())
        .build()
        .unwrap();

    let resp = client.post(url).send().await?;
    if !resp.status().is_success() {
        return Err(HistoryError::Other(format!(
            "failed to refresh access token: {}",
            resp.status()
        )));
    }
    Ok(())
}

pub async fn get_user_created_coins(user: &Pubkey) -> Result<CreatorHistory, HistoryError> {
    let request = format!("https://api3.axiom.trade/dev-tokens-v2?devAddress={}", user);

    let jar = Arc::new(Jar::default());
    jar.add_cookie_str(
        &env::var("AXIOM_COOKIE").unwrap(),
        &"https://api3.axiom.trade".parse::<Url>().unwrap(),
    );

    //refresh_access_token(&jar).await.unwrap();

    let client = ClientBuilder::new()
        .cookie_store(true)
        .cookie_provider(jar.clone())
        .build()
        .unwrap();

    //for attempt in 0..3 {
    let response = client.get(&request).send().await?;

    let body = response.text().await?;

    let history: CreatorHistory = serde_json::from_str(&body)?;
    if history.counts.totalCount != 0 || history.counts.migratedCount != 0 {
        return Ok(history);
    }
    //}

    Err(HistoryError::EmptyResponse)
}
