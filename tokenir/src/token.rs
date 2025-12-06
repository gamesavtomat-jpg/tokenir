use rust_decimal::{Decimal, dec};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::{Trade, constans::helper::CommunityInfo, database::DbToken};

const FRESH_MARKET_CAP: u64 = 4900;

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
}

#[derive(Clone, Serialize, Deserialize)]
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
        DbToken {
            mint: mint.to_string(),
            dev_address: self.dev.to_string(),
            ath: self.usd_ath() as i64,
        }
    }
}
