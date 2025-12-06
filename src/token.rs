use std::{env, str::FromStr};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};

use crate::migration::CreatorHistory;

#[derive(Clone, Serialize, Deserialize)]
pub struct Token {
    pub mint: Pubkey,
    pub name: String,
    pub ticker: String,
    pub mcap: u64,
    pub dev: Pubkey,
    pub reserves: u64,
    pub curve: Pubkey,
    pub ath: u64,
    pub twitter: Option<CommunityInfo>,
    pub dev_performance: Option<DevPerformance>,
    pub migrated: Option<CreatorHistory>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DevPerformance {
    pub average_ath: u64,
    pub last_tokens: Vec<DbToken>,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DbToken {
    pub mint: String,
    pub dev_address: String,
    pub ath: i64,

    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CommunityInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub member_count: Option<u64>,
    pub moderator_count: Option<u64>,
    pub created_at: Option<String>,
    pub creator: Creator,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Creator {
    pub id: String,
    pub screen_name: Option<String>,
}

const FRESH_MARKET_CAP: u64 = 4900;

#[derive(Debug, Deserialize)]
pub struct MoralisMetadata {
    pub name: Option<String>,
    pub symbol: Option<String>,
}

pub async fn fetch_metadata(mint: &Pubkey) -> Option<MoralisMetadata> {
    let api_key = env::var("METADATA_API").ok()?;
    let url = format!(
        "https://solana-gateway.moralis.io/token/mainnet/{}/metadata",
        mint.to_string()
    );

    let client = Client::new();
    let resp = client
        .get(url)
        .header("accept", "application/json")
        .header("X-API-Key", api_key)
        .send()
        .await.ok()?
        .json::<MoralisMetadata>()
        .await.ok()?;

    Some(resp)
}


impl Token {
    pub fn twitter(&self) -> &Option<CommunityInfo> {
        &self.twitter
    }

    pub fn fresh(
        name: String,
        ticker: String,
        dev: Pubkey,
        curve: Pubkey,
        twitter: Option<CommunityInfo>,
        mint: Pubkey,
    ) -> Self {
        Self {
            mint,
            name,
            ticker,
            mcap: FRESH_MARKET_CAP,
            dev,
            reserves: 1_073_000_000,
            curve,
            ath: FRESH_MARKET_CAP,
            twitter,
            dev_performance: None,
            migrated: None,
        }
    }

    pub fn usd_mcap(&self, price: u64) -> u64 {
        let mcap = self.mcap as u128;
        (mcap.saturating_mul(price as u128 * 1000000) / self.reserves as u128) as u64
    }

    pub fn usd_ath(&self) -> u64 {
        self.ath
    }

    pub fn dbtoken(self, mint: Pubkey) -> DbToken {
        DbToken {
            mint: mint.to_string(),
            dev_address: self.dev.to_string(),
            ath: self.usd_ath() as i64,
            name: None,
        }
    }
    

    pub async fn load_history(&mut self) -> Result<(), Error> {
        let Some(performance) = &mut self.dev_performance else {
            return Err(Error::NoDevPerformanceFound);
        };

        for token in performance.last_tokens.iter_mut() {
            let Ok(mint) = Pubkey::from_str(&token.mint) else {
                continue;
            };

            if let Some(meta) = fetch_metadata(&mint).await {
                if let Some(symb) = meta.symbol {
                    token.name = Some(format!("${}", symb));
                } else if let Some(name) = meta.name {
                    token.name = Some(name);
                }
            }
        }

        Ok(())
    }

}

pub enum Error {
    NoDevPerformanceFound,
}
