use rust_decimal::{dec, Decimal};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::{
    constans::helper::{pool_pda, CommunityInfo},
    database::DbToken,
    requests::Metadata,
    Trade,
};

const FRESH_MARKET_CAP: u64 = 3264;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub token_2022: bool,
    pub metadata_ipfs: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevPerformance {
    pub average_ath: u64,
    pub last_tokens: Vec<DbToken>,
    pub count: usize,
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
        token_2022: bool,
        metadata_ipfs: Option<String>,
        metadata: Option<Metadata>,
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
            token_2022,
            metadata_ipfs,
            metadata,
        }
    }

    pub fn update(&mut self, event: Trade, price: u64) {
        self.reserves = event.reserves();
        self.mcap = event.mcap();

        let mcap = self.usd_mcap(price);

        if mcap > self.ath {
            self.ath = mcap;
        }
    }

    pub fn usd_mcap(&self, price: u64) -> u64 {
        let mcap = self.mcap as u128;
        (mcap.saturating_mul(price as u128 * 1000000) / self.reserves as u128) as u64
    }

    pub fn usd_ath(&self) -> u64 {
        self.ath
    }

    pub fn dbtoken(&self, mint: Pubkey) -> DbToken {
        let image = self.metadata.as_ref().and_then(|m| m.image.clone());

        let description = self.metadata.as_ref().and_then(|m| m.description.clone());

        let twitter = self.twitter.as_ref().and_then(|t| Some(t.id.clone()));

        DbToken {
            mint: mint.to_string(),
            dev_address: self.dev.to_string(),
            ath: self.usd_ath() as i64,
            name: self.name.clone(),
            ticker: self.ticker.clone(),
            ipfs: self.metadata_ipfs.clone(),
            image: image,
            description,
            community_id: twitter,
            pool_address: pool_pda(&mint).0.to_string(),
        }
    }
}

pub fn usd_mcap(mcap: u64, reserves: u64, price: u64) -> u64 {
    let mcap = mcap as u128;
    (mcap.saturating_mul(price as u128 * 1000000) / reserves as u128) as u64
}
